use super::{
    DialogFuture, FileDialogs, MainTab, Switcher,
    update::{
        AvailableView, CheckResult, LastSuccessfulCheck, UpdateBackend, UpdateController,
        UpdateFailure, UpdateHandoff, UpdateInstaller, UpdatePackageKind, UpdatePreferences,
        UpdatePreferencesStore, UpdateStatus,
    },
};
use futures::channel::oneshot;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    point, px, size,
    test::{TestAppContextExt, TestWindowExt},
};
use std::{
    collections::VecDeque,
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use switcher_core::{
    Mode, Paths, Store,
    update::{CancellationToken, DownloadProgress, DownloadedUpdate, UpdateAsset},
};

type TestUpdateFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type DownloadResult = Result<DownloadedUpdate, UpdateFailure>;

enum DownloadBehavior {
    Immediate(DownloadResult),
    Pending(oneshot::Receiver<DownloadResult>),
}

#[derive(Default)]
struct TestUpdateBackend {
    checks: Mutex<VecDeque<Result<CheckResult, UpdateFailure>>>,
    pending_check: Mutex<Option<oneshot::Receiver<Result<CheckResult, UpdateFailure>>>>,
    downloads: Mutex<VecDeque<DownloadBehavior>>,
    verification_failures: Mutex<VecDeque<UpdateFailure>>,
    cancellation: Mutex<Option<CancellationToken>>,
    verify_calls: AtomicUsize,
}

impl TestUpdateBackend {
    fn new(
        checks: impl IntoIterator<Item = Result<CheckResult, UpdateFailure>>,
        downloads: impl IntoIterator<Item = DownloadBehavior>,
    ) -> Self {
        Self {
            checks: Mutex::new(checks.into_iter().collect()),
            pending_check: Mutex::new(None),
            downloads: Mutex::new(downloads.into_iter().collect()),
            verification_failures: Mutex::new(VecDeque::new()),
            cancellation: Mutex::new(None),
            verify_calls: AtomicUsize::new(0),
        }
    }
}

impl UpdateBackend for TestUpdateBackend {
    fn check(&self) -> TestUpdateFuture<Result<CheckResult, UpdateFailure>> {
        if let Some(receiver) = self
            .pending_check
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            return Box::pin(async move {
                receiver.await.unwrap_or_else(|_| {
                    Err(UpdateFailure {
                        message: "test update check response was dropped".to_owned(),
                        cancelled: false,
                    })
                })
            });
        }
        let result = self
            .checks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
            .unwrap_or_else(|| {
                Err(UpdateFailure {
                    message: "test update response queue is empty".to_owned(),
                    cancelled: false,
                })
            });
        Box::pin(async move { result })
    }

    fn download(
        &self,
        _asset: Option<UpdateAsset>,
        cancellation: CancellationToken,
        on_progress: Arc<dyn Fn(DownloadProgress) + Send + Sync + 'static>,
    ) -> TestUpdateFuture<DownloadResult> {
        *self
            .cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(cancellation);
        let behavior = self
            .downloads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
            .unwrap_or_else(|| {
                DownloadBehavior::Immediate(Err(UpdateFailure {
                    message: "test download response queue is empty".to_owned(),
                    cancelled: false,
                }))
            });
        Box::pin(async move {
            on_progress(DownloadProgress {
                received_bytes: 512,
                total_bytes: 1024,
            });
            match behavior {
                DownloadBehavior::Immediate(result) => result,
                DownloadBehavior::Pending(receiver) => receiver.await.unwrap_or_else(|_| {
                    Err(UpdateFailure {
                        message: "test download response was dropped".to_owned(),
                        cancelled: false,
                    })
                }),
            }
        })
    }

    fn verify_download(
        &self,
        _asset: Option<UpdateAsset>,
        path: PathBuf,
    ) -> TestUpdateFuture<Result<DownloadedUpdate, UpdateFailure>> {
        self.verify_calls.fetch_add(1, Ordering::SeqCst);
        let failure = self
            .verification_failures
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front();
        Box::pin(async move {
            if let Some(failure) = failure {
                return Err(failure);
            }
            Ok(DownloadedUpdate {
                path,
                from_cache: true,
            })
        })
    }
}

#[derive(Default)]
struct TestUpdateInstaller {
    calls: AtomicUsize,
    last_path: Mutex<Option<PathBuf>>,
    last_package_kind: Mutex<Option<UpdatePackageKind>>,
}

impl UpdateInstaller for TestUpdateInstaller {
    fn handoff(
        &self,
        path: &Path,
        _expected_name: &str,
        _cache_directory: &Path,
        package_kind: UpdatePackageKind,
    ) -> Result<UpdateHandoff, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self
            .last_path
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(path.to_path_buf());
        *self
            .last_package_kind
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(package_kind);
        Ok(match package_kind {
            UpdatePackageKind::Installer => UpdateHandoff::SystemOpenerStarted,
            UpdatePackageKind::AppImage => UpdateHandoff::AppImageDirectoryOpened,
            UpdatePackageKind::Dmg => UpdateHandoff::SystemOpenerStarted,
        })
    }
}

fn open(
    cx: &mut TestAppContext,
    home: PathBuf,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Entity<Switcher>) {
    let dialogs = test_dialogs(Arc::new(|| Box::pin(async { None }) as DialogFuture));
    open_with_dialogs(cx, home, width, height, dialogs)
}

fn test_dialogs(pick_model_catalog: Arc<dyn Fn() -> DialogFuture + Send + Sync>) -> FileDialogs {
    FileDialogs {
        pick_file: Arc::new(|_| Box::pin(async { None }) as DialogFuture),
        pick_folder: Arc::new(|| Box::pin(async { None }) as DialogFuture),
        pick_model_catalog,
    }
}

fn open_with_dialogs(
    cx: &mut TestAppContext,
    home: PathBuf,
    width: f32,
    height: f32,
    dialogs: FileDialogs,
) -> (AnyWindowHandle, Entity<Switcher>) {
    open_with_update_controller(
        cx,
        home,
        width,
        height,
        dialogs,
        UpdateController::disabled(),
    )
}

fn open_with_update_controller(
    cx: &mut TestAppContext,
    home: PathBuf,
    width: f32,
    height: f32,
    dialogs: FileDialogs,
    updater: UpdateController,
) -> (AnyWindowHandle, Entity<Switcher>) {
    let paths = Paths::resolve(Some(home))
        .unwrap_or_else(|error| panic!("cannot resolve test directory: {error}"));
    cx.update(gpui_kit::init);
    cx.update(|app| {
        gpui_kit::component::Theme::set_scrollbar_mode(
            gpui_kit::component::scroll::ScrollbarMode::Always,
            app,
        );
    });
    let (handle, content) = cx.update(|app| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(width), px(height)),
                })),
                ..Default::default()
            },
            app,
            |window, app| {
                app.new(|cx| {
                    Switcher::with_update_controller(
                        Store::isolated(paths),
                        dialogs,
                        updater,
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap_or_else(|error| panic!("cannot open GPUI test window: {error}"))
    });
    (handle, content)
}

async fn idle(cx: &mut TestAppContext, window: AnyWindowHandle, app: &Entity<Switcher>) {
    cx.wait_for(window, Duration::from_secs(3), |_, cx| !app.read(cx).busy)
        .await;
}

struct SetupExpectation {
    config_exists: bool,
    auth_exists: bool,
    action_label: &'static str,
    message: &'static str,
    status: &'static str,
    guidance: &'static str,
}

async fn assert_setup_state(cx: &mut TestAppContext, home: PathBuf, expected: SetupExpectation) {
    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .is_some_and(|status| status.config_exists)),
        expected.config_exists
    );
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .is_some_and(|status| status.auth_exists)),
        expected.auth_exists
    );
    assert!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .is_some_and(|status| !status.initialized)),
        "reading these file combinations should not create a mode archive"
    );
    assert_eq!(
        app.read_with(cx, |view, _| view.message.clone()),
        expected.message
    );
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let guidance = window.find("setup-guidance");
        assert!(guidance.visible());
        assert_eq!(guidance.label(), Some(expected.guidance));
        assert_eq!(window.find("setup-status").label(), Some(expected.status));
        let initialize = window.find("initialize");
        assert_eq!(initialize.label(), Some(expected.action_label));
    })
    .unwrap_or_else(|error| panic!("cannot inspect setup guidance and action: {error}"));
}

async fn update_idle(cx: &mut TestAppContext, window: AnyWindowHandle, app: &Entity<Switcher>) {
    cx.wait_for(window, Duration::from_secs(3), |_, cx| {
        !app.read(cx).updater.status_is_active() && !app.read(cx).updater.preferences_saving
    })
    .await;
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap_or_else(|error| panic!("cannot click {id}: {error}"));
}

fn replace_input(
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    id: &'static str,
    value: &str,
) {
    window.click(id, cx);
    window.press(
        if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        },
        cx,
    );
    if value.is_empty() {
        window.press("backspace", cx);
    } else {
        window.input(value, cx);
    }
}

