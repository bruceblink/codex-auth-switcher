use super::{
    CancellationToken, DOWNLOAD_IDLE_TIMEOUT, DownloadProgress, DownloadedUpdate, ProgressCallback,
    UpdateAsset, UpdateError, is_trusted_github_url, request_with_idle_timeout,
};
use reqwest::{Client, Response};
use sha2::{Digest, Sha256};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const HASH_BUFFER_BYTES: usize = 64 * 1024;

pub(super) async fn download_verified_asset(
    client: &Client,
    asset: UpdateAsset,
    cache_dir: PathBuf,
    progress_callback: ProgressCallback,
    cancellation: CancellationToken,
) -> Result<DownloadedUpdate, UpdateError> {
    if cancellation.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    fs::create_dir_all(&cache_dir)
        .await
        .map_err(|_| UpdateError::CacheIo)?;

    let destination = cache_dir.join(&asset.name);
    let partial = cache_dir.join(format!("{}.part", asset.name));
    remove_partial(&partial).await?;
    if cache_entry_exists(&destination).await? {
        match verify_file(&destination, asset.size, &asset.sha256, Some(&cancellation)).await {
            Ok(()) => {
                set_verified_asset_permissions(&asset, &destination).await?;
                if cancellation.is_cancelled() {
                    return Err(UpdateError::Cancelled);
                }
                let mut progress = ProgressReporter::new(progress_callback);
                progress.report(0, asset.size, true);
                if cancellation.is_cancelled() {
                    return Err(UpdateError::Cancelled);
                }
                progress.report(asset.size, asset.size, true);
                if cancellation.is_cancelled() {
                    return Err(UpdateError::Cancelled);
                }
                return Ok(DownloadedUpdate {
                    path: destination,
                    from_cache: true,
                });
            }
            Err(UpdateError::SizeMismatch | UpdateError::HashMismatch) => {
                fs::remove_file(&destination)
                    .await
                    .map_err(|_| UpdateError::CacheIo)?;
            }
            Err(error) => return Err(error),
        }
    }

    let result = tokio::time::timeout(
        super::MAX_DOWNLOAD_DURATION,
        download_to_partial(client, &asset, &partial, &progress_callback, &cancellation),
    )
    .await
    .map_err(|_| UpdateError::Timeout)
    .and_then(|result| result);
    if let Err(error) = result {
        return match remove_partial(&partial).await {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(cleanup_error),
        };
    }
    if cancellation.is_cancelled() {
        return match remove_partial(&partial).await {
            Ok(()) => Err(UpdateError::Cancelled),
            Err(cleanup_error) => Err(cleanup_error),
        };
    }
    if let Err(error) = set_verified_asset_permissions(&asset, &partial).await {
        return match remove_partial(&partial).await {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(cleanup_error),
        };
    }
    if cancellation.is_cancelled() {
        return match remove_partial(&partial).await {
            Ok(()) => Err(UpdateError::Cancelled),
            Err(cleanup_error) => Err(cleanup_error),
        };
    }
    if fs::rename(&partial, &destination).await.is_err() {
        return match remove_partial(&partial).await {
            Ok(()) => Err(UpdateError::CacheIo),
            Err(cleanup_error) => Err(cleanup_error),
        };
    }
    Ok(DownloadedUpdate {
        path: destination,
        from_cache: false,
    })
}

pub(super) async fn verify_cached_download(
    cache_dir: &Path,
    asset: &UpdateAsset,
    path: &Path,
) -> Result<DownloadedUpdate, UpdateError> {
    if path.file_name() != Some(std::ffi::OsStr::new(&asset.name)) {
        return Err(UpdateError::CacheIo);
    }
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|_| UpdateError::CacheIo)?;
    if !metadata.file_type().is_file() {
        return Err(UpdateError::CacheIo);
    }

    let canonical_cache = fs::canonicalize(cache_dir)
        .await
        .map_err(|_| UpdateError::CacheIo)?;
    let canonical_path = fs::canonicalize(path)
        .await
        .map_err(|_| UpdateError::CacheIo)?;
    if canonical_path != canonical_cache.join(&asset.name) {
        return Err(UpdateError::CacheIo);
    }
    verify_file(path, asset.size, &asset.sha256, None).await?;
    set_verified_asset_permissions(asset, path).await?;

    Ok(DownloadedUpdate {
        path: canonical_path,
        from_cache: true,
    })
}

