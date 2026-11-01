use std::{
    future::Future,
    io::{self, Write},
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::TimeZone as _;
use serde::{Deserialize, Serialize};
use switcher_core::update::{
    CancellationToken, DownloadProgress, DownloadedUpdate, UpdateAsset, UpdateCheck, UpdateClient,
    UpdateError, UpdateInfo, UpdateRelease,
};

use super::Switcher;

const PREFERENCES_FILE: &str = "update-preferences.json";
const CACHE_DIRECTORY: &str = "update-cache";
const STARTUP_CHECK_DELAY: Duration = Duration::from_secs(3);
const AUTO_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_millis(125);

type UpdateTask<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
pub(crate) type ProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync + 'static>;

/// The view uses these checked values while the backend keeps the validated
/// asset that may be handed to the downloader.
#[derive(Clone)]
pub(crate) struct ReleaseView {
    pub(crate) version: String,
    pub(crate) tag_name: String,
    pub(crate) notes: String,
    pub(crate) published_at: String,
    pub(crate) release_url: String,
}

#[derive(Clone)]
pub(crate) struct PackageView {
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    asset: Option<UpdateAsset>,
}

#[derive(Clone)]
pub(crate) struct AvailableView {
    pub(crate) release: ReleaseView,
    pub(crate) package: PackageView,
}

#[cfg(test)]
impl AvailableView {
    pub(crate) fn test_value() -> Self {
        Self::test_value_with_notes("测试更新说明".to_owned())
    }

    pub(crate) fn test_value_with_notes(notes: String) -> Self {
        Self {
            release: ReleaseView {
                version: "0.4.0".to_owned(),
                tag_name: "v0.4.0".to_owned(),
                notes,
                published_at: "2026-11-01T00:00:00Z".to_owned(),
                release_url:
                    "https://github.com/bruceblink/codex-auth-switcher/releases/tag/v0.4.0"
                        .to_owned(),
            },
            package: PackageView {
                name: "Codex-Auth-Switcher-test-windows-x64-setup.exe".to_owned(),
                size: 1024,
                sha256: "a".repeat(64),
                asset: None,
            },
        }
    }
}

#[derive(Clone)]
pub(crate) enum CheckResult {
    Current { current_version: String },
    Available(AvailableView),
    LegacyRelease { release: ReleaseView },
    UnsupportedPlatform { release: ReleaseView },
}

#[derive(Clone, Debug)]
pub(crate) struct UpdateFailure {
    pub(crate) message: String,
    pub(crate) cancelled: bool,
}

pub(crate) trait UpdateBackend: Send + Sync {
    fn check(&self) -> UpdateTask<Result<CheckResult, UpdateFailure>>;

    fn download(
        &self,
        asset: Option<UpdateAsset>,
        cancellation: CancellationToken,
        on_progress: ProgressCallback,
    ) -> UpdateTask<Result<DownloadedUpdate, UpdateFailure>>;

    fn verify_download(
        &self,
        asset: Option<UpdateAsset>,
        path: PathBuf,
    ) -> UpdateTask<Result<DownloadedUpdate, UpdateFailure>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateHandoff {
    SystemOpenerStarted,
    AppImageDirectoryOpened,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdatePackageKind {
    Installer,
    AppImage,
    Dmg,
}

pub(crate) fn update_package_kind(name: &str) -> UpdatePackageKind {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some(extension) if extension.eq_ignore_ascii_case("AppImage") => {
            UpdatePackageKind::AppImage
        }
        Some(extension) if extension.eq_ignore_ascii_case("dmg") => UpdatePackageKind::Dmg,
        _ => UpdatePackageKind::Installer,
    }
}

enum InstallAttemptError {
    Verification(String),
    Handoff(String),
}

pub(crate) trait UpdateInstaller: Send + Sync {
    fn handoff(
        &self,
        path: &Path,
        expected_name: &str,
        cache_directory: &Path,
        package_kind: UpdatePackageKind,
    ) -> Result<UpdateHandoff, String>;
}

struct SystemUpdateInstaller;

impl UpdateInstaller for SystemUpdateInstaller {
    fn handoff(
        &self,
        path: &Path,
        expected_name: &str,
        cache_directory: &Path,
        package_kind: UpdatePackageKind,
    ) -> Result<UpdateHandoff, String> {
        if package_kind == UpdatePackageKind::AppImage {
            reveal_verified_package(path, expected_name, cache_directory)?;
            return Ok(UpdateHandoff::AppImageDirectoryOpened);
        }

        #[cfg(target_os = "windows")]
        let mut command = {
            use std::os::windows::process::CommandExt as _;
            let mut command = Command::new(path);
            command.creation_flags(0x0800_0000);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("open");
            command.arg(path);
            command
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut command = {
            let mut command = Command::new("xdg-open");
            command.arg(path);
            command
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| "无法打开更新安装包。".to_owned())?;
        Ok(UpdateHandoff::SystemOpenerStarted)
    }
}

struct CoreUpdateBackend {
    client: Arc<UpdateClient>,
}

impl UpdateBackend for CoreUpdateBackend {
    fn check(&self) -> UpdateTask<Result<CheckResult, UpdateFailure>> {
        let client = self.client.clone();
        Box::pin(async move {
            client
                .check()
                .await
                .map(check_result)
                .map_err(update_failure)
        })
    }

    fn download(
        &self,
        asset: Option<UpdateAsset>,
        cancellation: CancellationToken,
        on_progress: ProgressCallback,
    ) -> UpdateTask<Result<DownloadedUpdate, UpdateFailure>> {
        let client = self.client.clone();
        let Some(asset) = asset else {
            return Box::pin(async {
                Err(UpdateFailure {
                    message: "无法读取已验证的更新安装包信息。请重新检查更新。".to_owned(),
                    cancelled: false,
                })
            });
        };
        Box::pin(async move {
            client
                .download(asset, cancellation, move |progress| on_progress(progress))
                .await
                .map_err(update_failure)
        })
    }

    fn verify_download(
        &self,
        asset: Option<UpdateAsset>,
        path: PathBuf,
    ) -> UpdateTask<Result<DownloadedUpdate, UpdateFailure>> {
        let client = self.client.clone();
        let Some(asset) = asset else {
            return Box::pin(async {
                Err(UpdateFailure {
                    message: "无法读取已验证的更新安装包信息。请重新检查更新。".to_owned(),
                    cancelled: false,
                })
            });
        };
        Box::pin(async move {
            client
                .verify_download(asset, path)
                .await
                .map_err(update_failure)
        })
    }
}

fn check_result(check: UpdateCheck) -> CheckResult {
    match check {
        UpdateCheck::Current { current_version } => CheckResult::Current {
            current_version: current_version.to_string(),
        },
        UpdateCheck::Available(info) => CheckResult::Available(available_view(info)),
        UpdateCheck::LegacyRelease { release } => CheckResult::LegacyRelease {
            release: release_view(release),
        },
        UpdateCheck::UnsupportedPlatform { release } => CheckResult::UnsupportedPlatform {
            release: release_view(release),
        },
    }
}

fn available_view(info: UpdateInfo) -> AvailableView {
    let release = release_view(info.release);
    let package = PackageView {
        name: info.asset.name().to_owned(),
        size: info.asset.size(),
        sha256: info.asset.sha256().to_owned(),
        asset: Some(info.asset),
    };
    AvailableView { release, package }
}

fn release_view(release: UpdateRelease) -> ReleaseView {
    ReleaseView {
        version: release.version.to_string(),
        tag_name: release.tag_name,
        notes: release.notes,
        published_at: release.published_at,
        release_url: release.release_url.to_string(),
    }
}

fn update_failure(error: UpdateError) -> UpdateFailure {
    let message = match error {
        UpdateError::InvalidVersion => "当前程序版本无效。".to_owned(),
        UpdateError::RuntimeUnavailable => "软件更新服务无法启动。".to_owned(),
        UpdateError::RuntimeTask => "软件更新任务意外停止。".to_owned(),
        UpdateError::RequestFailed => "无法连接到 GitHub。请检查网络后重试。".to_owned(),
        UpdateError::NoRelease => "GitHub 尚未发布稳定版更新。".to_owned(),
        UpdateError::HttpStatus(status) => {
            format!("GitHub 返回 HTTP {status}。请稍后重试。")
        }
        UpdateError::InvalidManifest => "更新清单无效，未下载任何文件。".to_owned(),
        UpdateError::Cancelled => "下载已取消。".to_owned(),
        UpdateError::CacheIo => "无法读取或写入更新缓存。请检查磁盘空间和目录权限。".to_owned(),
        UpdateError::Timeout => "连接超时。请检查网络后重试。".to_owned(),
        UpdateError::SizeMismatch => "安装包大小与更新清单不一致。请重新下载后再试。".to_owned(),
        UpdateError::HashMismatch => "安装包 SHA-256 校验失败。请重新下载后再试。".to_owned(),
    };
    UpdateFailure {
        message,
        cancelled: error.is_cancelled(),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct LastSuccessfulCheck {
    pub(crate) installed_version: String,
    pub(crate) unix_seconds: u64,
    #[serde(default)]
    pub(crate) latest_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdatePreferences {
    pub(crate) auto_check: bool,
    pub(crate) last_successful_check: Option<LastSuccessfulCheck>,
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            auto_check: true,
            last_successful_check: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct UpdatePreferencesStore {
    location: PreferencesLocation,
}

#[derive(Clone)]
enum PreferencesLocation {
    File {
        path: PathBuf,
        lock: Arc<Mutex<()>>,
    },
    #[cfg(test)]
    Memory(Arc<Mutex<UpdatePreferences>>),
    Unavailable(String),
}

impl UpdatePreferencesStore {
    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            location: PreferencesLocation::File {
                path,
                lock: Arc::new(Mutex::new(())),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self {
            location: PreferencesLocation::Memory(Arc::new(Mutex::new(
                UpdatePreferences::default(),
            ))),
        }
    }

    fn unavailable(message: String) -> Self {
        Self {
            location: PreferencesLocation::Unavailable(message),
        }
    }

    pub(crate) fn load(&self) -> Result<UpdatePreferences, String> {
        match &self.location {
            PreferencesLocation::File { path, .. } => {
                let bytes = match std::fs::read(path) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        return Ok(UpdatePreferences::default());
                    }
                    Err(_) => return Err("无法读取软件更新设置。".to_owned()),
                };
                serde_json::from_slice(&bytes)
                    .map_err(|_| "软件更新设置文件无效，已使用默认设置。".to_owned())
            }
            #[cfg(test)]
            PreferencesLocation::Memory(preferences) => Ok(preferences
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()),
            PreferencesLocation::Unavailable(message) => Err(message.clone()),
        }
    }

    pub(crate) fn save(&self, preferences: &UpdatePreferences) -> Result<(), String> {
        match &self.location {
            PreferencesLocation::File { path, lock } => {
                let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
                let Some(directory) = path.parent() else {
                    return Err("软件更新设置路径无效。".to_owned());
                };
                std::fs::create_dir_all(directory)
                    .map_err(|_| "无法创建软件更新设置目录。".to_owned())?;
                let bytes = serde_json::to_vec_pretty(preferences)
                    .map_err(|_| "无法生成软件更新设置文件。".to_owned())?;
                let mut temporary = tempfile::NamedTempFile::new_in(directory)
                    .map_err(|_| "无法创建软件更新设置临时文件。".to_owned())?;
                temporary
                    .write_all(&bytes)
                    .and_then(|()| temporary.as_file().sync_all())
                    .map_err(|_| "无法写入软件更新设置。".to_owned())?;
                temporary
                    .persist(path)
                    .map(|_| ())
                    .map_err(|_| "无法原子替换软件更新设置文件。".to_owned())
            }
            #[cfg(test)]
            PreferencesLocation::Memory(stored) => {
                *stored.lock().unwrap_or_else(|error| error.into_inner()) = preferences.clone();
                Ok(())
            }
            PreferencesLocation::Unavailable(message) => Err(message.clone()),
        }
    }
}

#[derive(Clone)]
pub(crate) enum UpdateStatus {
    Idle,
    Checking,
    Current {
        current_version: String,
    },
    Available(AvailableView),
    LegacyRelease {
        release: ReleaseView,
    },
    UnsupportedPlatform {
        release: ReleaseView,
    },
    Downloading {
        update: AvailableView,
        progress: Option<DownloadProgress>,
    },
    Cancelling {
        update: AvailableView,
        progress: Option<DownloadProgress>,
    },
    VerifyingInstall {
        update: AvailableView,
    },
    Ready {
        update: AvailableView,
        path: PathBuf,
    },
    FailedCheck {
        message: String,
    },
    FailedDownload {
        update: AvailableView,
        message: String,
    },
}

pub(crate) struct UpdateController {
    backend: Option<Arc<dyn UpdateBackend>>,
    pub(crate) preferences: UpdatePreferences,
    pub(crate) preferences_loaded: bool,
    pub(crate) preferences_saving: bool,
    pub(crate) preferences_error: Option<String>,
    pub(crate) operation_error: bool,
    pub(crate) status: UpdateStatus,
    pub(crate) update_attention: bool,
    store: UpdatePreferencesStore,
    data_directory: Option<PathBuf>,
    setup_error: Option<String>,
    pub(crate) operation_notice: Option<String>,
    cancellation: Option<CancellationToken>,
    installer: Arc<dyn UpdateInstaller>,
    preferences_revision: u64,
}

impl UpdateController {
    fn new(
        backend: Option<Arc<dyn UpdateBackend>>,
        store: UpdatePreferencesStore,
        data_directory: Option<PathBuf>,
        setup_error: Option<String>,
        preferences: UpdatePreferences,
        preferences_loaded: bool,
        installer: Arc<dyn UpdateInstaller>,
    ) -> Self {
        Self {
            backend,
            preferences,
            preferences_loaded,
            preferences_saving: false,
            preferences_error: None,
            operation_error: false,
            status: UpdateStatus::Idle,
            update_attention: false,
            store,
            data_directory,
            setup_error,
            operation_notice: None,
            cancellation: None,
            installer,
            preferences_revision: 0,
        }
    }

    pub(crate) fn system() -> Self {
        let data_directory = match application_data_directory() {
            Ok(directory) => directory,
            Err(error) => {
                return Self::new(
                    None,
                    UpdatePreferencesStore::unavailable(error.clone()),
                    None,
                    Some(error),
                    UpdatePreferences::default(),
                    false,
                    Arc::new(SystemUpdateInstaller),
                );
            }
        };
        let store = UpdatePreferencesStore::at(data_directory.join(PREFERENCES_FILE));
        let cache_directory = data_directory.join(CACHE_DIRECTORY);
        match UpdateClient::new(cache_directory) {
            Ok(client) => Self::new(
                Some(Arc::new(CoreUpdateBackend {
                    client: Arc::new(client),
                })),
                store,
                Some(data_directory),
                None,
                UpdatePreferences::default(),
                false,
                Arc::new(SystemUpdateInstaller),
            ),
            Err(error) => Self::new(
                None,
                store,
                Some(data_directory),
                Some(format!(
                    "无法初始化软件更新：{}",
                    update_failure(error).message
                )),
                UpdatePreferences::default(),
                false,
                Arc::new(SystemUpdateInstaller),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn isolated(
        data_directory: PathBuf,
        backend: Option<Arc<dyn UpdateBackend>>,
        preferences: UpdatePreferences,
    ) -> Self {
        Self::new(
            backend,
            UpdatePreferencesStore::at(data_directory.join(PREFERENCES_FILE)),
            Some(data_directory),
            None,
            preferences,
            true,
            Arc::new(SystemUpdateInstaller),
        )
    }

    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self::new(
            None,
            UpdatePreferencesStore::memory(),
            None,
            None,
            UpdatePreferences::default(),
            true,
            Arc::new(SystemUpdateInstaller),
        )
    }

    #[cfg(test)]
    pub(crate) fn isolated_with_installer(
        data_directory: PathBuf,
        backend: Option<Arc<dyn UpdateBackend>>,
        preferences: UpdatePreferences,
        installer: Arc<dyn UpdateInstaller>,
    ) -> Self {
        Self::new(
            backend,
            UpdatePreferencesStore::at(data_directory.join(PREFERENCES_FILE)),
            Some(data_directory),
            None,
            preferences,
            true,
            installer,
        )
    }

    pub(crate) fn cache_directory(&self) -> Option<PathBuf> {
        self.data_directory
            .as_ref()
            .map(|directory| directory.join(CACHE_DIRECTORY))
    }

    pub(crate) fn auto_check_is_due(&self, now: u64) -> bool {
        if !self.preferences.auto_check {
            return false;
        }
        let current_version = env!("CARGO_PKG_VERSION");
        !self
            .preferences
            .last_successful_check
            .as_ref()
            .is_some_and(|last| {
                last.installed_version == current_version
                    && last.latest_version.as_deref() == Some(current_version)
                    && now >= last.unix_seconds
                    && now.saturating_sub(last.unix_seconds) < AUTO_CHECK_INTERVAL.as_secs()
            })
    }

    pub(crate) fn status_is_active(&self) -> bool {
        matches!(
            &self.status,
            UpdateStatus::Checking
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::Cancelling { .. }
                | UpdateStatus::VerifyingInstall { .. }
        )
    }

    pub(crate) fn release(&self) -> Option<&ReleaseView> {
        match &self.status {
            UpdateStatus::Available(update)
            | UpdateStatus::Downloading { update, .. }
            | UpdateStatus::Cancelling { update, .. }
            | UpdateStatus::VerifyingInstall { update }
            | UpdateStatus::Ready { update, .. }
            | UpdateStatus::FailedDownload { update, .. } => Some(&update.release),
            UpdateStatus::LegacyRelease { release }
            | UpdateStatus::UnsupportedPlatform { release } => Some(release),
            UpdateStatus::Idle
            | UpdateStatus::Checking
            | UpdateStatus::Current { .. }
            | UpdateStatus::FailedCheck { .. } => None,
        }
    }

    pub(crate) fn message(&self) -> String {
        if let Some(error) = &self.preferences_error {
            return format!("{} 更新设置失败：{error}", self.status_message());
        }
        self.operation_notice
            .clone()
            .unwrap_or_else(|| self.status_message())
    }

    fn status_message(&self) -> String {
        if let Some(error) = &self.setup_error {
            return error.clone();
        }
        match &self.status {
            UpdateStatus::Idle if !self.preferences_loaded => "正在读取软件更新设置。".into(),
            UpdateStatus::Idle => "可以手动检查稳定版更新。".into(),
            UpdateStatus::Checking => "正在检查更新…".into(),
            UpdateStatus::Current { .. } => "已是最新版本。".into(),
            UpdateStatus::Available(_) => "发现可用更新。".into(),
            UpdateStatus::LegacyRelease { .. } => {
                "发现新版，但没有可验证的更新清单。请查看发布说明。".into()
            }
            UpdateStatus::UnsupportedPlatform { .. } => {
                "发现新版，但没有适用于当前平台的安装包。".into()
            }
            UpdateStatus::Downloading { .. } => "正在下载并校验安装包…".into(),
            UpdateStatus::Cancelling { .. } => "正在取消下载并清理临时文件…".into(),
            UpdateStatus::VerifyingInstall { .. } => "正在重新校验安装包…".into(),
            UpdateStatus::Ready { .. } => "安装包已下载并通过校验。请手动安装。".into(),
            UpdateStatus::FailedCheck { message }
            | UpdateStatus::FailedDownload { message, .. } => message.clone(),
        }
    }

    pub(crate) fn release_url(&self) -> Option<&str> {
        self.release().map(|release| release.release_url.as_str())
    }

    pub(crate) fn startup_check(&self) -> bool {
        !cfg!(debug_assertions) && self.backend.is_some()
    }

    fn load_preferences(&self) -> UpdateTask<Result<UpdatePreferences, String>> {
        let store = self.store.clone();
        Box::pin(async move { store.load() })
    }

    fn persist_preferences(
        &self,
        preferences: UpdatePreferences,
    ) -> UpdateTask<Result<(), String>> {
        let store = self.store.clone();
        Box::pin(async move { store.save(&preferences) })
    }

    fn apply_check_result(
        &mut self,
        result: Result<CheckResult, UpdateFailure>,
        now: u64,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Switcher>,
    ) {
        match result {
            Ok(result) => {
                self.operation_notice = None;
                self.operation_error = false;
                let latest_version = match &result {
                    CheckResult::Current { current_version } => current_version.clone(),
                    CheckResult::Available(update) => update.release.version.clone(),
                    CheckResult::LegacyRelease { release }
                    | CheckResult::UnsupportedPlatform { release } => release.version.clone(),
                };
                self.status = match result {
                    CheckResult::Current { current_version } => {
                        self.update_attention = false;
                        UpdateStatus::Current { current_version }
                    }
                    CheckResult::Available(update) => {
                        self.update_attention = true;
                        UpdateStatus::Available(update)
                    }
                    CheckResult::LegacyRelease { release } => {
                        self.update_attention = false;
                        UpdateStatus::LegacyRelease { release }
                    }
                    CheckResult::UnsupportedPlatform { release } => {
                        self.update_attention = false;
                        UpdateStatus::UnsupportedPlatform { release }
                    }
                };
                self.preferences.last_successful_check = Some(LastSuccessfulCheck {
                    installed_version: env!("CARGO_PKG_VERSION").to_owned(),
                    unix_seconds: now,
                    latest_version: Some(latest_version),
                });
                self.save_preferences(window, cx);
            }
            Err(failure) => {
                self.status = UpdateStatus::FailedCheck {
                    message: failure.message,
                };
                self.operation_error = true;
            }
        }
        cx.notify();
    }

    fn save_preferences(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Switcher>,
    ) {
        self.preferences_revision = self.preferences_revision.wrapping_add(1);
        if self.preferences_saving {
            cx.notify();
            return;
        }
        self.preferences_saving = true;
        let revision = self.preferences_revision;
        let task = cx
            .background_executor()
            .spawn(self.persist_preferences(self.preferences.clone()));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.updater.preferences_saving = false;
                this.updater.preferences_error = result.err();
                if this.updater.preferences_revision != revision {
                    this.updater.save_preferences(window, cx);
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Drop for UpdateController {
    fn drop(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
    }
}

impl Switcher {
    pub(crate) fn load_update_preferences(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
        schedule_startup_check: bool,
    ) {
        let load_task = cx
            .background_executor()
            .spawn(self.updater.load_preferences());
        let startup_check = schedule_startup_check && self.updater.startup_check();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = load_task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                match loaded {
                    Ok(preferences) => {
                        this.updater.preferences = preferences;
                        this.updater.preferences_error = None;
                    }
                    Err(error) => this.updater.preferences_error = Some(error),
                }
                this.updater.preferences_loaded = true;
                cx.notify();
            });
            if startup_check {
                cx.background_executor().timer(STARTUP_CHECK_DELAY).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.updater.auto_check_is_due(unix_seconds()) {
                        this.start_update_check(window, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub(crate) fn set_auto_check(
        &mut self,
        enabled: bool,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if self.updater.preferences_saving || !self.updater.preferences_loaded {
            return;
        }
        self.updater.preferences.auto_check = enabled;
        self.updater.preferences_error = None;
        self.updater.operation_notice = None;
        self.updater.operation_error = false;
        self.updater.save_preferences(window, cx);
        cx.notify();
    }

    pub(crate) fn start_update_check(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if self.updater.status_is_active() {
            return;
        }
        let Some(backend) = self.updater.backend.clone() else {
            self.updater.status = UpdateStatus::FailedCheck {
                message: self
                    .updater
                    .setup_error
                    .clone()
                    .unwrap_or_else(|| "软件更新服务暂不可用。".to_owned()),
            };
            cx.notify();
            return;
        };
        self.updater.status = UpdateStatus::Checking;
        self.updater.preferences_error = None;
        self.updater.operation_notice = None;
        self.updater.operation_error = false;
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { backend.check().await });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.updater
                    .apply_check_result(result, unix_seconds(), window, cx);
            });
        })
        .detach();
    }

    pub(crate) fn retry_update(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        match self.updater.status.clone() {
            UpdateStatus::FailedDownload { update, .. } => {
                self.start_update_download(update, window, cx)
            }
            UpdateStatus::FailedCheck { .. } | UpdateStatus::Idle => {
                self.start_update_check(window, cx)
            }
            UpdateStatus::Checking
            | UpdateStatus::Current { .. }
            | UpdateStatus::Available(_)
            | UpdateStatus::LegacyRelease { .. }
            | UpdateStatus::UnsupportedPlatform { .. }
            | UpdateStatus::Downloading { .. }
            | UpdateStatus::Cancelling { .. }
            | UpdateStatus::VerifyingInstall { .. }
            | UpdateStatus::Ready { .. } => {}
        }
    }

    pub(crate) fn start_update_download(
        &mut self,
        update: AvailableView,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if self.updater.status_is_active() {
            return;
        }
        let Some(backend) = self.updater.backend.clone() else {
            self.updater.status = UpdateStatus::FailedDownload {
                update,
                message: self
                    .updater
                    .setup_error
                    .clone()
                    .unwrap_or_else(|| "软件更新服务暂不可用。".to_owned()),
            };
            cx.notify();
            return;
        };
        let cancellation = CancellationToken::new();
        self.updater.cancellation = Some(cancellation.clone());
        self.updater.status = UpdateStatus::Downloading {
            update: update.clone(),
            progress: None,
        };
        let progress = Arc::new(Mutex::new(None));
        let progress_callback = {
            let progress = progress.clone();
            Arc::new(move |value| {
                *progress.lock().unwrap_or_else(|error| error.into_inner()) = Some(value);
            }) as ProgressCallback
        };
        let task = cx.background_executor().spawn(backend.download(
            update.package.asset.clone(),
            cancellation.clone(),
            progress_callback,
        ));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let mut download = Box::pin(task);
            loop {
                let timer = cx.background_executor().timer(PROGRESS_REFRESH_INTERVAL);
                match futures::future::select(download, Box::pin(timer)).await {
                    futures::future::Either::Left((result, _)) => {
                        let latest = progress
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .take();
                        let _ = this.update_in(cx, move |this, _, cx| {
                            if let Some(value) = latest {
                                this.updater.set_download_progress(value, cx);
                            }
                            this.updater
                                .finish_download(update, cancellation, result, cx);
                        });
                        break;
                    }
                    futures::future::Either::Right((_, pending)) => {
                        download = pending;
                        let latest = progress
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .take();
                        if let Some(value) = latest {
                            let _ = this.update_in(cx, move |this, _, cx| {
                                this.updater.set_download_progress(value, cx);
                            });
                        }
                    }
                }
            }
        })
        .detach();
    }

    pub(crate) fn cancel_update_download(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if let UpdateStatus::Downloading { update, progress } = self.updater.status.clone() {
            if let Some(cancellation) = &self.updater.cancellation {
                cancellation.cancel();
            }
            self.updater.status = UpdateStatus::Cancelling { update, progress };
            cx.notify();
        }
    }

    pub(crate) fn reveal_update_package(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        let UpdateStatus::Ready { update, path } = self.updater.status.clone() else {
            return;
        };
        let Some(cache_directory) = self.updater.cache_directory() else {
            self.updater.preferences_error = Some("无法定位更新缓存目录。".to_owned());
            cx.notify();
            return;
        };
        let expected_name = update.package.name.clone();
        let task = cx
            .background_executor()
            .spawn(async move { reveal_verified_package(&path, &expected_name, &cache_directory) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, move |this, _, cx| {
                this.updater.operation_notice = result.err();
                this.updater.operation_error = this.updater.operation_notice.is_some();
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn install_block_reason(&self, cx: &gpui_kit::Context<Self>) -> Option<String> {
        if self.busy {
            Some("配置操作正在进行。请等待操作完成后再启动安装程序。".to_owned())
        } else if self.prepared.is_some() {
            Some("当前有尚未应用的配置预览。请先应用或取消预览。".to_owned())
        } else if self.catalog.dirty {
            Some("模型目录有未保存改动。请先保存或放弃这些改动。".to_owned())
        } else if !self.key.read(cx).value().is_empty() {
            Some("API key 输入框中有未保存内容。请先保存或清空该输入。".to_owned())
        } else {
            None
        }
    }

    pub(crate) fn begin_update_install(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if let Some(reason) = self.install_block_reason(cx) {
            self.updater.operation_notice = Some(reason);
            self.updater.operation_error = false;
            cx.notify();
            return;
        }
        let UpdateStatus::Ready { update, path } = self.updater.status.clone() else {
            return;
        };
        let Some(backend) = self.updater.backend.clone() else {
            self.updater.operation_notice = Some(
                self.updater
                    .setup_error
                    .clone()
                    .unwrap_or_else(|| "软件更新服务暂不可用。".to_owned()),
            );
            self.updater.operation_error = true;
            cx.notify();
            return;
        };
        let Some(cache_directory) = self.updater.cache_directory() else {
            self.updater.operation_notice = Some("无法定位更新缓存目录。".to_owned());
            self.updater.operation_error = true;
            cx.notify();
            return;
        };

        let expected_name = update.package.name.clone();
        let package_kind = update_package_kind(&expected_name);
        let installer = self.updater.installer.clone();
        let asset = update.package.asset.clone();
        let fallback_path = path.clone();
        self.updater.status = UpdateStatus::VerifyingInstall {
            update: update.clone(),
        };
        self.busy = true;
        self.updater.operation_notice = None;
        self.updater.operation_error = false;
        let task = cx.background_executor().spawn(async move {
            let downloaded = backend
                .verify_download(asset, path)
                .await
                .map_err(|failure| InstallAttemptError::Verification(failure.message))?;
            let handoff = installer
                .handoff(
                    &downloaded.path,
                    &expected_name,
                    &cache_directory,
                    package_kind,
                )
                .map_err(InstallAttemptError::Handoff)?;
            Ok::<_, InstallAttemptError>((downloaded.path, handoff))
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, move |this, _, cx| {
                this.busy = false;
                match result {
                    Ok((verified_path, UpdateHandoff::SystemOpenerStarted)) => {
                        this.updater.status = UpdateStatus::Ready {
                            update: update.clone(),
                            path: verified_path,
                        };
                        this.updater.operation_notice = Some(match package_kind {
                            UpdatePackageKind::Dmg => format!(
                                "已打开 v{} 的 DMG，等待手动替换应用。",
                                update.release.version
                            ),
                            UpdatePackageKind::Installer => format!(
                                "已启动 v{} 安装程序。Codex 不会自动重启。",
                                update.release.version
                            ),
                            UpdatePackageKind::AppImage => {
                                "AppImage 应通过已校验文件所在目录处理。".to_owned()
                            }
                        });
                        this.updater.operation_error = false;
                    }
                    Ok((verified_path, UpdateHandoff::AppImageDirectoryOpened)) => {
                        this.updater.status = UpdateStatus::Ready {
                            update: update.clone(),
                            path: verified_path,
                        };
                        this.updater.operation_notice = Some(
                            "已打开 AppImage 所在目录。请手动替换文件；程序不会执行 AppImage。"
                                .to_owned(),
                        );
                        this.updater.operation_error = false;
                    }
                    Err(InstallAttemptError::Verification(error)) => {
                        this.updater.status = UpdateStatus::FailedDownload {
                            update,
                            message: error,
                        };
                        this.updater.operation_notice = None;
                        this.updater.operation_error = true;
                    }
                    Err(InstallAttemptError::Handoff(error)) => {
                        this.updater.status = UpdateStatus::Ready {
                            update,
                            path: fallback_path,
                        };
                        this.updater.operation_notice = Some(error);
                        this.updater.operation_error = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn open_update_release_notes(&self, cx: &mut gpui_kit::Context<Self>) {
        if let Some(url) = self.updater.release_url() {
            cx.open_url(url);
        }
    }
}

impl UpdateController {
    fn set_download_progress(
        &mut self,
        progress: DownloadProgress,
        cx: &mut gpui_kit::Context<Switcher>,
    ) {
        match &mut self.status {
            UpdateStatus::Downloading {
                progress: current, ..
            }
            | UpdateStatus::Cancelling {
                progress: current, ..
            } => *current = Some(progress),
            _ => {}
        }
        cx.notify();
    }

    fn finish_download(
        &mut self,
        update: AvailableView,
        cancellation: CancellationToken,
        result: Result<DownloadedUpdate, UpdateFailure>,
        cx: &mut gpui_kit::Context<Switcher>,
    ) {
        self.cancellation = None;
        match result {
            Err(failure) if cancellation.is_cancelled() && failure.cancelled => {
                self.status = UpdateStatus::Available(update);
                self.operation_error = failure.message != "下载已取消。";
                self.operation_notice = Some(if self.operation_error {
                    failure.message
                } else {
                    "下载已取消，未安装更新。".to_owned()
                });
            }
            result => match result {
                Ok(downloaded) => {
                    self.status = UpdateStatus::Ready {
                        update,
                        path: downloaded.path,
                    };
                    self.preferences_error = None;
                    self.operation_notice = None;
                    self.operation_error = false;
                }
                Err(failure) => {
                    self.status = UpdateStatus::FailedDownload {
                        update,
                        message: failure.message,
                    };
                    self.preferences_error = None;
                    self.operation_notice = None;
                    self.operation_error = true;
                }
            },
        }
        cx.notify();
    }
}

pub(crate) fn application_data_directory() -> Result<PathBuf, String> {
    if let Some(override_path) =
        std::env::var_os("CODEX_AUTH_SWITCHER_DATA_DIR").filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(override_path);
        if path.is_absolute() {
            return Ok(path);
        }
        return std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|_| "无法解析应用数据目录。".to_owned());
    }
    directories::BaseDirs::new()
        .map(|directories| directories.data_local_dir().join("Codex Auth Switcher"))
        .ok_or_else(|| "无法定位当前用户的应用数据目录。".to_owned())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn format_last_check(last: Option<&LastSuccessfulCheck>) -> String {
    let Some(last) = last else {
        return "尚无成功检查记录".to_owned();
    };
    let Ok(seconds) = i64::try_from(last.unix_seconds) else {
        return "时间记录无效".to_owned();
    };
    chrono::Local
        .timestamp_opt(seconds, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "时间记录无效".to_owned())
}

pub(crate) fn platform_name() -> &'static str {
    use switcher_core::update::{UpdatePlatform, current_platform};
    match current_platform() {
        Some(UpdatePlatform::WindowsX64) => "Windows x64 安装向导",
        Some(UpdatePlatform::LinuxAmd64Deb) => "Linux x86_64 Debian",
        Some(UpdatePlatform::LinuxX86_64AppImage) => "Linux x86_64 AppImage",
        Some(UpdatePlatform::MacosArm64) => "macOS ARM64",
        Some(UpdatePlatform::MacosX86_64) => "macOS x86_64",
        None => "当前平台未受支持",
    }
}

pub(crate) fn format_bytes(size: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if size >= MIB {
        format!("{:.1} MiB", size as f64 / MIB as f64)
    } else {
        format!("{:.1} KiB", size as f64 / 1024.)
    }
}

pub(crate) fn progress_percent(progress: Option<DownloadProgress>) -> f32 {
    progress
        .filter(|progress| progress.total_bytes > 0)
        .map(|progress| {
            (progress.received_bytes as f64 * 100. / progress.total_bytes as f64) as f32
        })
        .unwrap_or(0.)
        .clamp(0., 100.)
}

pub(crate) fn progress_summary(progress: Option<DownloadProgress>) -> String {
    progress
        .map(|progress| {
            format!(
                "{} / {}",
                format_bytes(progress.received_bytes),
                format_bytes(progress.total_bytes)
            )
        })
        .unwrap_or_else(|| "正在连接…".to_owned())
}

fn reveal_verified_package(
    path: &Path,
    expected_name: &str,
    cache_directory: &Path,
) -> Result<(), String> {
    let cache_directory =
        std::fs::canonicalize(cache_directory).map_err(|_| "更新缓存目录不可用。".to_owned())?;
    let package_path =
        std::fs::canonicalize(path).map_err(|_| "已校验的安装包已不存在。".to_owned())?;
    if !package_path.is_file()
        || package_path.file_name().and_then(|name| name.to_str()) != Some(expected_name)
        || !package_path.starts_with(&cache_directory)
    {
        return Err("安装包路径与已校验文件不一致。".to_owned());
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt as _;
        let mut command = Command::new("explorer.exe");
        command
            .arg(format!("/select,{}", package_path.display()))
            .creation_flags(0x0800_0000);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg("-R").arg(&package_path);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(
            package_path
                .parent()
                .ok_or_else(|| "无法定位安装包所在目录。".to_owned())?,
        );
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|_| "无法打开已校验安装包所在目录。".to_owned())
}

#[cfg(all(test, target_os = "windows"))]
mod installer_tests {
    use super::{SystemUpdateInstaller, UpdateHandoff, UpdateInstaller};
    use std::{env, path::PathBuf};

    #[test]
    fn windows_installer_starts_a_direct_executable_with_hidden_stdio() {
        let Some(system_root) = env::var_os("SystemRoot") else {
            return;
        };
        let executable = PathBuf::from(system_root)
            .join("System32")
            .join("where.exe");
        let Some(cache_directory) = executable.parent() else {
            return;
        };

        assert_eq!(
            SystemUpdateInstaller.handoff(
                &executable,
                "where.exe",
                cache_directory,
                super::UpdatePackageKind::Installer,
            ),
            Ok(UpdateHandoff::SystemOpenerStarted)
        );
    }
}