fn assert_buttons_fit_without_overlap(
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    ids: &[&'static str],
) {
    window.render_frame(cx);
    let viewport = window.viewport_size();
    let bounds = ids
        .iter()
        .map(|id| {
            let button = window.find(*id);
            assert!(button.visible(), "{id} is outside the window");
            let bounds = button.bounds();
            assert!(
                bounds.left() >= px(0.)
                    && bounds.top() >= px(0.)
                    && bounds.right() <= viewport.width
                    && bounds.bottom() <= viewport.height,
                "{id} is outside the viewport: {bounds:?}"
            );
            bounds
        })
        .collect::<Vec<_>>();
    for (index, left) in bounds.iter().enumerate() {
        for right in bounds.iter().skip(index + 1) {
            let overlaps_x = left.left() < right.right() && left.right() > right.left();
            let overlaps_y = left.top() < right.bottom() && left.bottom() > right.top();
            assert!(
                !(overlaps_x && overlaps_y),
                "actions overlap: {left:?} {right:?}"
            );
        }
    }
}

fn write_model_catalog(path: &Path, slug: &str) {
    fs::write(
        path,
        format!(
            r#"{{"models":[{{"slug":"{slug}","display_name":"{slug}","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{{"effort":"high","description":"Synthetic high"}},{{"effort":"xhigh","description":"Synthetic xhigh"}},{{"effort":"max","description":"Synthetic max"}}],"priority":1}}]}}"#
        ),
    )
    .unwrap_or_else(|error| panic!("cannot write test model catalog: {error}"));
}

fn configure_catalog_path(config_path: &Path, catalog_path: &Path) {
    let config = fs::read_to_string(config_path)
        .unwrap_or_else(|error| panic!("cannot read test configuration: {error}"));
    let catalog_path = catalog_path.to_string_lossy().replace('\\', "/");
    fs::write(
        config_path,
        format!("model_catalog_json = \"{catalog_path}\"\n{config}"),
    )
    .unwrap_or_else(|error| panic!("cannot configure test model catalog: {error}"));
}

#[test]
fn automatic_check_repeats_for_available_updates_old_records_and_future_timestamps() {
    let version = env!("CARGO_PKG_VERSION").to_owned();
    let recent = 10_000;
    let preferences = |latest_version: Option<String>, unix_seconds| UpdatePreferences {
        auto_check: true,
        last_successful_check: Some(LastSuccessfulCheck {
            installed_version: version.clone(),
            unix_seconds,
            latest_version,
        }),
    };
    let controller = |preferences| {
        UpdateController::isolated(PathBuf::from("update-test-data"), None, preferences)
    };

    assert!(
        !controller(preferences(Some(version.clone()), recent)).auto_check_is_due(recent + 100)
    );
    assert!(
        controller(preferences(Some("99.0.0".to_owned()), recent)).auto_check_is_due(recent + 100)
    );
    assert!(
        controller(preferences(None, recent)).auto_check_is_due(recent + 100),
        "records written before latest_version was stored must be rechecked"
    );
    assert!(
        controller(preferences(Some(version.clone()), recent + 1_000)).auto_check_is_due(recent),
        "a future timestamp must not defer automatic checks"
    );

    let old_record: UpdatePreferences = serde_json::from_str(&format!(
        r#"{{"auto_check":true,"last_successful_check":{{"installed_version":"{version}","unix_seconds":{recent}}}}}"#
    ))
    .unwrap_or_else(|error| panic!("cannot read old update preferences: {error}"));
    assert_eq!(
        old_record
            .last_successful_check
            .as_ref()
            .and_then(|last| last.latest_version.as_deref()),
        None
    );

    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let store = UpdatePreferencesStore::at(temp.path().join("update-preferences.json"));
    let first_save = preferences(Some(version.clone()), recent);
    store
        .save(&first_save)
        .unwrap_or_else(|error| panic!("cannot save update preferences: {error}"));
    assert_eq!(
        store
            .load()
            .unwrap_or_else(|error| panic!("cannot load update preferences: {error}")),
        first_save
    );
    let second_save = UpdatePreferences::default();
    store
        .save(&second_save)
        .unwrap_or_else(|error| panic!("cannot replace update preferences: {error}"));
    assert_eq!(
        store
            .load()
            .unwrap_or_else(|error| panic!("cannot reload update preferences: {error}")),
        second_save
    );
}

#[gpui_kit::test]
async fn update_check_keeps_focus_and_long_notes_scroll_at_small_window(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let data_directory = temp.path().join("application-data");
    let (check_sender, check_receiver) = oneshot::channel();
    let notes = (0..40)
        .map(|index| format!("## 更新内容 {index}\n\n说明文字，保留版本更新的完整内容。"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let backend = Arc::new(TestUpdateBackend::new([], []));
    *backend
        .pending_check
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(check_receiver);
    let installer = Arc::new(TestUpdateInstaller::default());
    let updater = UpdateController::isolated_with_installer(
        data_directory,
        Some(backend),
        UpdatePreferences::default(),
        installer,
    );
    let (window, app) = open_with_update_controller(
        cx,
        home,
        720.,
        560.,
        test_dialogs(Arc::new(|| Box::pin(async { None }) as DialogFuture)),
        updater,
    );
    idle(cx, window, &app).await;
    click(cx, window, "update-tab");
    click(cx, window, "update-check");
    click(cx, window, "switch-tab");
    check_sender
        .send(Ok(CheckResult::Available(
            AvailableView::test_value_with_notes(notes),
        )))
        .unwrap_or_else(|_| panic!("update check stopped before test response"));
    update_idle(cx, window, &app).await;
    assert_eq!(app.read_with(cx, |view, _| view.tab), MainTab::Switch);
    assert!(app.read_with(cx, |view, _| view.updater.update_attention));

    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("update-tab").label(),
            Some("软件更新，有可用版本"),
            "a newly discovered update should be visible without changing tabs"
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect update attention label: {error}"));

    click(cx, window, "update-tab");
    assert!(!app.read_with(cx, |view, _| view.updater.update_attention));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let scroll = app.read(cx).update_scroll.clone();
        assert!(scroll.max_offset().y > px(0.));
        let before = scroll.offset().y;
        let bounds = scroll.bounds();
        window.drag(
            point(bounds.right() - px(8.), bounds.top() + px(12.)),
            point(bounds.right() - px(8.), bounds.bottom() - px(12.)),
            cx,
        );
        assert!(
            scroll.offset().y < before,
            "dragging the update scrollbar should move long release notes"
        );
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &["update-release-notes", "update-check-again"],
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect update page scrolling: {error}"));
}

#[gpui_kit::test]
async fn update_install_requires_confirmation_and_rechecks_the_cached_file(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let data_directory = temp.path().join("application-data");
    let mut update = AvailableView::test_value();
    update.package.name = "Codex-Auth-Switcher-0.4.0-macos-arm64.dmg".to_owned();
    let package_path = data_directory
        .join("update-cache")
        .join(&update.package.name);
    fs::create_dir_all(
        package_path
            .parent()
            .unwrap_or_else(|| panic!("package path has no parent")),
    )
    .unwrap_or_else(|error| panic!("cannot create update cache: {error}"));
    fs::write(&package_path, b"synthetic verified package")
        .unwrap_or_else(|error| panic!("cannot write update package: {error}"));

    let backend = Arc::new(TestUpdateBackend::new(
        [Ok(CheckResult::Available(update))],
        [DownloadBehavior::Immediate(Ok(DownloadedUpdate {
            path: package_path.clone(),
            from_cache: false,
        }))],
    ));
    let installer = Arc::new(TestUpdateInstaller::default());
    let updater = UpdateController::isolated_with_installer(
        data_directory,
        Some(backend.clone()),
        UpdatePreferences::default(),
        installer.clone(),
    );
    let (window, app) = open_with_update_controller(
        cx,
        home,
        720.,
        560.,
        test_dialogs(Arc::new(|| Box::pin(async { None }) as DialogFuture)),
        updater,
    );
    idle(cx, window, &app).await;
    click(cx, window, "update-tab");
    click(cx, window, "update-check");
    update_idle(cx, window, &app).await;
    click(cx, window, "update-download");
    update_idle(cx, window, &app).await;

    assert!(matches!(
        app.read_with(cx, |view, _| view.updater.status.clone()),
        UpdateStatus::Ready { .. }
    ));
    cx.update_window(window, |_, window, cx| {
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &[
                "update-release-notes",
                "update-install",
                "update-open-package",
                "update-check-again",
            ],
        );
        let scroll = app.read(cx).update_scroll.clone();
        scroll.scroll_to_top_of_item(6);
        window.render_frame(cx);
        let hash = window.find("update-package-sha256");
        assert!(
            hash.visible(),
            "the SHA-256 label should be visible after scrolling to the package summary"
        );
        assert!(
            hash.bounds().left() >= px(0.) && hash.bounds().right() <= window.viewport_size().width,
            "the SHA-256 label should stay within the window"
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect update action layout: {error}"));
    cx.update_window(window, |_, window, _cx| {
        assert_eq!(
            window.find("update-install").label(),
            Some("重新校验并打开 DMG，以手动替换应用程序")
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect the DMG update action: {error}"));

    click(cx, window, "update-install");
    assert_eq!(installer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.verify_calls.load(Ordering::SeqCst), 0);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("confirm-update-install").visible());
        window.press("escape", cx);
    })
    .unwrap_or_else(|error| panic!("cannot cancel update confirmation: {error}"));
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("dialog").is_none()
    })
    .await;
    assert_eq!(installer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.verify_calls.load(Ordering::SeqCst), 0);

    click(cx, window, "update-install");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("confirm-update-install").is_some()
    })
    .await;
    assert_eq!(
        {
            let mut label = None;
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                label = window
                    .find("confirm-update-install")
                    .label()
                    .map(str::to_owned);
            })
            .unwrap_or_else(|error| panic!("cannot inspect DMG confirmation: {error}"));
            label
        },
        Some("打开 DMG".to_owned())
    );
    click(cx, window, "confirm-update-install");
    update_idle(cx, window, &app).await;
    assert_eq!(backend.verify_calls.load(Ordering::SeqCst), 1);
    assert_eq!(installer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *installer
            .last_path
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
        Some(package_path)
    );
    assert_eq!(
        *installer
            .last_package_kind
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
        Some(UpdatePackageKind::Dmg)
    );
    assert!(app.read_with(cx, |view, _| {
        view.updater.message().contains("已打开")
            && view.updater.message().contains("DMG")
            && view.updater.message().contains("手动替换")
            && !view.updater.message().contains("已启动")
    }));

    backend
        .verification_failures
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push_back(UpdateFailure {
            message: format!(
                "安装包 SHA-256 校验失败。请重新下载后再试。{}",
                "更新服务器连接失败，请检查网络后重试。".repeat(4)
            ),
            cancelled: false,
        });
    click(cx, window, "update-install");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("confirm-update-install").is_some()
    })
    .await;
    click(cx, window, "confirm-update-install");
    update_idle(cx, window, &app).await;
    assert_eq!(backend.verify_calls.load(Ordering::SeqCst), 2);
    assert_eq!(installer.calls.load(Ordering::SeqCst), 1);
    assert!(app.read_with(cx, |view, _| {
        matches!(
            &view.updater.status,
            UpdateStatus::FailedDownload { message, .. }
                if message.contains("请重新下载") && !message.contains("已清理")
        )
    }));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let message = window.find("update-message");
        assert!(
            message.bounds().size.height > px(20.),
            "a long update error should wrap to more than one line"
        );
        assert!(message.bounds().right() <= window.viewport_size().width);
    })
    .unwrap_or_else(|error| panic!("cannot inspect wrapped update error: {error}"));
}

