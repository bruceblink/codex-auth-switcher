use super::{
    CancellationToken, DownloadProgress, UpdateCheck, UpdateClient, UpdateError, UpdateInfo,
    UpdatePlatform, current_platform,
};
use semver::Version;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

const PACKAGE_BYTES: &[u8] = b"verified package contents for update client test";
const MODE_VALID: u8 = 0;
const MODE_TRUNCATED: u8 = 1;
const MODE_OVERSIZED: u8 = 2;
const MODE_DELAYED: u8 = 3;

/// Exercises the production GitHub URL, TLS verification, redirects, cache,
/// hashing, cancellation, and cleanup against the Docker HTTPS test service.
#[tokio::test]
#[ignore = "Run scripts/update/test-local-integration.ps1 to start the Docker HTTPS service."]
async fn docker_update_integration() {
    let root = required_path("UPDATE_TEST_ROOT");
    let cache_dir = required_path("UPDATE_TEST_CACHE");
    let certificate_path = required_path("UPDATE_TEST_TLS_CERT");
    let tls_port = required_port("UPDATE_TEST_HTTPS_PORT");
    let release_version = required_version("UPDATE_TEST_RELEASE_VERSION");
    let backend_mode_file = required_path("UPDATE_TEST_BACKEND_MODE_FILE");
    let certificate = match fs::read(certificate_path) {
        Ok(certificate) => certificate,
        Err(error) => panic!("could not read the test TLS certificate: {error}"),
    };
    let Some(platform) = current_platform() else {
        panic!("Docker update integration requires a supported x64/arm64 platform");
    };
    let client = match UpdateClient::for_local_server(cache_dir, tls_port, &certificate) {
        Ok(client) => client,
        Err(error) => panic!("could not create the local HTTPS update client: {error}"),
    };
    write_file(
        &root.join("assets").join("update-package.bin"),
        PACKAGE_BYTES,
    );
    let backend = BackendControl::new(backend_mode_file);

    write_manifest(
        &root,
        &release_version,
        platform,
        PACKAGE_BYTES.len() as u64,
        sha256(PACKAGE_BYTES),
    );
    let info = require_available(client.check().await);
    assert_eq!(info.release.version, release_version);
    assert_eq!(info.asset.name(), platform.asset_name(&release_version));
    assert_eq!(
        info.release.release_url.as_str(),
        format!(
            "https://github.com/bruceblink/codex-auth-switcher/releases/tag/v{release_version}"
        )
    );

    let progress_values = Arc::new(Mutex::new(Vec::<DownloadProgress>::new()));
    let callback_values = Arc::clone(&progress_values);
    let first = client
        .download(
            info.asset.clone(),
            CancellationToken::new(),
            move |progress| {
                if let Ok(mut values) = callback_values.lock() {
                    values.push(progress);
                }
            },
        )
        .await;
    let first = require_download(first);
    assert!(!first.from_cache);
    assert_eq!(read_file(&first.path), PACKAGE_BYTES);
    let progress = match progress_values.lock() {
        Ok(progress) => progress.clone(),
        Err(_) => panic!("progress callback lock was poisoned"),
    };
    assert!(
        progress
            .first()
            .is_some_and(|value| value.received_bytes == 0)
    );
    assert!(progress.last().is_some_and(|value| {
        value.received_bytes == value.total_bytes && value.total_bytes == PACKAGE_BYTES.len() as u64
    }));

    let cached = client
        .download(info.asset.clone(), CancellationToken::new(), |_| {})
        .await;
    assert!(require_download(cached).from_cache);

    let cache_cancellation = CancellationToken::new();
    let callback_cancellation = cache_cancellation.clone();
    let cancelled_cache = client
        .download(info.asset.clone(), cache_cancellation, move |progress| {
            if progress.received_bytes == 0 {
                callback_cancellation.cancel();
            }
        })
        .await;
    assert_eq!(cancelled_cache, Err(UpdateError::Cancelled));
    assert_eq!(read_file(&first.path), PACKAGE_BYTES);

    write_file(&first.path, b"corrupted cache");
    let repaired = client
        .download(info.asset.clone(), CancellationToken::new(), |_| {})
        .await;
    let repaired = require_download(repaired);
    assert!(!repaired.from_cache);
    assert_eq!(read_file(&repaired.path), PACKAGE_BYTES);

    let wrong_hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    write_manifest(
        &root,
        &release_version,
        platform,
        PACKAGE_BYTES.len() as u64,
        wrong_hash.to_owned(),
    );
    let bad_hash_info = require_available(client.check().await);
    let bad_hash_result = client
        .download(bad_hash_info.asset, CancellationToken::new(), |_| {})
        .await;
    assert_eq!(bad_hash_result, Err(UpdateError::HashMismatch));
    assert_no_partial(&repaired.path);
    assert!(!repaired.path.exists());

    write_manifest(
        &root,
        &release_version,
        platform,
        PACKAGE_BYTES.len() as u64,
        sha256(PACKAGE_BYTES),
    );
    let retry_info = require_available(client.check().await);
    let retry_result = client
        .download(retry_info.asset.clone(), CancellationToken::new(), |_| {})
        .await;
    let retry = require_download(retry_result);
    assert!(!retry.from_cache);
    assert_eq!(read_file(&retry.path), PACKAGE_BYTES);

    let verified = client
        .verify_download(retry_info.asset.clone(), retry.path.clone())
        .await;
    let verified = require_download(verified);
    assert!(verified.from_cache);
    assert_eq!(read_file(&verified.path), PACKAGE_BYTES);

    let tampered = vec![b'x'; PACKAGE_BYTES.len()];
    write_file(&retry.path, &tampered);
    let tampered_result = client
        .verify_download(retry_info.asset.clone(), retry.path.clone())
        .await;
    assert_eq!(tampered_result, Err(UpdateError::HashMismatch));
    write_file(&retry.path, PACKAGE_BYTES);

    let restored = client
        .verify_download(retry_info.asset.clone(), retry.path.clone())
        .await;
    let restored = require_download(restored);
    assert!(restored.from_cache);
    assert_eq!(read_file(&restored.path), PACKAGE_BYTES);

    fs::remove_file(&retry.path)
        .unwrap_or_else(|error| panic!("could not clear the retry cache: {error}"));
    backend.set_mode(MODE_TRUNCATED);
    let truncation_info = require_available(client.check().await);
    let truncated = client
        .download(truncation_info.asset, CancellationToken::new(), |_| {})
        .await;
    assert!(matches!(
        truncated,
        Err(UpdateError::SizeMismatch | UpdateError::RequestFailed | UpdateError::HttpStatus(_))
    ));
    assert_no_partial(&retry.path);
    assert!(!retry.path.exists());

    backend.set_mode(MODE_OVERSIZED);
    write_manifest(&root, &release_version, platform, 8, sha256(PACKAGE_BYTES));
    let overflow_info = require_available(client.check().await);
    let overflow = client
        .download(overflow_info.asset, CancellationToken::new(), |_| {})
        .await;
    assert_eq!(overflow, Err(UpdateError::SizeMismatch));
    assert_no_partial(&retry.path);

    backend.set_mode(MODE_DELAYED);
    write_manifest(
        &root,
        &release_version,
        platform,
        PACKAGE_BYTES.len() as u64,
        sha256(PACKAGE_BYTES),
    );
    let cancel_info = require_available(client.check().await);
    let cancellation = CancellationToken::new();
    let (progress_sender, progress_receiver) = oneshot::channel();
    let progress_sender = Arc::new(Mutex::new(Some(progress_sender)));
    let callback_sender = Arc::clone(&progress_sender);
    let download_client = client.clone();
    let download_cancellation = cancellation.clone();
    let download = tokio::spawn(async move {
        download_client
            .download(cancel_info.asset, download_cancellation, move |progress| {
                if progress.received_bytes > 0
                    && let Ok(mut sender) = callback_sender.lock()
                    && let Some(sender) = sender.take()
                {
                    let _ = sender.send(());
                }
            })
            .await
    });
    let first_chunk = tokio::time::timeout(Duration::from_secs(5), progress_receiver).await;
    assert!(first_chunk.is_ok());
    cancellation.cancel();
    let cancelled = tokio::time::timeout(Duration::from_secs(5), download).await;
    let Ok(cancelled) = cancelled else {
        panic!("cancelled update did not stop promptly");
    };
    let Ok(cancelled) = cancelled else {
        panic!("cancelled update task stopped unexpectedly");
    };
    assert_eq!(cancelled, Err(UpdateError::Cancelled));
    let cache_path = retry.path;
    assert_no_partial(&cache_path);
    assert!(!cache_path.exists());

    let manifest_path = root.join("update-manifest.json");
    let hidden_manifest_path = root.join("update-manifest.saved");
    fs::rename(&manifest_path, &hidden_manifest_path)
        .unwrap_or_else(|error| panic!("could not simulate a legacy release: {error}"));
    let legacy_result = client.check().await;
    fs::rename(&hidden_manifest_path, &manifest_path)
        .unwrap_or_else(|error| panic!("could not restore the test manifest: {error}"));
    let Ok(UpdateCheck::LegacyRelease { release }) = legacy_result else {
        panic!("a release without a manifest must not offer a package");
    };
    assert_eq!(release.version, release_version);
    assert_eq!(
        release.release_url.as_str(),
        format!(
            "https://github.com/bruceblink/codex-auth-switcher/releases/tag/v{release_version}"
        )
    );
    assert!(matches!(
        client.check().await,
        Ok(UpdateCheck::Available(_))
    ));
}