async fn download_to_partial(
    client: &Client,
    asset: &UpdateAsset,
    partial: &Path,
    progress_callback: &ProgressCallback,
    cancellation: &CancellationToken,
) -> Result<(), UpdateError> {
    if cancellation.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Err(UpdateError::Cancelled),
        response = request_with_idle_timeout(client.get(asset.download_url.clone()).send()) => response?,
    };
    ensure_download_response(&response, asset.size)?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(partial)
        .await
        .map_err(|_| UpdateError::CacheIo)?;
    let mut reporter = ProgressReporter::new(progress_callback.clone());
    reporter.report(0, asset.size, true);

    let mut response = response;
    let mut received = 0u64;
    let mut hasher = Sha256::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(UpdateError::Cancelled),
            chunk = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, response.chunk()) => {
                chunk.map_err(|_| UpdateError::Timeout)?
                    .map_err(|error| if error.is_timeout() { UpdateError::Timeout } else { UpdateError::RequestFailed })?
            }
        };
        let Some(chunk) = chunk else {
            break;
        };
        if cancellation.is_cancelled() {
            return Err(UpdateError::Cancelled);
        }
        let chunk_size = u64::try_from(chunk.len()).map_err(|_| UpdateError::SizeMismatch)?;
        received = received
            .checked_add(chunk_size)
            .ok_or(UpdateError::SizeMismatch)?;
        if received > asset.size || received > super::MAX_DOWNLOAD_BYTES {
            return Err(UpdateError::SizeMismatch);
        }
        tokio::select! {
            _ = cancellation.cancelled() => return Err(UpdateError::Cancelled),
            result = file.write_all(&chunk) => result.map_err(|_| UpdateError::CacheIo)?,
        }
        hasher.update(&chunk);
        reporter.report(received, asset.size, false);
    }

    if received != asset.size {
        return Err(UpdateError::SizeMismatch);
    }
    let actual_hash = format!("{:x}", hasher.finalize());
    if actual_hash != asset.sha256 {
        return Err(UpdateError::HashMismatch);
    }
    tokio::select! {
        _ = cancellation.cancelled() => return Err(UpdateError::Cancelled),
        result = file.sync_all() => result.map_err(|_| UpdateError::CacheIo)?,
    }
    if cancellation.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    reporter.report(asset.size, asset.size, true);
    Ok(())
}

fn ensure_download_response(response: &Response, expected_size: u64) -> Result<(), UpdateError> {
    if !response.status().is_success() {
        return Err(UpdateError::HttpStatus(response.status().as_u16()));
    }
    if !is_trusted_github_url(response.url(), None) {
        return Err(UpdateError::RequestFailed);
    }
    if response
        .content_length()
        .is_some_and(|length| length != expected_size)
    {
        return Err(UpdateError::SizeMismatch);
    }
    Ok(())
}

async fn cache_entry_exists(path: &Path) -> Result<bool, UpdateError> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(UpdateError::CacheIo),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(UpdateError::CacheIo),
    }
}

async fn remove_partial(path: &Path) -> Result<(), UpdateError> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
            fs::remove_file(path)
                .await
                .map_err(|_| UpdateError::CacheIo)
        }
        Ok(_) => Err(UpdateError::CacheIo),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(UpdateError::CacheIo),
    }
}

async fn verify_file(
    path: &Path,
    expected_size: u64,
    expected_hash: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<(), UpdateError> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(UpdateError::Cancelled);
    }
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|_| UpdateError::CacheIo)?;
    if !metadata.file_type().is_file() {
        return Err(UpdateError::CacheIo);
    }
    if metadata.len() != expected_size {
        return Err(UpdateError::SizeMismatch);
    }
    let mut file = File::open(path).await.map_err(|_| UpdateError::CacheIo)?;
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    let mut hasher = Sha256::new();
    loop {
        let count = match cancellation {
            Some(cancellation) => tokio::select! {
                _ = cancellation.cancelled() => return Err(UpdateError::Cancelled),
                result = file.read(&mut buffer) => result,
            },
            None => file.read(&mut buffer).await,
        }
        .map_err(|_| UpdateError::CacheIo)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        Err(UpdateError::Cancelled)
    } else if format!("{:x}", hasher.finalize()) == expected_hash {
        Ok(())
    } else {
        Err(UpdateError::HashMismatch)
    }
}