#[gpui_kit::test]
async fn update_install_preserves_drafts_and_cancel_cleanup_errors(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let data_directory = temp.path().join("application-data");
    let update = AvailableView::test_value();
    let (download_cancel, download_pending) = oneshot::channel();
    let backend = Arc::new(TestUpdateBackend::new(
        [],
        [DownloadBehavior::Pending(download_pending)],
    ));
    let installer = Arc::new(TestUpdateInstaller::default());
    let mut updater = UpdateController::isolated_with_installer(
        data_directory.clone(),
        Some(backend.clone()),
        UpdatePreferences::default(),
        installer.clone(),
    );
    updater.status = UpdateStatus::Ready {
        update: update.clone(),
        path: data_directory
            .join("update-cache")
            .join(&update.package.name),
    };
    let (window, app) = open_with_update_controller(
        cx,
        home,
        720.,
        560.,
        test_dialogs(Arc::new(|| Box::pin(async { None }) as DialogFuture)),
        updater,
    );
    idle(cx, window, &app).await;
    click(cx, window, "update-tab");

    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value("synthetic-unsaved-update-key", window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot set synthetic API key: {error}"));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("update-install").disabled(), Some(true));
    })
    .unwrap_or_else(|error| panic!("cannot inspect update draft guard: {error}"));
    assert!(app.read_with(cx, |view, cx| {
        view.key.read(cx).value() == "synthetic-unsaved-update-key"
    }));
    app.update(cx, |this, cx| {
        let message = this
            .install_block_reason(cx)
            .unwrap_or_else(|| panic!("API key draft did not block update installation"));
        assert!(message.contains("API key"));
    });
    assert_eq!(installer.calls.load(Ordering::SeqCst), 0);

    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot clear synthetic API key: {error}"));
    app.update(cx, |this, cx| {
        this.catalog.dirty = true;
        let message = this
            .install_block_reason(cx)
            .unwrap_or_else(|| panic!("dirty model catalog did not block update installation"));
        assert!(message.contains("模型目录"));
    });
    assert_eq!(installer.calls.load(Ordering::SeqCst), 0);

    app.update(cx, |this, _| {
        this.catalog.dirty = false;
        this.updater.status = UpdateStatus::Available(update);
    });
    click(cx, window, "update-download");
    click(cx, window, "update-cancel");
    assert!(
        backend
            .cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    );
    download_cancel
        .send(Err(UpdateFailure {
            message: "取消下载后清理缓存失败。".to_owned(),
            cancelled: true,
        }))
        .unwrap_or_else(|_| panic!("download task stopped before cleanup error was delivered"));
    update_idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.updater.message().contains("取消下载后清理缓存失败") && view.updater.operation_error
    }));
    assert_eq!(installer.calls.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
async fn switches_modes_and_clears_the_api_input(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home.clone(), 1200., 1100.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;

    let old_key = app.read_with(cx, |view, _| view.key.entity_id());
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value("synthetic-gpui-input", window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot edit test input: {error}"));
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.prepared
            .as_ref()
            .is_some_and(|item| item.preview.changed)
    }));
    click(cx, window, "apply");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .and_then(|status| status.mode)),
        Some(Mode::Api)
    );
    assert!(app.read_with(cx, |view, cx| view.key.read(cx).value().is_empty()));
    assert_ne!(old_key, app.read_with(cx, |view, _| view.key.entity_id()));

    click(cx, window, "switch-tab");
    click(cx, window, "subscription");
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    click(cx, window, "apply");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .and_then(|status| status.mode)),
        Some(Mode::Subscription)
    );
    let config = fs::read_to_string(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    assert!(config.contains("[projects.\"F:/example\"]"));
    assert!(config.contains("[features]"));
}

#[gpui_kit::test]
async fn failed_apply_preserves_the_api_key_draft(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home.clone(), 1200., 1100.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;

    let api_key = "synthetic-stale-preview-key";
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "api-key-input", api_key);
    })
    .unwrap_or_else(|error| panic!("cannot edit test API key: {error}"));
    assert_eq!(
        app.read_with(cx, |view, cx| view.key.read(cx).value().to_string()),
        api_key
    );
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));

    let config_path = home.join("config.toml");
    let mut external_config = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("cannot read config: {error}"));
    external_config.push_str("\n# external edit after preview\n");
    fs::write(&config_path, &external_config)
        .unwrap_or_else(|error| panic!("cannot simulate external config edit: {error}"));

    click(cx, window, "apply");
    idle(cx, window, &app).await;

    assert_eq!(
        app.read_with(cx, |view, cx| view.key.read(cx).value().to_string()),
        api_key,
        "a failed apply must preserve the API key draft"
    );
    assert!(app.read_with(cx, |view, _| view.prepared.is_none()));
    assert!(app.read_with(cx, |view, _| view.message.contains("预览后文件已发生变化")));
    assert_eq!(
        fs::read_to_string(&config_path)
            .unwrap_or_else(|error| panic!("cannot read config after failed apply: {error}")),
        external_config
    );
}

#[gpui_kit::test]
async fn first_run_creates_defaults_without_fake_authentication(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    let (window, app) = open(cx, home.clone(), 1040., 740.);
    idle(cx, window, &app).await;
    assert!(!home.join("config.toml").exists());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let initialize = window.find("initialize");
        assert!(initialize.visible());
        assert_eq!(initialize.label(), Some("初始化"));
        assert!(window.find("setup-guidance").visible());
    })
    .unwrap_or_else(|error| panic!("cannot inspect first-run action: {error}"));
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    let config = fs::read_to_string(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    assert!(config.contains("cli_auth_credentials_store"));
    assert!(!config.contains("gateway"));
    assert!(!home.join("auth.json").exists());
    assert_eq!(
        app.read_with(cx, |view, cx| view.endpoint.read(cx).value().to_string()),
        "https://api.openai.com/v1"
    );
}

#[gpui_kit::test]
async fn setup_guidance_matches_config_and_auth_file_presence(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));

    let neither = temp.path().join("neither");
    assert_setup_state(
        cx,
        neither.clone(),
        SetupExpectation {
            config_exists: false,
            auth_exists: false,
            action_label: "初始化",
            message: "未找到 config.toml 和 auth.json。请先初始化配置。",
            status: "首次使用",
            guidance:
                "未找到 config.toml 和 auth.json。点击“初始化”创建默认配置，再登录或输入 API key。",
        },
    )
    .await;
    assert!(!neither.join("config.toml").exists());
    assert!(!neither.join("auth.json").exists());
    assert_setup_state(
        cx,
        {
            let home = temp.path().join("config-only");
            crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
            fs::remove_file(home.join("auth.json"))
                .unwrap_or_else(|error| panic!("cannot remove synthetic auth for test: {error}"));
            home
        },
        SetupExpectation {
            config_exists: true,
            auth_exists: false,
            action_label: "保存当前模式",
            message: "已读取配置，缺少 auth.json。请登录 Codex、导入认证或输入 API key。",
            status: "待补齐",
            guidance: "已有配置，缺少 auth.json。请在 Codex 登录、导入认证，或在下方输入 API key。",
        },
    )
    .await;
    assert_setup_state(
        cx,
        {
            let home = temp.path().join("auth-only");
            fs::create_dir_all(&home)
                .unwrap_or_else(|error| panic!("cannot create auth-only test home: {error}"));
            fs::write(
                home.join("auth.json"),
                br#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access","refresh_token":"synthetic-refresh","id_token":"synthetic-id","account_id":"synthetic-account"}}"#,
            )
                .unwrap_or_else(|error| panic!("cannot write config-only test config: {error}"));
            home
        },
        SetupExpectation {
            config_exists: false,
            auth_exists: true,
            action_label: "补齐配置",
            message: "已找到 auth.json，缺少 config.toml。请补齐配置。",
            status: "待补齐",
            guidance: "已找到认证，缺少 config.toml。点击“补齐配置”；现有认证保持不变。",
        },
    )
    .await;
    assert_setup_state(
        cx,
        {
            let home = temp.path().join("both");
            crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
            home
        },
        SetupExpectation {
            config_exists: true,
            auth_exists: true,
            action_label: "保存当前模式",
            message: "已读取配置和认证。请选择目标模式，再预览变更。",
            status: "已就绪",
            guidance: "配置和认证已存在。选择目标模式后点击“预览变更”，检查无误再应用。",
        },
    )
    .await;
}

