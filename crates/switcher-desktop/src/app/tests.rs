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
        assert_eq!(initialize.label(), Some("创建默认配置"));
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
