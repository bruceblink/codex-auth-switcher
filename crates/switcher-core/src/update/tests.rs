use super::{
    MAX_DOWNLOAD_BYTES, MAX_METADATA_BYTES, MAX_RELEASE_ASSETS, MAX_RELEASE_NOTES_BYTES,
    RELEASE_TAG_PREFIX, UpdateCheck, UpdateError, UpdatePlatform, current_platform,
    is_safe_asset_name, is_trusted_github_url, metadata, stable_tag,
};
use semver::Version;
use serde_json::{Value, json};
use url::Url;

const TEST_VERSION: &str = "0.3.1";
const TEST_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn valid_manifest() -> Value {
    let version = Version::new(0, 3, 1);
    let platforms = [
        UpdatePlatform::WindowsX64,
        UpdatePlatform::LinuxAmd64Deb,
        UpdatePlatform::LinuxX86_64AppImage,
        UpdatePlatform::MacosArm64,
        UpdatePlatform::MacosX86_64,
    ];
    let assets = platforms
        .into_iter()
        .map(|platform| {
            json!({
                "name": platform.asset_name(&version),
                "size": 1,
                "sha256": TEST_HASH,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "version": TEST_VERSION,
        "tag_name": "v0.3.1",
        "notes": "Release notes",
        "published_at": "2026-11-01T00:00:00Z",
        "assets": assets,
    })
}

fn manifest_bytes(manifest: &Value) -> Vec<u8> {
    serde_json::to_vec(manifest).unwrap_or_default()
}

#[test]
fn stable_tag_requires_canonical_stable_semver() {
    assert_eq!(stable_tag("v0.3.1"), Ok(Version::new(0, 3, 1)));
    assert_eq!(stable_tag("v0.3.1-rc.1"), Err(UpdateError::InvalidManifest));
    assert_eq!(
        stable_tag("v0.3.1+build.1"),
        Err(UpdateError::InvalidManifest)
    );
    assert_eq!(stable_tag("v00.3.1"), Err(UpdateError::InvalidManifest));
    assert_eq!(stable_tag("0.3.1"), Err(UpdateError::InvalidManifest));
    assert_eq!(
        stable_tag("v0.3.1/other"),
        Err(UpdateError::InvalidManifest)
    );
}

#[test]
fn release_permalink_requires_the_own_stable_tag_route() {
    let valid = Url::parse("https://github.com/bruceblink/codex-auth-switcher/releases/tag/v0.3.1");
    assert!(valid.is_ok_and(|url| metadata::parse_release_permalink(&url).is_ok()));

    for value in [
        "https://github.com/other/repo/releases/tag/v0.3.1",
        "https://github.com/bruceblink/codex-auth-switcher/releases/tag/0.3.1",
        "https://github.com/bruceblink/codex-auth-switcher/releases/tag/v0.3.1-rc.1",
        "https://github.com/bruceblink/codex-auth-switcher/releases/tag/v0.3.1/extra",
        "https://github.com/bruceblink/codex-auth-switcher/releases/tag/v0.3.1?token=value",
    ] {
        let url = Url::parse(value);
        assert!(url.is_ok_and(|url| {
            metadata::parse_release_permalink(&url) == Err(UpdateError::InvalidManifest)
        }));
    }
}

#[test]
fn manifest_accepts_canonical_release_and_platform_assets() {
    let parsed = metadata::parse_manifest(&manifest_bytes(&valid_manifest()));
    assert!(parsed.is_ok());
    let Ok((release, assets)) = parsed else {
        return;
    };
    assert_eq!(release.version, Version::new(0, 3, 1));
    assert_eq!(release.tag_name, "v0.3.1");
    assert_eq!(release.notes, "Release notes");
    assert_eq!(
        release.release_url.as_str(),
        format!("{RELEASE_TAG_PREFIX}v0.3.1")
    );
    assert_eq!(assets.len(), 5);

    for platform in [
        UpdatePlatform::WindowsX64,
        UpdatePlatform::LinuxAmd64Deb,
        UpdatePlatform::LinuxX86_64AppImage,
        UpdatePlatform::MacosArm64,
        UpdatePlatform::MacosX86_64,
    ] {
        let result = metadata::select_platform_asset(
            release.clone(),
            assets.clone(),
            Version::new(0, 3, 0),
            Some(platform),
        );
        let UpdateCheck::Available(update) = result else {
            panic!("expected the exact platform package");
        };
        assert_eq!(
            update.asset.name(),
            platform.asset_name(&update.release.version)
        );
        assert_eq!(update.asset.size(), 1);
        assert_eq!(update.asset.sha256(), TEST_HASH);
    }
}

#[test]
fn manifest_rejects_version_tag_mismatch_and_prereleases() {
    let mut manifest = valid_manifest();
    manifest["version"] = json!("0.3.2");
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut manifest = valid_manifest();
    manifest["tag_name"] = json!("v0.3.1-rc.1");
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );
}

#[test]
fn manifest_rejects_duplicate_unsafe_oversized_and_invalid_assets() {
    let mut manifest = valid_manifest();
    let duplicate = manifest["assets"][0].clone();
    if let Some(assets) = manifest["assets"].as_array_mut() {
        assets.push(duplicate);
    }
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut manifest = valid_manifest();
    manifest["assets"][0]["name"] = json!("../installer.exe");
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut manifest = valid_manifest();
    manifest["assets"][0]["size"] = json!(0);
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut manifest = valid_manifest();
    manifest["assets"][0]["size"] = json!(MAX_DOWNLOAD_BYTES + 1);
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut manifest = valid_manifest();
    manifest["assets"][0]["sha256"] = json!("not-a-sha256");
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );
}