#[gpui_kit::test]
async fn configuration_check_is_read_only_and_confirmed_cleanup_preserves_other_data(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    let config_before = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("cannot read synthetic test config: {error}"));
    let configured = config_before.replace(
        "[features]\nmulti_agent = true",
        "personality = \"friendly\"\nunknown_option = \"preserve-me\"\n\
         [features]\nmulti_agent = true\npersonality = true\nunified_exec = false",
    );
    fs::write(&config_path, configured)
        .unwrap_or_else(|error| panic!("cannot write synthetic test config: {error}"));
    let config_before_check = fs::read(&config_path)
        .unwrap_or_else(|error| panic!("cannot snapshot config before check: {error}"));
    let auth_before = fs::read(&auth_path)
        .unwrap_or_else(|error| panic!("cannot snapshot synthetic auth: {error}"));

    let (window, app) = open(cx, home.clone(), 1040., 740.);
    idle(cx, window, &app).await;
    assert!(!home.join("auth-switcher-v2").join("modes.enc").exists());
    click(cx, window, "config-tab");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("config-check-empty").visible());
        assert_ne!(window.find("config-check").disabled(), Some(true));
    })
    .unwrap_or_else(|error| panic!("cannot inspect configuration check page: {error}"));
    click(cx, window, "config-check");
    idle(cx, window, &app).await;

    assert_eq!(
        fs::read(&config_path)
            .unwrap_or_else(|error| panic!("cannot read config after check: {error}")),
        config_before_check,
        "checking config.toml must not modify it"
    );
    assert_eq!(
        fs::read(&auth_path)
            .unwrap_or_else(|error| panic!("cannot read auth after check: {error}")),
        auth_before,
        "checking config.toml must not modify auth.json"
    );
    assert!(
        app.read_with(cx, |view, _| view.health_report.as_ref().is_some_and(
            |report| {
                report
                    .issues
                    .iter()
                    .any(|issue| issue.key == "personality" && issue.cleanup)
                    && report
                        .issues
                        .iter()
                        .any(|issue| issue.key == "features.unified_exec" && issue.cleanup)
            }
        ))
    );
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("config-check-report").visible());
        assert!(window.find("config-issue-personality").visible());
        assert!(window.find("config-issue-features.unified_exec").visible());
        assert_ne!(window.find("config-clean").disabled(), Some(true));
    })
    .unwrap_or_else(|error| panic!("cannot inspect configuration findings: {error}"));

    click(cx, window, "config-clean");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("config-clean-confirm").is_some()
            && window.try_find("config-clean-cancel").is_some()
    })
    .await;
    click(cx, window, "config-clean-cancel");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("config-clean-confirm").is_none()
    })
    .await;
    assert_eq!(
        fs::read(&config_path)
            .unwrap_or_else(|error| panic!("cannot read config after cancel: {error}")),
        config_before_check,
        "canceling cleanup must leave config.toml unchanged"
    );
    assert_eq!(
        fs::read(&auth_path)
            .unwrap_or_else(|error| panic!("cannot read auth after cancel: {error}")),
        auth_before
    );
    assert!(app.read_with(cx, |view, _| view.health_report.is_some()));

    click(cx, window, "config-clean");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("config-clean-confirm").is_some()
    })
    .await;
    click(cx, window, "config-clean-confirm");
    idle(cx, window, &app).await;
    assert!(
        app.read_with(cx, |view, _| view
            .health_report
            .as_ref()
            .is_some_and(|report| !report.can_cleanup())),
        "successful cleanup should refresh the report and remove cleanable findings"
    );

    let cleaned = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("cannot read cleaned test config: {error}"));
    assert!(!cleaned.contains("personality = \"friendly\""));
    assert!(!cleaned.contains("personality = true"));
    assert!(!cleaned.contains("unified_exec = false"));
    assert!(cleaned.contains("model_provider = \"openai\""));
    assert!(cleaned.contains("multi_agent = true"));
    assert!(cleaned.contains("unknown_option = \"preserve-me\""));
    assert!(cleaned.contains("[projects.\"F:/example\"]"));
    assert_eq!(
        fs::read(&auth_path)
            .unwrap_or_else(|error| panic!("cannot read auth after cleanup: {error}")),
        auth_before
    );
    assert!(!home.join("auth-switcher-v2").join("modes.enc").exists());

    let backups = fs::read_dir(&home)
        .unwrap_or_else(|error| panic!("cannot list config backup directory: {error}"))
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("config.toml.bak-cleanup-")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    assert_eq!(
        backups.len(),
        1,
        "confirmed cleanup should create one backup"
    );
    let backup = fs::read_to_string(&backups[0])
        .unwrap_or_else(|error| panic!("cannot read cleanup patch backup: {error}"));
    assert!(backup.contains("personality = \"friendly\""));
    assert!(backup.contains("personality = true"));
    assert!(backup.contains("unified_exec = false"));
    assert!(!backup.contains("multi_agent"));
    assert!(!backup.contains("unknown_option"));
    assert!(!backup.contains("projects"));
    assert!(!backup.contains("synthetic-access"));

    click(cx, window, "config-check");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.health_report
            .as_ref()
            .is_some_and(|report| !report.can_cleanup())
    }));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("config-clean").disabled(), Some(true));
    })
    .unwrap_or_else(|error| panic!("cannot inspect rechecked configuration: {error}"));
}

#[gpui_kit::test]
async fn configuration_cleanup_rejects_stale_check_without_overwriting_external_changes(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let config_path = home.join("config.toml");
    let original = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("cannot read synthetic test config: {error}"));
    let cleanup_target = original.replace(
        "[features]\nmulti_agent = true",
        "personality = \"friendly\"\n[features]\nmulti_agent = true",
    );
    fs::write(&config_path, cleanup_target)
        .unwrap_or_else(|error| panic!("cannot write synthetic cleanup target: {error}"));
    let external_change = format!(
        "{}\nexternal_change_marker = \"preserve-external-edit\"\n",
        fs::read_to_string(&config_path)
            .unwrap_or_else(|error| panic!("cannot read cleanup target: {error}"))
    );
    let auth_before = fs::read(home.join("auth.json"))
        .unwrap_or_else(|error| panic!("cannot snapshot synthetic auth: {error}"));

    let (window, app) = open(cx, home.clone(), 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "config-tab");
    click(cx, window, "config-check");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.health_report
            .as_ref()
            .is_some_and(|report| report.can_cleanup())
    }));

    fs::write(&config_path, external_change)
        .unwrap_or_else(|error| panic!("cannot simulate an external config edit: {error}"));
    let external_bytes = fs::read(&config_path)
        .unwrap_or_else(|error| panic!("cannot snapshot external config edit: {error}"));
    click(cx, window, "config-clean");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("config-clean-confirm").is_some()
    })
    .await;
    click(cx, window, "config-clean-confirm");
    idle(cx, window, &app).await;

    assert!(app.read_with(cx, |view, _| view.error));
    assert_eq!(
        fs::read(&config_path)
            .unwrap_or_else(|error| panic!("cannot read config after stale cleanup: {error}")),
        external_bytes
    );
    assert!(String::from_utf8_lossy(&external_bytes).contains("preserve-external-edit"));
    assert_eq!(
        fs::read(home.join("auth.json"))
            .unwrap_or_else(|error| panic!("cannot read auth after stale cleanup: {error}")),
        auth_before
    );
    assert!(
        !fs::read_dir(&home)
            .unwrap_or_else(|error| panic!("cannot list home after stale cleanup: {error}"))
            .filter_map(std::result::Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config.toml.bak-cleanup-")
            }),
        "stale cleanup must fail before creating a patch backup"
    );
}

