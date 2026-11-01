//! Stable GitHub Release checks and verified installer downloads.

mod download;
mod metadata;
mod platform;
mod runtime;

#[cfg(test)]
mod local_release_tests;
#[cfg(test)]
mod tests;

use reqwest::{Certificate, Client, redirect};
use semver::Version;
use std::{
    fmt,
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;
use url::Url;

pub use platform::{UpdatePlatform, current_platform};

const REPOSITORY: &str = "bruceblink/codex-auth-switcher";
const LATEST_RELEASE_URL: &str =
    "https://github.com/bruceblink/codex-auth-switcher/releases/latest";
const RELEASE_TAG_PREFIX: &str = "https://github.com/bruceblink/codex-auth-switcher/releases/tag/";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/bruceblink/codex-auth-switcher/releases/download/";
const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_RELEASE_NOTES_BYTES: usize = 64 * 1024;
const MAX_RELEASE_ASSETS: usize = 64;
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const METADATA_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const METADATA_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_DOWNLOAD_DURATION: Duration = Duration::from_secs(30 * 60);

type ProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync + 'static>;

/// Information shared by the available and non-downloadable release states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateRelease {
    pub version: Version,
    pub tag_name: String,
    pub notes: String,
    pub published_at: String,
    pub release_url: Url,
}

/// A newer release and the exact installer selected for this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub release: UpdateRelease,
    pub asset: UpdateAsset,
}

/// A validated installer entry from a tag-pinned update manifest.
///
/// Fields that control the download URL and cache path are private. Callers
/// can inspect the displayed metadata but cannot construct an arbitrary URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateAsset {
    name: String,
    size: u64,
    sha256: String,
    download_url: Url,
}

impl UpdateAsset {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Result of comparing the latest stable GitHub release with this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    Current { current_version: Version },
    Available(UpdateInfo),
    LegacyRelease { release: UpdateRelease },
    UnsupportedPlatform { release: UpdateRelease },
}

/// Bytes written and expected bytes for the selected installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadProgress {
    pub received_bytes: u64,
    pub total_bytes: u64,
}

/// A verified installer in the app-owned update cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadedUpdate {
    pub path: PathBuf,
    pub from_cache: bool,
}

/// Shared cancellation signal for a download.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: watch::Sender<bool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        let (cancelled, _) = watch::channel(false);
        Self { cancelled }
    }

    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }

    async fn cancelled(&self) {
        let mut receiver = self.cancelled.subscribe();
        loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors returned by update checks and verified downloads.
///
/// Messages intentionally omit response bodies, request URLs, and local paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateError {
    InvalidVersion,
    RuntimeUnavailable,
    RuntimeTask,
    RequestFailed,
    NoRelease,
    HttpStatus(u16),
    InvalidManifest,
    Cancelled,
    CacheIo,
    Timeout,
    SizeMismatch,
    HashMismatch,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVersion => formatter.write_str("The application version is invalid."),
            Self::RuntimeUnavailable => formatter.write_str("The update service could not start."),
            Self::RuntimeTask => formatter.write_str("The update operation stopped unexpectedly."),
            Self::RequestFailed => {
                formatter.write_str("Could not reach GitHub. Check the connection and retry.")
            }
            Self::NoRelease => formatter.write_str("GitHub has no published stable release."),
            Self::HttpStatus(status) => write!(formatter, "GitHub returned HTTP {status}."),
            Self::InvalidManifest => formatter.write_str("The GitHub update metadata is invalid."),
            Self::Cancelled => formatter.write_str("The update download was cancelled."),
            Self::CacheIo => formatter.write_str("The update cache could not be read or written."),
            Self::Timeout => formatter.write_str("The update request timed out."),
            Self::SizeMismatch => {
                formatter.write_str("The installer size does not match release metadata.")
            }
            Self::HashMismatch => formatter.write_str("The installer SHA-256 check failed."),
        }
    }
}

impl std::error::Error for UpdateError {}

impl UpdateError {
    pub fn is_cancelled(self) -> bool {
        self == Self::Cancelled
    }
}

#[derive(Clone)]
pub struct UpdateClient {
    metadata_client: Client,
    download_client: Client,
    cache_dir: PathBuf,
    current_version: Version,
}

impl UpdateClient {
    /// Creates a client whose cache belongs to the application, not CODEX_HOME.
    pub fn new(cache_dir: PathBuf) -> Result<Self, UpdateError> {
        Self::build(cache_dir, None, None)
    }