struct BackendControl {
    mode_file: PathBuf,
}

impl BackendControl {
    fn new(mode_file: PathBuf) -> Self {
        fs::write(&mode_file, MODE_VALID.to_string())
            .unwrap_or_else(|error| panic!("could not initialize the test backend mode: {error}"));
        Self { mode_file }
    }

    fn set_mode(&self, mode: u8) {
        fs::write(&self.mode_file, mode.to_string())
            .unwrap_or_else(|error| panic!("could not update the test backend mode: {error}"));
    }
}

fn write_manifest(
    root: &Path,
    version: &Version,
    selected_platform: UpdatePlatform,
    selected_size: u64,
    selected_hash: String,
) {
    let names = [
        UpdatePlatform::WindowsX64,
        UpdatePlatform::LinuxAmd64Deb,
        UpdatePlatform::LinuxX86_64AppImage,
        UpdatePlatform::MacosArm64,
    ];
    let assets = names
        .into_iter()
        .map(|platform| {
            if platform == selected_platform {
                json!({
                    "name": platform.asset_name(version),
                    "size": selected_size,
                    "sha256": selected_hash,
                })
            } else {
                json!({
                    "name": platform.asset_name(version),
                    "size": PACKAGE_BYTES.len(),
                    "sha256": sha256(PACKAGE_BYTES),
                })
            }
        })
        .collect::<Vec<_>>();
    let manifest = json!({
        "schema_version": 1,
        "version": version.to_string(),
        "tag_name": format!("v{version}"),
        "notes": "Local Docker integration release",
        "published_at": "2026-11-01T00:00:00Z",
        "assets": assets,
    });
    fs::create_dir_all(root)
        .unwrap_or_else(|error| panic!("could not create local release data: {error}"));
    let bytes = serde_json::to_vec(&manifest)
        .unwrap_or_else(|error| panic!("could not encode local update manifest: {error}"));
    fs::write(root.join("update-manifest.json"), bytes)
        .unwrap_or_else(|error| panic!("could not write local update manifest: {error}"));
}