#[gpui_kit::test]
async fn changing_directory_clears_unsaved_key_but_refreshing_same_directory_keeps_it(
    cx: &mut TestAppContext,
) {
    let temporary = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let first_home = temporary.path().join("first-codex-home");
    let second_home = temporary.path().join("second-codex-home");
    let (window, app) = open(cx, first_home, 1040., 740.);
    idle(cx, window, &app).await;

    let initial_key = "synthetic-key-for-first-directory";
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value(initial_key, window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot set test API key: {error}"));
    let original_key_entity = app.read_with(cx, |view, _| view.key.entity_id());

    click(cx, window, "refresh");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, cx| view.key.read(cx).value().to_string()),
        initial_key,
        "refreshing the same directory must retain the unsaved key"
    );
    assert_eq!(
        app.read_with(cx, |view, _| view.key.entity_id()),
        original_key_entity,
        "refreshing the same directory must retain the input history"
    );

    cx.update_window(window, |_, window, cx| {
        let directory = app.read(cx).directory.clone();
        directory.update(cx, |state, cx| {
            state.set_value(second_home.to_string_lossy(), window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot set test Codex directory: {error}"));
    click(cx, window, "refresh");
    idle(cx, window, &app).await;

    assert!(app.read_with(cx, |view, cx| view.key.read(cx).value().is_empty()));
    assert_ne!(
        app.read_with(cx, |view, _| view.key.entity_id()),
        original_key_entity,
        "changing directories must replace the key input and revoke its edit history"
    );
}

#[gpui_kit::test]
async fn changing_directory_resets_provider_and_endpoint_without_profiles(cx: &mut TestAppContext) {
    let temporary = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let first_home = temporary.path().join("first-codex-home");
    let second_home = temporary.path().join("second-codex-home");
    crate::demo::create(&first_home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, first_home, 1040., 740.);
    idle(cx, window, &app).await;

    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .api_provider
            .read(cx)
            .value()
            .to_string()),
        "OpenAI"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view.endpoint.read(cx).value().to_string()),
        "https://gateway.example.invalid/v1"
    );

    cx.update_window(window, |_, window, cx| {
        let directory = app.read(cx).directory.clone();
        directory.update(cx, |state, cx| {
            state.set_value(second_home.to_string_lossy(), window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot set test Codex directory: {error}"));
    click(cx, window, "refresh");
    idle(cx, window, &app).await;

    assert!(app.read_with(cx, |view, _| {
        view.status
            .as_ref()
            .is_some_and(|status| status.modes.is_empty())
    }));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .api_provider
            .read(cx)
            .value()
            .to_string()),
        "openai"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view.endpoint.read(cx).value().to_string()),
        "https://api.openai.com/v1"
    );
}

#[gpui_kit::test]
async fn controls_stay_visible_and_picker_cancellation_is_safe(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home.clone(), 720., 560.);
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        assert_buttons_fit_without_overlap(window, cx, &["preview", "restart"]);
        assert!(
            window.try_find("apply").is_none(),
            "apply action must be shown only after review"
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect test window: {error}"));

    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &["back-to-settings", "cancel-preview", "apply"],
        );
        assert!(
            window.try_find("restart").is_none(),
            "restart action should not compete with review confirmation"
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect review actions: {error}"));
    click(cx, window, "cancel-preview");
    assert_eq!(app.read_with(cx, |view, _| view.tab), MainTab::Switch);

    click(cx, window, "models-tab");
    cx.update_window(window, |_, window, cx| {
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &["catalog-save", "catalog-discard", "restart"],
        );
        for id in ["preview", "apply"] {
            assert!(
                window.try_find(id).is_none(),
                "{id} should not appear on the model page"
            );
        }
    })
    .unwrap_or_else(|error| panic!("cannot inspect model page actions: {error}"));
    click(cx, window, "switch-tab");

    let before = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    click(cx, window, "browse");
    idle(cx, window, &app).await;
    let after = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    assert_eq!(before, after);
    assert!(app.read_with(cx, |view, _| view.message.contains("取消")));
}

#[gpui_kit::test]
async fn scrollbars_reach_overflowing_switch_and_model_content(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    let models = (0..48)
        .map(|index| {
            format!(
                r#"{{"slug":"model-{index}","display_name":"Model {index}","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{{"effort":"high","description":"Synthetic high"}}],"priority":{index}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        &catalog_path,
        format!(r#"{{"catalog_version":3,"models":[{models}]}}"#),
    )
    .unwrap_or_else(|error| panic!("cannot write test model catalog: {error}"));
    let catalog_path = catalog_path.to_string_lossy().replace('\\', "/");
    let config_path = home.join("config.toml");
    let config = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    fs::write(
        &config_path,
        format!("model_catalog_json = \"{catalog_path}\"\n{config}"),
    )
    .unwrap_or_else(|error| panic!("cannot configure test model catalog: {error}"));

    let (window, app) = open(cx, home, 720., 560.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let scroll = app.read(cx).switch_scroll.clone();
        assert!(
            scroll.max_offset().y > px(0.),
            "switch page content does not overflow"
        );
        let bounds = scroll.bounds();
        let start = point(bounds.right() - px(8.), bounds.top() + px(20.));
        let end = point(bounds.right() - px(8.), bounds.bottom() - px(20.));
        let offset_before = scroll.offset().y;
        window.drag(start, end, cx);
        assert!(
            scroll.offset().y < offset_before,
            "dragging the switch page scrollbar did not move its content"
        );
        assert_buttons_fit_without_overlap(window, cx, &["preview", "restart"]);
        assert!(window.try_find("apply").is_none());
    })
    .unwrap_or_else(|error| panic!("cannot inspect switch page scrolling: {error}"));
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value("synthetic-scroll-test-key", window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot seed preview input: {error}"));
    click(cx, window, "preview");
    idle(cx, window, &app).await;

    cx.update_window(window, |_, window, cx| {
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &["back-to-settings", "cancel-preview", "apply"],
        );
        assert_eq!(app.read(cx).tab, MainTab::Review);
        assert!(window.try_find("restart").is_none());
    })
    .unwrap_or_else(|error| panic!("cannot inspect review actions: {error}"));

    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &["catalog-save", "catalog-discard", "restart"],
        );
        assert!(
            window.try_find("preview").is_none() && window.try_find("apply").is_none(),
            "mode-switch actions must not appear on the model page"
        );
        let model_list_scroll = app.read(cx).model_list_scroll.clone();
        assert!(
            model_list_scroll.max_offset().y > px(0.),
            "model list content does not overflow"
        );
        let catalog_scroll = app.read(cx).catalog_scroll.clone();
        let list_bounds = model_list_scroll.bounds();
        catalog_scroll.set_offset(point(
            px(0.),
            catalog_scroll.offset().y + catalog_scroll.bounds().top() - list_bounds.top(),
        ));
        window.render_frame(cx);
        let list_bounds = model_list_scroll.bounds();
        assert!(list_bounds.top() >= catalog_scroll.bounds().top());
        assert!(list_bounds.bottom() <= catalog_scroll.bounds().bottom());
        assert!(list_bounds.size.height <= px(170.));
        let start = point(list_bounds.right() - px(8.), list_bounds.top() + px(20.));
        let end = point(list_bounds.right() - px(8.), list_bounds.bottom() - px(20.));
        let list_offset_before = model_list_scroll.offset().y;
        window.drag(start, end, cx);
        assert!(
            model_list_scroll.offset().y < list_offset_before,
            "dragging the model list scrollbar did not move the list"
        );
        catalog_scroll.set_offset(point(px(0.), px(0.)));
        window.render_frame(cx);
        assert!(
            catalog_scroll.max_offset().y > px(0.),
            "model catalog page content does not overflow"
        );
        let catalog_bounds = catalog_scroll.bounds();
        let start = point(
            catalog_bounds.right() - px(8.),
            catalog_bounds.top() + px(20.),
        );
        let end = point(
            catalog_bounds.right() - px(8.),
            catalog_bounds.bottom() - px(20.),
        );
        let catalog_offset_before = catalog_scroll.offset().y;
        window.drag(start, end, cx);
        assert!(
            catalog_scroll.offset().y < catalog_offset_before,
            "dragging the model catalog page scrollbar did not move its content"
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect model list scrolling: {error}"));
}

#[gpui_kit::test]
async fn narrow_model_list_height_tracks_small_catalogs(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    let models = (0..2)
        .map(|index| {
            format!(
                r#"{{"slug":"small-model-{index}","display_name":"Small Model {index}","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{{"effort":"high","description":"Synthetic high"}}],"priority":{index}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        &catalog_path,
        format!(r#"{{"catalog_version":3,"models":[{models}]}}"#),
    )
    .unwrap_or_else(|error| panic!("cannot write small model catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home, 720., 560.);
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let scroll = app.read(cx).model_list_scroll.clone();
        assert_eq!(
            scroll.bounds().size.height,
            px(64.),
            "two rows should use two 32px slots"
        );
        assert_eq!(scroll.max_offset().y, px(0.));
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &["catalog-save", "catalog-discard", "restart"],
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect compact model list: {error}"));
}

#[gpui_kit::test]
async fn api_key_can_be_cleared_after_target_changes_to_subscription(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;

    let original_key_entity = app.read_with(cx, |view, _| view.key.entity_id());
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value("synthetic-key-to-clear", window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot seed API key input: {error}"));
    click(cx, window, "subscription");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("clear-api-key").visible());
        assert_ne!(window.find("clear-api-key").disabled(), Some(true));

        let scroll = app.read(cx).switch_scroll.clone();
        let scroll_bounds = scroll.bounds();
        let clear_bounds = window.find("clear-api-key").bounds();
        let scroll_offset = scroll.offset();
        let scroll_y = (clear_bounds.bottom() - scroll_bounds.bottom()).max(px(0.));
        if scroll_y > px(0.) {
            scroll.set_offset(point(scroll_offset.x, scroll_offset.y - scroll_y));
        }
        window.render_frame(cx);

        let clear_bounds = window.find("clear-api-key").bounds();
        let clear_center = clear_bounds.center();
        let scroll_bounds = scroll.bounds();
        assert!(
            clear_center.x >= scroll_bounds.left()
                && clear_center.x < scroll_bounds.right()
                && clear_center.y >= scroll_bounds.top()
                && clear_center.y < scroll_bounds.bottom(),
            "clear API key button center must be inside the switch page viewport: button={clear_bounds:?}, viewport={scroll_bounds:?}"
        );
        assert!(window.find("clear-api-key").visible());
        window.click("clear-api-key", cx);
    })
    .unwrap_or_else(|error| panic!("cannot clear API key from subscription target: {error}"));

    assert!(app.read_with(cx, |view, cx| view.key.read(cx).value().is_empty()));
    assert_ne!(
        app.read_with(cx, |view, _| view.key.entity_id()),
        original_key_entity,
        "clearing the API key must replace the input entity and its edit history"
    );
}

#[gpui_kit::test]
async fn preview_review_can_return_or_cancel_without_losing_input_or_writing_files(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home.clone(), 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;

    let api_key = "synthetic-preview-cancel-key";
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| state.set_value(api_key, window, cx));
    })
    .unwrap_or_else(|error| panic!("cannot seed test API key: {error}"));
    let config_before = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    let auth_before = fs::read(home.join("auth.json"))
        .unwrap_or_else(|error| panic!("cannot read test authentication: {error}"));
    let vault_before = fs::read(home.join("auth-switcher-v2/modes.enc"))
        .unwrap_or_else(|error| panic!("cannot read test mode archive: {error}"));

    click(cx, window, "preview");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        assert_buttons_fit_without_overlap(
            window,
            cx,
            &["back-to-settings", "cancel-preview", "apply"],
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect 1040px review layout: {error}"));
    assert_eq!(app.read_with(cx, |view, _| view.tab), MainTab::Review);
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after preview: {error}")),
        config_before
    );
    assert_eq!(
        fs::read(home.join("auth.json"))
            .unwrap_or_else(|error| panic!("cannot read authentication after preview: {error}")),
        auth_before
    );
    assert_eq!(
        fs::read(home.join("auth-switcher-v2/modes.enc"))
            .unwrap_or_else(|error| panic!("cannot read archive after preview: {error}")),
        vault_before
    );

    cx.update_window(window, |_, window, cx| {
        app.update(cx, |this, cx| {
            this.busy = true;
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.find("back-to-settings").disabled(), Some(true));
        assert_eq!(window.find("cancel-preview").disabled(), Some(true));
        window.click("back-to-settings", cx);
        window.click("cancel-preview", cx);
    })
    .unwrap_or_else(|error| panic!("cannot check busy review controls: {error}"));
    assert_eq!(app.read_with(cx, |view, _| view.tab), MainTab::Review);
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));

    cx.update_window(window, |_, _, cx| {
        app.update(cx, |this, cx| {
            this.busy = false;
            cx.notify();
        });
    })
    .unwrap_or_else(|error| panic!("cannot release synthetic busy state: {error}"));
    click(cx, window, "back-to-settings");
    assert_eq!(app.read_with(cx, |view, _| view.tab), MainTab::Switch);
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));
    assert_eq!(
        app.read_with(cx, |view, cx| view.key.read(cx).value().to_string()),
        api_key
    );

    click(cx, window, "switch-tab");
    click(cx, window, "cancel-preview");
    assert_eq!(app.read_with(cx, |view, _| view.tab), MainTab::Switch);
    assert!(app.read_with(cx, |view, _| view.prepared.is_none()));
    assert_eq!(
        app.read_with(cx, |view, cx| view.key.read(cx).value().to_string()),
        api_key
    );
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after cancel: {error}")),
        config_before
    );
    assert_eq!(
        fs::read(home.join("auth.json"))
            .unwrap_or_else(|error| panic!("cannot read authentication after cancel: {error}")),
        auth_before
    );
    assert_eq!(
        fs::read(home.join("auth-switcher-v2/modes.enc"))
            .unwrap_or_else(|error| panic!("cannot read archive after cancel: {error}")),
        vault_before
    );
}