async fn set_verified_asset_permissions(
    asset: &UpdateAsset,
    path: &Path,
) -> Result<(), UpdateError> {
    #[cfg(target_os = "linux")]
    if asset.name.ends_with(".AppImage") {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|_| UpdateError::CacheIo)?;
    }

    #[cfg(not(target_os = "linux"))]
    let _ = (asset, path);

    Ok(())
}

struct ProgressReporter {
    callback: ProgressCallback,
    last_reported: Option<Instant>,
}

impl ProgressReporter {
    fn new(callback: ProgressCallback) -> Self {
        Self {
            callback,
            last_reported: None,
        }
    }

    fn report(&mut self, received_bytes: u64, total_bytes: u64, force: bool) {
        let now = Instant::now();
        if force
            || self
                .last_reported
                .is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL)
        {
            (self.callback)(DownloadProgress {
                received_bytes,
                total_bytes,
            });
            self.last_reported = Some(now);
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod appimage_permission_tests {
    use super::verify_cached_download;
    use crate::update::{UpdateAsset, UpdateError};
    use sha2::{Digest, Sha256};
    use std::{fs, os::unix::fs::PermissionsExt};
    use url::Url;

    const PACKAGE_BYTES: &[u8] = b"verified Linux update package";

    #[tokio::test]
    async fn verified_cache_sets_execute_bits_only_for_valid_appimages()
    -> Result<(), Box<dyn std::error::Error>> {
        let cache = tempfile::tempdir()?;
        let appimage_name = "Codex-Auth-Switcher-0.3.1-linux-x86_64.AppImage";
        let appimage_path = cache.path().join(appimage_name);
        fs::write(&appimage_path, PACKAGE_BYTES)?;
        fs::set_permissions(&appimage_path, fs::Permissions::from_mode(0o644))?;

        let appimage = test_asset(appimage_name, PACKAGE_BYTES)?;
        let verified = verify_cached_download(cache.path(), &appimage, &appimage_path).await?;
        assert!(verified.from_cache);
        assert_eq!(
            fs::metadata(&appimage_path)?.permissions().mode() & 0o777,
            0o755
        );

        let archive_name = "Codex-Auth-Switcher-0.3.1-linux-amd64.deb";
        let archive_path = cache.path().join(archive_name);
        fs::write(&archive_path, PACKAGE_BYTES)?;
        fs::set_permissions(&archive_path, fs::Permissions::from_mode(0o644))?;

        let archive = test_asset(archive_name, PACKAGE_BYTES)?;
        let verified = verify_cached_download(cache.path(), &archive, &archive_path).await?;
        assert!(verified.from_cache);
        assert_eq!(
            fs::metadata(&archive_path)?.permissions().mode() & 0o777,
            0o644
        );

        let invalid_name = "Codex-Auth-Switcher-0.3.1-linux-x86_64-invalid.AppImage";
        let invalid_path = cache.path().join(invalid_name);
        fs::write(&invalid_path, PACKAGE_BYTES)?;
        fs::set_permissions(&invalid_path, fs::Permissions::from_mode(0o644))?;

        let mut invalid = test_asset(invalid_name, PACKAGE_BYTES)?;
        invalid.sha256 = "0".repeat(64);
        assert_eq!(
            verify_cached_download(cache.path(), &invalid, &invalid_path).await,
            Err(UpdateError::HashMismatch)
        );
        assert_eq!(
            fs::metadata(&invalid_path)?.permissions().mode() & 0o777,
            0o644
        );
        Ok(())
    }

    fn test_asset(name: &str, bytes: &[u8]) -> Result<UpdateAsset, url::ParseError> {
        Ok(UpdateAsset {
            name: name.to_owned(),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            download_url: Url::parse(&format!(
                "https://github.com/bruceblink/codex-auth-switcher/releases/download/v0.3.1/{name}"
            ))?,
        })
    }
}