    fn build(
        cache_dir: PathBuf,
        test_server: Option<TestServer>,
        certificate: Option<&[u8]>,
    ) -> Result<Self, UpdateError> {
        let current_version =
            Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| UpdateError::InvalidVersion)?;
        let user_agent = format!("Codex-Auth-Switcher/{current_version}");
        let metadata_client = build_http_client(
            &user_agent,
            METADATA_CONNECT_TIMEOUT,
            Some(METADATA_TIMEOUT),
            test_server,
            certificate,
        )?;
        let download_client = build_http_client(
            &user_agent,
            DOWNLOAD_CONNECT_TIMEOUT,
            None,
            test_server,
            certificate,
        )?;
        Ok(Self {
            metadata_client,
            download_client,
            cache_dir,
            current_version,
        })
    }

    pub fn current_version(&self) -> &Version {
        &self.current_version
    }

    /// Runs the request on the updater runtime and can be awaited by GPUI.
    pub async fn check(&self) -> Result<UpdateCheck, UpdateError> {
        let client = self.metadata_client.clone();
        let current_version = self.current_version.clone();
        runtime::spawn(async move { metadata::check_for_update(&client, current_version).await })
            .await
    }

    /// Downloads an asset produced by `check`, verifies it, then publishes it
    /// atomically in the supplied app-owned cache directory.
    pub async fn download<F>(
        &self,
        asset: UpdateAsset,
        cancellation: CancellationToken,
        on_progress: F,
    ) -> Result<DownloadedUpdate, UpdateError>
    where
        F: Fn(DownloadProgress) + Send + Sync + 'static,
    {
        let client = self.download_client.clone();
        let cache_dir = self.cache_dir.clone();
        let progress: ProgressCallback = Arc::new(on_progress);
        runtime::spawn(async move {
            download::download_verified_asset(&client, asset, cache_dir, progress, cancellation)
                .await
        })
        .await
    }

    /// Revalidates a downloaded installer before the caller launches it.
    pub async fn verify_download(
        &self,
        asset: UpdateAsset,
        path: PathBuf,
    ) -> Result<DownloadedUpdate, UpdateError> {
        let cache_dir = self.cache_dir.clone();
        runtime::spawn(
            async move { download::verify_cached_download(&cache_dir, &asset, &path).await },
        )
        .await
    }

    #[cfg(test)]
    fn for_local_server(
        cache_dir: PathBuf,
        port: u16,
        certificate: &[u8],
    ) -> Result<Self, UpdateError> {
        Self::build(cache_dir, Some(TestServer { port }), Some(certificate))
    }
}

fn build_http_client(
    user_agent: &str,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
    test_server: Option<TestServer>,
    certificate: Option<&[u8]>,
) -> Result<Client, UpdateError> {
    let test_port = test_server.map(|server| server.port);
    let mut builder = Client::builder()
        .connect_timeout(connect_timeout)
        .redirect(trusted_redirect_policy(test_port))
        .user_agent(user_agent);
    if let Some(timeout) = request_timeout {
        builder = builder.timeout(timeout);
    }
    if let (Some(server), Some(certificate)) = (test_server, certificate) {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), server.port);
        let addresses = [address];
        let certificate =
            Certificate::from_pem(certificate).map_err(|_| UpdateError::InvalidManifest)?;
        builder = builder
            .no_proxy()
            .resolve_to_addrs("github.com", &addresses)
            .resolve_to_addrs("release-assets.githubusercontent.com", &addresses)
            .tls_certs_only([certificate]);
    }
    builder.build().map_err(|_| UpdateError::RequestFailed)
}

fn trusted_redirect_policy(test_port: Option<u16>) -> redirect::Policy {
    redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("update request exceeded redirect limit");
        }
        if is_trusted_github_url(attempt.url(), test_port) {
            attempt.follow()
        } else {
            attempt.error("update request redirected to an untrusted host")
        }
    })
}

fn is_trusted_github_url(url: &Url, test_port: Option<u16>) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let trusted_host = host == "github.com" || host.ends_with(".githubusercontent.com");
    let trusted_port = url.port_or_known_default() == Some(443)
        || test_port.is_some_and(|port| url.port() == Some(port));
    url.scheme() == "https"
        && trusted_host
        && trusted_port
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn stable_tag(tag_name: &str) -> Result<Version, UpdateError> {
    let raw_version = tag_name
        .strip_prefix('v')
        .ok_or(UpdateError::InvalidManifest)?;
    let version = Version::parse(raw_version).map_err(|_| UpdateError::InvalidManifest)?;
    if version.to_string() != raw_version || !version.pre.is_empty() || !version.build.is_empty() {
        return Err(UpdateError::InvalidManifest);
    }
    Ok(version)
}

fn release_url(version: &Version) -> Result<Url, UpdateError> {
    let url = Url::parse(&format!("{RELEASE_TAG_PREFIX}v{version}"))
        .map_err(|_| UpdateError::InvalidManifest)?;
    if !is_trusted_github_url(&url, None) || url.query().is_some() {
        return Err(UpdateError::InvalidManifest);
    }
    Ok(url)
}

fn manifest_url(version: &Version) -> Result<Url, UpdateError> {
    Url::parse(&format!(
        "{RELEASE_DOWNLOAD_PREFIX}v{version}/update-manifest.json"
    ))
    .map_err(|_| UpdateError::InvalidManifest)
}

fn asset_url(version: &Version, asset_name: &str) -> Result<Url, UpdateError> {
    Url::parse(&format!("{RELEASE_DOWNLOAD_PREFIX}v{version}/{asset_name}"))
        .map_err(|_| UpdateError::InvalidManifest)
}

fn is_safe_asset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[derive(Clone, Copy)]
struct TestServer {
    port: u16,
}

async fn request_with_idle_timeout<T>(
    future: impl Future<Output = Result<T, reqwest::Error>>,
) -> Result<T, UpdateError> {
    tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, future)
        .await
        .map_err(|_| UpdateError::Timeout)?
        .map_err(|error| {
            if error.is_timeout() {
                UpdateError::Timeout
            } else {
                UpdateError::RequestFailed
            }
        })
}