#[gpui_kit::test]
async fn catalog_save_and_discard_preserve_selected_model_and_saved_values(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    fs::write(
        &catalog_path,
        r#"{"catalog_version":3,"models":[{"slug":"first-model","display_name":"First Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"Synthetic high"}],"priority":1},{"slug":"second-model","display_name":"Second Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"Synthetic high"}],"priority":2}]}"#,
    )
    .unwrap_or_else(|error| panic!("cannot write test catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    click(cx, window, "catalog-model-1");
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 1);

    cx.update_window(window, |_, window, cx| {
        window.click("model-name", cx);
        window.press("end", cx);
        window.input("-saved", cx);
    })
    .unwrap_or_else(|error| panic!("cannot edit selected model: {error}"));
    click(cx, window, "catalog-save");
    idle(cx, window, &app).await;
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 1);
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .model_id
            .read(cx)
            .value()
            .to_string()),
        "second-model"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "Second Model-saved"
    );

    cx.update_window(window, |_, window, cx| {
        window.click("model-name", cx);
        window.press("end", cx);
        window.input("-draft", cx);
    })
    .unwrap_or_else(|error| panic!("cannot create discard draft: {error}"));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    click(cx, window, "catalog-discard");
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 1);
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "Second Model-saved"
    );
}

#[gpui_kit::test]
async fn opening_an_independent_catalog_supports_search_add_and_save_without_linking(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let config_before = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read initial test config: {error}"));
    let auth_before = fs::read(home.join("auth.json"))
        .unwrap_or_else(|error| panic!("cannot read initial test authentication: {error}"));
    let catalog_path = temp.path().join("selected-models.json");
    let original_catalog = br#"{"catalog_version":3,"catalog_metadata":{"source":"synthetic"},"models":[{"slug":"alpha-id","display_name":"Alpha Model","visibility":"hidden","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"High detail","source":"high-meta"},{"effort":"low","description":"Low detail","source":"low-meta"}],"priority":3,"model_metadata":{"keep":"alpha"}},{"slug":"beta-id","display_name":"Beta Model","visibility":"list","default_reasoning_level":"low","supported_reasoning_levels":[{"effort":"low","description":"Low detail","source":"low-meta"}],"priority":4,"model_metadata":{"keep":"beta"}},{"slug":"new-model-1","display_name":"Existing New Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"High detail"}],"priority":0}]}"#.to_vec();
    fs::write(&catalog_path, original_catalog.clone())
        .unwrap_or_else(|error| panic!("cannot write selected model catalog: {error}"));

    let selected_file = Arc::new(Mutex::new(Some(catalog_path.clone())));
    let picker_calls = Arc::new(AtomicUsize::new(0));
    let selected_file_for_picker = selected_file.clone();
    let picker_calls_for_picker = picker_calls.clone();
    let pick_model_catalog = Arc::new(move || {
        picker_calls_for_picker.fetch_add(1, Ordering::SeqCst);
        let selected = selected_file_for_picker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        Box::pin(async move { selected }) as DialogFuture
    });
    let (window, app) = open_with_dialogs(
        cx,
        home.clone(),
        1040.,
        740.,
        test_dialogs(pick_model_catalog),
    );
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-open");
    idle(cx, window, &app).await;
    assert_eq!(picker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 3);
    assert!(app.read_with(cx, |view, _| {
        view.catalog
            .catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.is_linked())
    }));
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after opening catalog: {error}")),
        config_before
    );
    assert_eq!(
        fs::read(home.join("auth.json"))
            .unwrap_or_else(|error| panic!("cannot read auth after opening catalog: {error}")),
        auth_before
    );

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "catalog-search", "ALPHA");
        window.render_frame(cx);
        assert!(window.try_find("catalog-model-0").is_some());
        assert!(window.try_find("catalog-model-1").is_none());
    })
    .unwrap_or_else(|error| panic!("cannot inspect display-name search results: {error}"));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .search
            .read(cx)
            .value()
            .to_string()),
        "ALPHA"
    );
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "catalog-search", "BETA-ID");
        window.render_frame(cx);
        assert!(window.try_find("catalog-model-0").is_none());
        assert!(window.try_find("catalog-model-1").is_some());
    })
    .unwrap_or_else(|error| panic!("cannot inspect model-ID search results: {error}"));
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "catalog-search", "");
    })
    .unwrap_or_else(|error| panic!("cannot clear model catalog search: {error}"));
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));

    click(cx, window, "catalog-add");
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 4);
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 3);
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .model_id
            .read(cx)
            .value()
            .to_string()),
        "new-model-2"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "新模型"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .visibility
            .read(cx)
            .value()
            .to_string()),
        "hidden"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .priority
            .read(cx)
            .value()
            .to_string()),
        "5"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .supported_reasoning
            .read(cx)
            .value()
            .to_string()),
        "high, low"
    );
    let added_model = app.read_with(cx, |view, _| {
        serde_json::to_value(
            view.catalog
                .catalog
                .as_ref()
                .unwrap_or_else(|| panic!("test catalog should remain loaded"))
                .models[3]
                .clone(),
        )
        .unwrap_or_else(|error| panic!("cannot serialize added test model: {error}"))
    });
    assert_eq!(added_model["model_metadata"]["keep"], "alpha");
    assert_eq!(
        added_model["supported_reasoning_levels"][0]["source"],
        "high-meta"
    );
    assert_eq!(
        added_model["supported_reasoning_levels"][1]["description"],
        "Low detail"
    );

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-id", "custom-model-v2");
        replace_input(window, cx, "model-name", "Custom Added Model");
    })
    .unwrap_or_else(|error| panic!("cannot edit added model fields: {error}"));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .model_id
            .read(cx)
            .value()
            .to_string()),
        "custom-model-v2"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "Custom Added Model"
    );
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("catalog-link").disabled(), Some(true));
        window.click("catalog-link", cx);
    })
    .unwrap_or_else(|error| panic!("cannot verify link is disabled for dirty draft: {error}"));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(app.read_with(cx, |view, _| {
        view.catalog
            .catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.is_linked())
    }));
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after blocked link: {error}")),
        config_before
    );
    click(cx, window, "catalog-save");
    idle(cx, window, &app).await;
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(app.read_with(cx, |view, _| {
        view.catalog
            .catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.is_linked())
    }));

    let saved: serde_json::Value = serde_json::from_slice(
        &fs::read(&catalog_path)
            .unwrap_or_else(|error| panic!("cannot read saved independent catalog: {error}")),
    )
    .unwrap_or_else(|error| panic!("cannot parse saved independent catalog: {error}"));
    assert_eq!(saved["catalog_version"], 3);
    assert_eq!(saved["catalog_metadata"]["source"], "synthetic");
    assert_eq!(saved["models"].as_array().map(Vec::len), Some(4));
    assert_eq!(saved["models"][3]["slug"], "custom-model-v2");
    assert_eq!(saved["models"][3]["display_name"], "Custom Added Model");
    assert_eq!(saved["models"][3]["model_metadata"]["keep"], "alpha");
    assert_eq!(
        saved["models"][3]["supported_reasoning_levels"][0]["source"],
        "high-meta"
    );
    let backups = fs::read_dir(
        catalog_path
            .parent()
            .unwrap_or_else(|| panic!("selected catalog should have a parent directory")),
    )
    .unwrap_or_else(|error| panic!("cannot list catalog backup directory: {error}"))
    .filter_map(std::result::Result::ok)
    .filter(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("selected-models.json.bak-")
    })
    .map(|entry| entry.path())
    .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1, "save should create one catalog backup");
    assert_eq!(
        fs::read(&backups[0]).unwrap_or_else(|error| panic!("cannot read catalog backup: {error}")),
        original_catalog
    );
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after save: {error}")),
        config_before
    );
    assert_eq!(
        fs::read(home.join("auth.json"))
            .unwrap_or_else(|error| panic!("cannot read auth after save: {error}")),
        auth_before
    );

    click(cx, window, "catalog-link");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.catalog
            .catalog
            .as_ref()
            .is_some_and(switcher_core::ModelCatalog::is_linked)
    }));
    let linked_config_bytes = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read config after explicit link: {error}"));
    assert_ne!(linked_config_bytes, config_before);
    let linked_config = fs::read_to_string(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot decode linked config: {error}"));
    assert!(linked_config.contains("model_catalog_json"));
    assert!(linked_config.contains("selected-models.json"));
    assert_eq!(
        fs::read(home.join("auth.json"))
            .unwrap_or_else(|error| panic!("cannot read auth after explicit link: {error}")),
        auth_before
    );
}

#[gpui_kit::test]
async fn invalid_catalog_priority_keeps_draft_until_repaired_or_discarded(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    fs::write(
        &catalog_path,
        r#"{"catalog_version":3,"models":[{"slug":"first-model","display_name":"First Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"Synthetic high"}],"priority":1},{"slug":"second-model","display_name":"Second Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"Synthetic high"}],"priority":2}]}"#,
    )
    .unwrap_or_else(|error| panic!("cannot write test catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);
    let catalog_before = fs::read(&catalog_path)
        .unwrap_or_else(|error| panic!("cannot read initial test catalog: {error}"));

    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-priority", "");
    })
    .unwrap_or_else(|error| panic!("cannot clear model priority: {error}"));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .priority
            .read(cx)
            .value()
            .to_string()),
        "",
        "the actual input value should be empty after Ctrl+A and Backspace"
    );
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(!app.read_with(cx, |view, _| view.catalog.priority_valid));
    assert!(!app.read_with(cx, |view, _| view.catalog.can_save()));

    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("catalog-priority-error").is_some());
        assert_ne!(
            window.find("model-priority").disabled(),
            Some(true),
            "the invalid priority field must remain editable"
        );
        for id in ["catalog-model-1", "catalog-clone", "catalog-save"] {
            assert_eq!(
                window.find(id).disabled(),
                Some(true),
                "{id} should be disabled"
            );
            window.click(id, cx);
        }
        assert_ne!(window.find("catalog-discard").disabled(), Some(true));
    })
    .unwrap_or_else(|error| panic!("cannot inspect invalid-priority controls: {error}"));
    cx.update_window(window, |_, window, cx| {
        app.update(cx, |this, cx| {
            this.catalog.select(1, window, cx);
            this.catalog.clone_selected(window, cx);
            this.clone_model(window, cx);
            this.save_catalog(window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot call invalid-priority guards: {error}"));
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 0);
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .priority
            .read(cx)
            .value()
            .to_string()),
        ""
    );
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(!app.read_with(cx, |view, _| view.catalog.priority_valid));
    assert_eq!(
        fs::read(&catalog_path)
            .unwrap_or_else(|error| panic!("cannot read catalog after guarded actions: {error}")),
        catalog_before
    );

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-priority", "-");
    })
    .unwrap_or_else(|error| panic!("cannot enter a negative-sign priority draft: {error}"));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .priority
            .read(cx)
            .value()
            .to_string()),
        "-"
    );
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(!app.read_with(cx, |view, _| view.catalog.priority_valid));
    assert!(!app.read_with(cx, |view, _| view.catalog.can_save()));

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-priority", "1");
    })
    .unwrap_or_else(|error| panic!("cannot restore original model priority: {error}"));
    assert!(app.read_with(cx, |view, _| view.catalog.priority_valid));
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(!app.read_with(cx, |view, _| view.catalog.can_save()));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("catalog-priority-error").is_none());
        assert_ne!(window.find("catalog-model-1").disabled(), Some(true));
        assert_ne!(window.find("catalog-clone").disabled(), Some(true));
        assert_eq!(window.find("catalog-discard").disabled(), Some(true));
    })
    .unwrap_or_else(|error| panic!("cannot inspect repaired-priority controls: {error}"));
    click(cx, window, "catalog-model-1");
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 1);
    click(cx, window, "catalog-model-0");
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 0);
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-priority", "-");
    })
    .unwrap_or_else(|error| panic!("cannot create invalid priority before discard: {error}"));
    click(cx, window, "catalog-discard");
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .priority
            .read(cx)
            .value()
            .to_string()),
        "1"
    );
    assert!(app.read_with(cx, |view, _| view.catalog.priority_valid));
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(
        fs::read(&catalog_path)
            .unwrap_or_else(|error| panic!("cannot read catalog after discard: {error}")),
        catalog_before
    );
}