#[test]
fn manifest_rejects_notes_asset_count_timestamp_and_body_limit_violations() {
    let mut manifest = valid_manifest();
    manifest["notes"] = json!("n".repeat(MAX_RELEASE_NOTES_BYTES + 1));
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut manifest = valid_manifest();
    let extra_assets = (0..MAX_RELEASE_ASSETS + 1)
        .map(|index| {
            json!({
                "name": format!("extra-{index}.bin"),
                "size": 1,
                "sha256": TEST_HASH,
            })
        })
        .collect::<Vec<_>>();
    manifest["assets"] = json!(extra_assets);
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut manifest = valid_manifest();
    manifest["published_at"] = json!("not-a-timestamp");
    assert_eq!(
        metadata::parse_manifest(&manifest_bytes(&manifest)),
        Err(UpdateError::InvalidManifest)
    );

    let mut too_large = manifest_bytes(&valid_manifest());
    too_large.resize(MAX_METADATA_BYTES + 1, b' ');
    assert_eq!(
        metadata::parse_manifest(&too_large),
        Err(UpdateError::InvalidManifest)
    );
}

#[test]
fn platform_selection_reports_current_and_unsupported_states() {
    let parsed = metadata::parse_manifest(&manifest_bytes(&valid_manifest()));
    assert!(parsed.is_ok());
    let Ok((release, assets)) = parsed else {
        return;
    };

    assert_eq!(
        metadata::select_platform_asset(
            release.clone(),
            assets.clone(),
            Version::new(0, 3, 1),
            Some(UpdatePlatform::WindowsX64),
        ),
        UpdateCheck::Current {
            current_version: Version::new(0, 3, 1)
        }
    );
    assert!(matches!(
        metadata::select_platform_asset(
            release.clone(),
            assets.clone(),
            Version::new(0, 3, 0),
            None,
        ),
        UpdateCheck::UnsupportedPlatform { .. }
    ));
    assert!(matches!(
        metadata::select_platform_asset(
            release,
            Vec::new(),
            Version::new(0, 3, 0),
            Some(UpdatePlatform::WindowsX64),
        ),
        UpdateCheck::UnsupportedPlatform { .. }
    ));
}

#[test]
fn asset_names_and_redirect_urls_stay_within_supported_boundaries() {
    assert!(is_safe_asset_name(
        "Codex-Auth-Switcher-0.3.1-windows-x64-setup.exe"
    ));
    assert!(!is_safe_asset_name("../update.exe"));
    assert!(!is_safe_asset_name(".."));
    assert!(!is_safe_asset_name("包.exe"));

    let trusted = Url::parse("https://github.com/bruceblink/codex-auth-switcher").ok();
    assert!(trusted.is_some_and(|url| is_trusted_github_url(&url, None)));
    let trusted_asset =
        Url::parse("https://release-assets.githubusercontent.com/assets/pkg?signature=test").ok();
    assert!(trusted_asset.is_some_and(|url| is_trusted_github_url(&url, None)));

    for value in [
        "http://github.com/bruceblink/codex-auth-switcher",
        "https://github.com.evil.example/releases/file",
        "https://user@github.com/releases/file",
        "https://example.com/releases/file",
        "https://github.com:444/releases/file",
    ] {
        let url = Url::parse(value);
        assert!(url.is_ok_and(|url| !is_trusted_github_url(&url, None)));
    }
}

#[test]
fn current_platform_matches_a_supported_build_target() {
    assert!(current_platform().is_some());

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    assert_eq!(current_platform(), Some(UpdatePlatform::WindowsX64));

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    assert_eq!(
        current_platform(),
        Some(
            if std::env::var_os("APPIMAGE").is_some_and(|value| !value.is_empty()) {
                UpdatePlatform::LinuxX86_64AppImage
            } else {
                UpdatePlatform::LinuxAmd64Deb
            }
        )
    );

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(current_platform(), Some(UpdatePlatform::MacosArm64));

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    assert_eq!(current_platform(), Some(UpdatePlatform::MacosX86_64));
}

#[test]
fn cancellation_is_idempotent_and_errors_are_safe_to_display() {
    let cancellation = super::CancellationToken::new();
    assert!(!cancellation.is_cancelled());
    cancellation.cancel();
    cancellation.cancel();
    assert!(cancellation.is_cancelled());
    assert!(UpdateError::Cancelled.is_cancelled());
    assert!(!UpdateError::CacheIo.is_cancelled());
    assert!(!UpdateError::HashMismatch.to_string().contains("github.com"));
}
