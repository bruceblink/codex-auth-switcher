use super::{
    LATEST_RELEASE_URL, MAX_METADATA_BYTES, MAX_RELEASE_ASSETS, MAX_RELEASE_NOTES_BYTES,
    UpdateAsset, UpdateCheck, UpdateError, UpdateInfo, UpdatePlatform, UpdateRelease, asset_url,
    is_safe_asset_name, is_trusted_github_url, manifest_url, stable_tag,
};
use reqwest::{Client, Response, StatusCode, Url};
use semver::Version;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateManifest {
    schema_version: u32,
    version: String,
    tag_name: String,
    notes: String,
    published_at: String,
    assets: Vec<ManifestAsset>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestAsset {
    name: String,
    size: u64,
    sha256: String,
}

pub(super) async fn check_for_update(
    client: &Client,
    current_version: Version,
) -> Result<UpdateCheck, UpdateError> {
    let latest_url = Url::parse(LATEST_RELEASE_URL).map_err(|_| UpdateError::InvalidManifest)?;
    let response = client
        .head(latest_url)
        .send()
        .await
        .map_err(request_error)?;
    let latest_release = release_from_permalink(&response)?;

    if latest_release.version <= current_version {
        return Ok(UpdateCheck::Current { current_version });
    }

    let response = client
        .get(manifest_url(&latest_release.version)?)
        .send()
        .await
        .map_err(request_error)?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(UpdateCheck::LegacyRelease {
            release: latest_release,
        });
    }
    ensure_success(&response)?;
    if !is_trusted_github_url(response.url(), None) {
        return Err(UpdateError::RequestFailed);
    }
    let body = read_bounded_body(response).await?;
    let (release, assets) = parse_manifest(&body)?;
    if release.version != latest_release.version || release.tag_name != latest_release.tag_name {
        return Err(UpdateError::InvalidManifest);
    }
    Ok(select_platform_asset(
        release,
        assets,
        current_version,
        super::current_platform(),
    ))
}

fn release_from_permalink(response: &Response) -> Result<UpdateRelease, UpdateError> {
    if response.status() == StatusCode::NOT_FOUND {
        return Err(UpdateError::NoRelease);
    }
    ensure_success(response)?;
    parse_release_permalink(response.url())
}

pub(super) fn parse_release_permalink(url: &Url) -> Result<UpdateRelease, UpdateError> {
    let prefix = format!("/{}/releases/tag/", super::REPOSITORY);
    if !is_trusted_github_url(url, None)
        || url.host_str() != Some("github.com")
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with(&prefix)
    {
        return Err(UpdateError::InvalidManifest);
    }
    let tag_name = &url.path()[prefix.len()..];
    if tag_name.contains('/') {
        return Err(UpdateError::InvalidManifest);
    }
    let version = stable_tag(tag_name)?;
    let canonical_url = super::release_url(&version)?;
    if url != &canonical_url {
        return Err(UpdateError::InvalidManifest);
    }
    Ok(UpdateRelease {
        version,
        tag_name: tag_name.to_owned(),
        notes: String::new(),
        published_at: String::new(),
        release_url: canonical_url,
    })
}

pub(super) fn parse_manifest(
    body: &[u8],
) -> Result<(UpdateRelease, Vec<UpdateAsset>), UpdateError> {
    if body.len() > MAX_METADATA_BYTES {
        return Err(UpdateError::InvalidManifest);
    }
    let manifest: UpdateManifest =
        serde_json::from_slice(body).map_err(|_| UpdateError::InvalidManifest)?;
    if manifest.schema_version != 1
        || manifest.notes.len() > MAX_RELEASE_NOTES_BYTES
        || manifest.assets.len() > MAX_RELEASE_ASSETS
    {
        return Err(UpdateError::InvalidManifest);
    }

    let version = stable_tag(&manifest.tag_name)?;
    if manifest.version != version.to_string()
        || manifest.tag_name != format!("v{version}")
        || chrono::DateTime::parse_from_rfc3339(&manifest.published_at).is_err()
    {
        return Err(UpdateError::InvalidManifest);
    }

    let mut names = HashSet::with_capacity(manifest.assets.len());
    let mut assets = Vec::with_capacity(manifest.assets.len());
    for asset in manifest.assets {
        if !is_safe_asset_name(&asset.name)
            || !names.insert(asset.name.clone())
            || asset.size == 0
            || asset.size > super::MAX_DOWNLOAD_BYTES
            || !is_sha256(&asset.sha256)
        {
            return Err(UpdateError::InvalidManifest);
        }
        let download_url = asset_url(&version, &asset.name)?;
        assets.push(UpdateAsset {
            name: asset.name,
            size: asset.size,
            sha256: asset.sha256.to_ascii_lowercase(),
            download_url,
        });
    }

    let release_url = super::release_url(&version)?;
    Ok((
        UpdateRelease {
            version,
            tag_name: manifest.tag_name,
            notes: manifest.notes,
            published_at: manifest.published_at,
            release_url,
        },
        assets,
    ))
}

pub(super) fn select_platform_asset(
    release: UpdateRelease,
    assets: Vec<UpdateAsset>,
    current_version: Version,
    platform: Option<UpdatePlatform>,
) -> UpdateCheck {
    if release.version <= current_version {
        return UpdateCheck::Current { current_version };
    }
    let Some(platform) = platform else {
        return UpdateCheck::UnsupportedPlatform { release };
    };
    let expected_name = platform.asset_name(&release.version);
    let Some(asset) = assets.into_iter().find(|asset| asset.name == expected_name) else {
        return UpdateCheck::UnsupportedPlatform { release };
    };
    UpdateCheck::Available(UpdateInfo { release, asset })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn read_bounded_body(response: Response) -> Result<Vec<u8>, UpdateError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES as u64)
    {
        return Err(UpdateError::InvalidManifest);
    }
    let mut response = response;
    let mut body = Vec::new();
    loop {
        let chunk = super::request_with_idle_timeout(response.chunk())
            .await?
            .unwrap_or_default();
        if chunk.is_empty() {
            break;
        }
        if body.len().saturating_add(chunk.len()) > MAX_METADATA_BYTES {
            return Err(UpdateError::InvalidManifest);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn ensure_success(response: &Response) -> Result<(), UpdateError> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(UpdateError::HttpStatus(response.status().as_u16()))
    }
}

fn request_error(error: reqwest::Error) -> UpdateError {
    if error.is_timeout() {
        UpdateError::Timeout
    } else {
        UpdateError::RequestFailed
    }
}