#[gpui_kit::test]
async fn catalog_edits_survive_model_selection_and_returning_to_baseline_clears_dirty(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    fs::write(
        &catalog_path,
        r#"{"catalog_version":3,"models":[{"slug":"first-model","display_name":"First Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"Synthetic high"}],"priority":1},{"slug":"second-model","display_name":"Second Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"Synthetic high"}],"priority":2}]}"#,
    )
    .unwrap_or_else(|error| panic!("cannot write test catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;

    cx.update_window(window, |_, window, cx| {
        window.click("model-name", cx);
        window.press("end", cx);
        window.input("-draft", cx);
    })
    .unwrap_or_else(|error| panic!("cannot edit first model name: {error}"));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    click(cx, window, "catalog-model-1");
    assert_eq!(app.read_with(cx, |view, _| view.catalog.selected_model), 1);
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "Second Model"
    );
    cx.update_window(window, |_, window, cx| {
        window.click("model-name", cx);
        window.press("end", cx);
        window.input("-draft", cx);
    })
    .unwrap_or_else(|error| panic!("cannot edit second model name: {error}"));
    click(cx, window, "catalog-model-0");
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "First Model-draft"
    );
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-name", "First Model");
    })
    .unwrap_or_else(|error| panic!("cannot restore first model name: {error}"));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    click(cx, window, "catalog-model-1");
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "Second Model-draft"
    );
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-name", "Second Model");
    })
    .unwrap_or_else(|error| panic!("cannot restore second model name: {error}"));
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(app.read_with(cx, |view, _| view.catalog.priority_valid));
    assert!(!app.read_with(cx, |view, _| view.catalog.can_save()));
}

#[gpui_kit::test]
async fn restoring_supported_reasoning_levels_preserves_descriptions_and_metadata(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    fs::write(
        &catalog_path,
        r#"{"catalog_version":3,"models":[{"slug":"reasoning-model","display_name":"Reasoning Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"High detail","source":"test-high"},{"effort":"low","description":"Low detail","source":"test-low"}],"priority":1,"model_metadata":"keep-me"}]}"#,
    )
    .unwrap_or_else(|error| panic!("cannot write test catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-supported-reasoning", "high");
    })
    .unwrap_or_else(|error| panic!("cannot remove a supported reasoning level: {error}"));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .supported_reasoning
            .read(cx)
            .value()
            .to_string()),
        "high"
    );
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-supported-reasoning", "high, low");
    })
    .unwrap_or_else(|error| panic!("cannot restore supported reasoning levels: {error}"));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .supported_reasoning
            .read(cx)
            .value()
            .to_string()),
        "high, low"
    );

    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    let serialized = app.read_with(cx, |view, _| {
        serde_json::to_value(
            view.catalog
                .catalog
                .as_ref()
                .unwrap_or_else(|| panic!("test catalog should remain loaded"))
                .models[0]
                .clone(),
        )
        .unwrap_or_else(|error| panic!("cannot serialize restored test model: {error}"))
    });
    assert_eq!(serialized["model_metadata"], "keep-me");
    assert_eq!(
        serialized["supported_reasoning_levels"][0]["description"],
        "High detail"
    );
    assert_eq!(
        serialized["supported_reasoning_levels"][0]["source"],
        "test-high"
    );
    assert_eq!(
        serialized["supported_reasoning_levels"][1]["description"],
        "Low detail"
    );
    assert_eq!(
        serialized["supported_reasoning_levels"][1]["source"],
        "test-low"
    );
}

#[cfg(windows)]
#[gpui_kit::test]
async fn restart_confirmation_traps_focus_and_escape_cancels(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    let restart_calls = Arc::new(AtomicUsize::new(0));
    let calls = restart_calls.clone();
    app.update(cx, |this, _| {
        this.restart_handler = Arc::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
    });

    click(cx, window, "restart");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("dialog").is_some()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(gpui_kit::base::active_focus_trap(window, cx).is_some());
        assert!(window.try_find("confirm-restart").is_some());
        window.press("tab", cx);
        window.press("shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("confirm-restart").focused(),
            Some(true),
            "Tab then Shift+Tab should return focus to the confirmation action"
        );
        window.press("escape", cx);
    })
    .unwrap_or_else(|error| panic!("cannot inspect restart confirmation: {error}"));
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("dialog").is_none()
    })
    .await;
    assert_eq!(app.read_with(cx, |view, _| view.tab), MainTab::Switch);
    assert!(app.read_with(cx, |view, _| view.message.contains("已取消重启")));
    assert_eq!(restart_calls.load(Ordering::SeqCst), 0);

    click(cx, window, "restart");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("confirm-restart").is_some()
    })
    .await;
    click(cx, window, "confirm-restart");
    cx.wait_for(window, Duration::from_secs(1), |_, cx| {
        !app.read(cx).busy && restart_calls.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(restart_calls.load(Ordering::SeqCst), 1);
    assert!(
        app.read_with(cx, |view, _| view.message.contains("已重启")),
        "the confirmed callback should complete the controlled restart operation"
    );
}

#[gpui_kit::test]
async fn saving_catalog_refreshes_mode_readiness(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog = home.join("subscription-models.json");
    write_model_catalog(&catalog, "demo-subscription-model");
    configure_catalog_path(&home.join("config.toml"), &catalog);
    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    assert!(
        app.read_with(cx, |view, _| view.status.as_ref().is_some_and(|status| {
            status
                .modes
                .iter()
                .any(|mode| mode.mode == Mode::Subscription && mode.ready)
        }))
    );
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.click("model-id", cx);
        window.press("end", cx);
        window.input("-renamed", cx);
    })
    .unwrap_or_else(|error| panic!("cannot rename current catalog model: {error}"));
    click(cx, window, "catalog-save");
    idle(cx, window, &app).await;
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(
        app.read_with(cx, |view, _| view.status.as_ref().is_some_and(|status| {
            status.modes.iter().any(|mode| {
                mode.mode == Mode::Subscription && !mode.ready && !mode.issue.is_empty()
            })
        }))
    );

    click(cx, window, "switch-tab");
    click(cx, window, "subscription");
    assert_eq!(
        app.read_with(cx, |view, _| view.target),
        Mode::Subscription,
        "the target mode must be Subscription for this regression check"
    );
    assert!(app.read_with(cx, |view, _| {
        view.status.as_ref().is_some_and(|status| {
            status.modes.iter().any(|mode| {
                mode.mode == Mode::Subscription && !mode.ready && !mode.issue.is_empty()
            })
        })
    }));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("target-mode-issue").is_some(),
            "the unavailable target mode must show its issue"
        );
        assert!(
            window.try_find("saved-mode-issue-subscription").is_some(),
            "the saved Subscription card must show the same issue"
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect mode availability issues: {error}"));
}

#[gpui_kit::test]
async fn selecting_model_catalog_loads_it_and_preserves_current_data_on_cancel_or_error(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let selected_path = temp.path().join("selected-models.json");
    fs::write(
        &selected_path,
        r#"{"catalog_version":3,"models":[{"slug":"selected-model","display_name":"Selected Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"Synthetic high"}],"priority":1}]}"#,
    )
    .unwrap_or_else(|error| panic!("cannot write selected model catalog: {error}"));
    let invalid_path = temp.path().join("invalid-models.json");
    fs::write(&invalid_path, "not JSON")
        .unwrap_or_else(|error| panic!("cannot write invalid model catalog: {error}"));

    let selected_file = Arc::new(Mutex::new(Some(selected_path.clone())));
    let picker_calls = Arc::new(AtomicUsize::new(0));
    let selected_file_for_picker = selected_file.clone();
    let picker_calls_for_picker = picker_calls.clone();
    let pick_model_catalog = Arc::new(move || {
        picker_calls_for_picker.fetch_add(1, Ordering::SeqCst);
        let selected = selected_file_for_picker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        Box::pin(async move { selected }) as DialogFuture
    });
    let dialogs = test_dialogs(pick_model_catalog);
    let (window, app) = open_with_dialogs(cx, home.clone(), 1040., 740., dialogs);
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, cx| {
        view.catalog.configured_path.read(cx).value().is_empty()
    }));

    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));
    click(cx, window, "models-tab");

    click(cx, window, "catalog-choose");
    idle(cx, window, &app).await;
    assert_eq!(picker_calls.load(Ordering::SeqCst), 1);
    let loaded_path = app.read_with(cx, |view, _| {
        view.catalog
            .catalog
            .as_ref()
            .map(|catalog| PathBuf::from(catalog.path.as_str()))
    });
    assert_eq!(
        loaded_path.and_then(|path| path.file_name().map(PathBuf::from)),
        Some(PathBuf::from("selected-models.json"))
    );
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 1);
    assert!(app.read_with(cx, |view, _| view.prepared.is_none()));
    assert!(app.read_with(cx, |view, _| view.restart_needed));
    assert!(app.read_with(cx, |view, _| view.message.contains("重启 Codex")));
    assert!(app.read_with(cx, |view, _| view.message.contains("关联到 config.toml")));
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .and_then(|status| status.model_catalog_path.clone())),
        Some(selected_path.to_string_lossy().into_owned())
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .configured_path
            .read(cx)
            .value()
            .to_string()),
        selected_path.to_string_lossy()
    );
    let configured = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read configured test config: {error}"));
    assert!(String::from_utf8_lossy(&configured).contains("selected-models.json"));

    *selected_file
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;
    click(cx, window, "catalog-choose");
    idle(cx, window, &app).await;
    assert_eq!(picker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after cancellation: {error}")),
        configured
    );
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 1);
    assert!(app.read_with(cx, |view, _| view.message.contains("取消")));

    *selected_file
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(invalid_path);
    click(cx, window, "catalog-choose");
    idle(cx, window, &app).await;
    assert_eq!(picker_calls.load(Ordering::SeqCst), 3);
    assert!(app.read_with(cx, |view, _| view.error));
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 1);
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after invalid selection: {error}")),
        configured
    );

    cx.update_window(window, |_, window, cx| {
        window.click("model-name", cx);
        window.press("end", cx);
        window.input("-edit", cx);
    })
    .unwrap_or_else(|error| panic!("cannot edit model in test: {error}"));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    let call_count = picker_calls.load(Ordering::SeqCst);
    click(cx, window, "catalog-choose");
    idle(cx, window, &app).await;
    assert_eq!(picker_calls.load(Ordering::SeqCst), call_count);
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "Selected Model-edit"
    );
}