fn require_available(result: Result<UpdateCheck, UpdateError>) -> UpdateInfo {
    match result {
        Ok(UpdateCheck::Available(info)) => info,
        Ok(state) => panic!("expected available update, received {state:?}"),
        Err(error) => panic!("update check failed: {error}"),
    }
}

fn require_download(
    result: Result<super::DownloadedUpdate, UpdateError>,
) -> super::DownloadedUpdate {
    match result {
        Ok(download) => download,
        Err(error) => panic!("verified download failed: {error}"),
    }
}

fn required_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("missing integration-test environment variable {name}"))
}

fn required_port(name: &str) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|port| *port != 0)
        .unwrap_or_else(|| panic!("invalid integration-test port {name}"))
}

fn required_version(name: &str) -> Version {
    let version = std::env::var(name)
        .ok()
        .and_then(|value| Version::parse(&value).ok())
        .unwrap_or_else(|| panic!("invalid integration-test release version {name}"));
    assert!(version.pre.is_empty());
    version
}

fn read_file(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("could not read verified package: {error}"))
}

fn write_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes)
        .unwrap_or_else(|error| panic!("could not alter cached package: {error}"));
}

fn assert_no_partial(path: &Path) {
    let partial = path.with_file_name(format!(
        "{}.part",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    ));
    assert!(!partial.exists());
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