#[gpui_kit::test]
async fn unsaved_model_edits_block_mode_apply_and_remain_intact(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("subscription-models.json");
    write_model_catalog(&catalog_path, "subscription-model");
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home.clone(), 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.click("model-name", cx);
        window.press("end", cx);
        window.input("-unsaved", cx);
    })
    .unwrap_or_else(|error| panic!("cannot edit test model: {error}"));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));

    click(cx, window, "switch-tab");
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));

    let config_before = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    let vault_before = fs::read(home.join("auth-switcher-v2/modes.enc"))
        .unwrap_or_else(|error| panic!("cannot read test mode archive: {error}"));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("apply").disabled(), Some(true));
        window.click("apply", cx);
    })
    .unwrap_or_else(|error| panic!("cannot inspect blocked apply button: {error}"));

    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .and_then(|status| status.mode)),
        Some(Mode::Subscription)
    );
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .display_name
            .read(cx)
            .value()
            .to_string()),
        "subscription-model-unsaved"
    );
    assert_eq!(
        fs::read(home.join("config.toml"))
            .unwrap_or_else(|error| panic!("cannot read config after blocked apply: {error}")),
        config_before
    );
    assert_eq!(
        fs::read(home.join("auth-switcher-v2/modes.enc")).unwrap_or_else(|error| panic!(
            "cannot read mode archive after blocked apply: {error}"
        )),
        vault_before
    );

    cx.update_window(window, |_, window, cx| {
        app.update(cx, |this, cx| this.apply(window, cx));
    })
    .unwrap_or_else(|error| panic!("cannot check direct apply guard: {error}"));
    assert!(app.read_with(cx, |view, _| view.message.contains("未保存改动")));
    assert!(app.read_with(cx, |view, _| view.prepared.is_some()));
    assert!(app.read_with(cx, |view, _| view.catalog.dirty));
}

#[gpui_kit::test]
async fn successful_mode_switch_clears_the_previous_model_catalog(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let subscription_catalog = home.join("subscription-models.json");
    let api_catalog = home.join("api-models.json");
    write_model_catalog(&subscription_catalog, "subscription-model");
    write_model_catalog(&api_catalog, "demo-api-model");
    configure_catalog_path(&home.join("config.toml"), &subscription_catalog);
    configure_catalog_path(&home.join("config.toml.bak-demo-api"), &api_catalog);

    let (window, app) = open(cx, home.clone(), 1040., 740.);
    idle(cx, window, &app).await;
    assert_eq!(
        PathBuf::from(app.read_with(cx, |view, cx| {
            view.catalog.configured_path.read(cx).value().to_string()
        })),
        subscription_catalog
    );
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.catalog
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.path.ends_with("subscription-models.json"))
    }));

    click(cx, window, "switch-tab");
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    click(cx, window, "apply");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .and_then(|status| status.mode)),
        Some(Mode::Api)
    );
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 0);
    assert!(app.read_with(cx, |view, _| view.catalog.catalog.is_none()));
    assert_eq!(
        PathBuf::from(app.read_with(cx, |view, cx| {
            view.catalog.configured_path.read(cx).value().to_string()
        })),
        api_catalog
    );

    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.catalog
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.path.ends_with("api-models.json"))
    }));
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 1);
    assert!(app.read_with(cx, |view, cx| {
        view.catalog
            .display_name
            .read(cx)
            .value()
            .contains("demo-api-model")
    }));
}

#[gpui_kit::test]
async fn cloned_model_restores_removed_reasoning_metadata(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    fs::write(
        &catalog_path,
        r#"{"catalog_version":3,"models":[{"slug":"reasoning-model","display_name":"Reasoning Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high","description":"High detail","source":"test-high"},{"effort":"low","description":"Low detail","source":"test-low"}],"priority":1,"model_metadata":"keep-me"}]}"#,
    )
    .unwrap_or_else(|error| panic!("cannot write test catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;

    click(cx, window, "catalog-clone");
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 2);
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .model_id
            .read(cx)
            .value()
            .to_string()),
        "new-model-1"
    );
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-supported-reasoning", "high");
    })
    .unwrap_or_else(|error| panic!("cannot remove reasoning level from clone: {error}"));
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-supported-reasoning", "high, low");
    })
    .unwrap_or_else(|error| panic!("cannot restore reasoning level on clone: {error}"));

    let cloned_model = app.read_with(cx, |view, _| {
        serde_json::to_value(
            view.catalog
                .catalog
                .as_ref()
                .unwrap_or_else(|| panic!("test catalog should remain loaded"))
                .models[1]
                .clone(),
        )
        .unwrap_or_else(|error| panic!("cannot serialize cloned test model: {error}"))
    });
    assert_eq!(cloned_model["model_metadata"], "keep-me");
    assert_eq!(
        cloned_model["supported_reasoning_levels"][0]["description"],
        "High detail"
    );
    assert_eq!(
        cloned_model["supported_reasoning_levels"][0]["source"],
        "test-high"
    );
    assert_eq!(
        cloned_model["supported_reasoning_levels"][1]["description"],
        "Low detail"
    );
    assert_eq!(
        cloned_model["supported_reasoning_levels"][1]["source"],
        "test-low"
    );
}

#[gpui_kit::test]
async fn catalog_add_and_clone_stop_at_the_core_model_limit(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    let models = (0..512)
        .map(|index| {
            serde_json::json!({
                "slug": format!("model-{index}"),
                "display_name": format!("Model {index}"),
                "visibility": "list",
                "default_reasoning_level": "high",
                "supported_reasoning_levels": [{"effort": "high"}],
                "priority": index,
            })
        })
        .collect::<Vec<_>>();
    let contents = serde_json::to_vec(&serde_json::json!({
        "catalog_version": 3,
        "models": models,
    }))
    .unwrap_or_else(|error| panic!("cannot serialize test model catalog: {error}"));
    fs::write(&catalog_path, contents)
        .unwrap_or_else(|error| panic!("cannot write maximum-size test catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 512);
    assert!(!app.read_with(cx, |view, _| view.catalog.can_add()));

    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        for id in ["catalog-add", "catalog-clone"] {
            assert_eq!(window.find(id).disabled(), Some(true));
            window.click(id, cx);
        }
        app.update(cx, |this, cx| {
            assert!(!this.catalog.clone_selected(window, cx));
            this.clone_model(window, cx);
            this.add_model(window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot inspect model limit behavior: {error}"));
    assert_eq!(app.read_with(cx, |view, _| view.catalog.model_count()), 512);
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert!(app.read_with(cx, |view, _| view.error));
    assert!(app.read_with(cx, |view, _| view.message.contains("512")));
}

#[gpui_kit::test]
async fn catalog_search_survives_saving_and_discarding_edits(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    fs::write(
        &catalog_path,
        r#"{"catalog_version":3,"models":[{"slug":"first-model","display_name":"First Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high"}],"priority":1},{"slug":"second-model","display_name":"Second Model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high"}],"priority":2}]}"#,
    )
    .unwrap_or_else(|error| panic!("cannot write test catalog: {error}"));
    configure_catalog_path(&home.join("config.toml"), &catalog_path);

    let (window, app) = open(cx, home, 1040., 740.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "catalog-search", "first-model");
        window.render_frame(cx);
        assert!(window.try_find("catalog-model-0").is_some());
        assert!(window.try_find("catalog-model-1").is_none());
    })
    .unwrap_or_else(|error| panic!("cannot filter test model catalog: {error}"));

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-name", "Edited Model");
    })
    .unwrap_or_else(|error| panic!("cannot edit first model name: {error}"));
    click(cx, window, "catalog-save");
    idle(cx, window, &app).await;
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .search
            .read(cx)
            .value()
            .to_string()),
        "first-model"
    );
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("catalog-model-0").is_some());
        assert!(window.try_find("catalog-model-1").is_none());
    })
    .unwrap_or_else(|error| panic!("search filter disappeared after save: {error}"));

    cx.update_window(window, |_, window, cx| {
        replace_input(window, cx, "model-name", "Discarded Model");
    })
    .unwrap_or_else(|error| panic!("cannot create discard draft: {error}"));
    click(cx, window, "catalog-discard");
    assert!(!app.read_with(cx, |view, _| view.catalog.dirty));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .catalog
            .search
            .read(cx)
            .value()
            .to_string()),
        "first-model"
    );
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("catalog-model-0").is_some());
        assert!(window.try_find("catalog-model-1").is_none());
    })
    .unwrap_or_else(|error| panic!("search filter disappeared after discard: {error}"));
}
