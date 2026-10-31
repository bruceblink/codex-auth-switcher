//! GPUI desktop application state and asynchronous core operations.
//!
//! Rendering lives in [`view`]. This module owns the input entities and state
//! transitions that may start filesystem work. Core work runs on GPUI's
//! background executor and updates this entity only when it completes.

mod catalog;
mod format;
mod view;

#[cfg(test)]
mod tests;

use catalog::CatalogEditor;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, ScrollHandle, Subscription, Window, WindowBounds,
    WindowOptions, px, rgb, size,
};
use rfd::AsyncFileDialog;
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};
use switcher_core::{
    ImportSources, Mode, Paths, PreparedSwitch, Status, Store, SwitchOptions, config,
};
use zeroize::Zeroizing;

type CoreResult<T> = switcher_core::Result<T>;
type DialogFuture = Pin<Box<dyn Future<Output = Option<PathBuf>> + Send + 'static>>;

/// File dialogs are injected so the UI tests can use deterministic, isolated paths.
#[derive(Clone)]
pub(crate) struct FileDialogs {
    pub(crate) pick_file: Arc<dyn Fn(bool) -> DialogFuture + Send + Sync>,
    pub(crate) pick_folder: Arc<dyn Fn() -> DialogFuture + Send + Sync>,
}

impl FileDialogs {
    fn system() -> Self {
        Self {
            pick_file: Arc::new(|auth| {
                Box::pin(async move {
                    let dialog = AsyncFileDialog::new().set_title(if auth {
                        "导入认证 JSON"
                    } else {
                        "导入模式 TOML"
                    });
                    let file = if auth {
                        dialog.add_filter("JSON", &["json"]).pick_file().await
                    } else {
                        dialog.add_filter("TOML", &["toml"]).pick_file().await
                    };
                    file.map(|file| file.path().to_path_buf())
                })
            }),
            pick_folder: Arc::new(|| {
                Box::pin(async move {
                    AsyncFileDialog::new()
                        .set_title("选择 Codex 配置目录")
                        .pick_folder()
                        .await
                        .map(|folder| folder.path().to_path_buf())
                })
            }),
        }
    }
}

/// Which main tab is visible in the single application window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainTab {
    Switch,
    Models,
}

/// The one GPUI entity backing the application window.
pub(crate) struct Switcher {
    pub(crate) store: Store,
    pub(crate) file_dialogs: FileDialogs,
    pub(crate) target: Mode,
    pub(crate) tab: MainTab,
    pub(crate) status: Option<Status>,
    pub(crate) prepared: Option<PreparedSwitch>,
    pub(crate) directory: Entity<InputState>,
    pub(crate) subscription_provider: Entity<InputState>,
    pub(crate) api_provider: Entity<InputState>,
    pub(crate) endpoint: Entity<InputState>,
    pub(crate) key: Entity<InputState>,
    pub(crate) key_subscription: Option<Subscription>,
    _input_subscriptions: Vec<Subscription>,
    pub(crate) busy: bool,
    pub(crate) message: String,
    pub(crate) error: bool,
    pub(crate) directory_needs_refresh: bool,
    pub(crate) restart_needed: bool,
    pub(crate) restart_confirmation_open: bool,
    pub(crate) catalog: CatalogEditor,
    pub(crate) switch_scroll: ScrollHandle,
    pub(crate) catalog_scroll: ScrollHandle,
    pub(crate) model_list_scroll: ScrollHandle,
}

impl Switcher {
    pub(crate) fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_dialogs(store, FileDialogs::system(), window, cx)
    }

    pub(crate) fn with_dialogs(
        store: Store,
        file_dialogs: FileDialogs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let directory = input("Codex 配置目录", false, 4096, window, cx);
        directory.update(cx, |state, cx| {
            state.set_value(store.paths.home.to_string_lossy(), window, cx)
        });
        let subscription_provider = input("openai", false, 128, window, cx);
        let api_provider = input("openai", false, 128, window, cx);
        let endpoint = input(config::DEFAULT_API_BASE_URL, false, 2048, window, cx);
        let key = input("API key", true, 16384, window, cx);
        subscription_provider.update(cx, |state, cx| state.set_value("openai", window, cx));
        api_provider.update(cx, |state, cx| state.set_value("openai", window, cx));
        endpoint.update(cx, |state, cx| {
            state.set_value(config::DEFAULT_API_BASE_URL, window, cx)
        });

        let directory_subscription = cx.subscribe(&directory, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.directory_needs_refresh = true;
                this.status = None;
                this.prepared = None;
                cx.notify();
            }
        });
        let mut input_subscriptions = vec![directory_subscription];
        for field in [&subscription_provider, &api_provider, &endpoint] {
            input_subscriptions.push(cx.subscribe(field, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.prepared = None;
                    cx.notify();
                }
            }));
        }
        let key_subscription = Some(cx.subscribe(&key, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.prepared = None;
                cx.notify();
            }
        }));
        let catalog = CatalogEditor::new(window, cx);
        let mut this = Self {
            store,
            file_dialogs,
            target: Mode::Api,
            tab: MainTab::Switch,
            status: None,
            prepared: None,
            directory,
            subscription_provider,
            api_provider,
            endpoint,
            key,
            key_subscription,
            _input_subscriptions: input_subscriptions,
            busy: false,
            message: String::new(),
            error: false,
            directory_needs_refresh: false,
            restart_needed: false,
            restart_confirmation_open: false,
            catalog,
            switch_scroll: ScrollHandle::default(),
            catalog_scroll: ScrollHandle::default(),
            model_list_scroll: ScrollHandle::default(),
        };
        this.refresh(window, cx);
        this
    }

    /// Schedule core work away from the UI thread, then apply the result on the
    /// UI thread. Only one operation may be pending at a time.
    pub(crate) fn execute<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        job: impl FnOnce(Store) -> CoreResult<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = false;
        self.message = "正在处理...".into();
        cx.notify();
        let store = self.store.clone();
        let task = cx.background_executor().spawn(async move { job(store) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(value) => done(this, value, window, cx),
                    Err(error) => {
                        this.message = format::user_error(&error);
                        this.error = true;
                        this.prepared = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn select_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let home = self.directory.read(cx).value();
        if home.trim().is_empty() {
            self.message = "请选择 Codex 配置目录".into();
            self.error = true;
            cx.notify();
            return false;
        }
        match Paths::resolve(Some(PathBuf::from(home.trim()))) {
            Ok(paths) => {
                if paths.home != self.store.paths.home && self.catalog.dirty {
                    self.message = "请先保存或放弃当前模型目录改动，再选择其他配置目录。".into();
                    self.error = true;
                    cx.notify();
                    return false;
                }
                if paths.home != self.store.paths.home {
                    self.catalog.clear_loaded(window, cx);
                    self.status = None;
                }
                self.store = self.store.with_paths(paths);
                self.directory_needs_refresh = false;
                true
            }
            Err(error) => {
                self.message = format::user_error(&error);
                self.error = true;
                cx.notify();
                false
            }
        }
    }

    pub(crate) fn sync_profiles(
        &mut self,
        status: &Status,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(profile) = status
            .modes
            .iter()
            .find(|profile| profile.mode == Mode::Subscription)
        {
            let provider = profile.provider.clone();
            self.subscription_provider
                .update(cx, |state, cx| state.set_value(provider, window, cx));
        }
        if let Some(profile) = status
            .modes
            .iter()
            .find(|profile| profile.mode == Mode::Api)
        {
            let provider = profile.provider.clone();
            let endpoint = profile.base_url.clone();
            self.api_provider
                .update(cx, |state, cx| state.set_value(provider, window, cx));
            self.endpoint
                .update(cx, |state, cx| state.set_value(endpoint, window, cx));
        }
    }

    pub(crate) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepared = None;
        if self.catalog.dirty {
            self.message = "请先保存或放弃模型目录改动，再刷新状态。".into();
            self.error = true;
            cx.notify();
            return;
        }
        if !self.select_directory(window, cx) {
            return;
        }
        self.status = None;
        self.execute(
            window,
            cx,
            |store| store.status(),
            |this, status, window, cx| {
                this.target = if status.mode == Some(Mode::Api) {
                    Mode::Subscription
                } else {
                    Mode::Api
                };
                this.sync_profiles(&status, window, cx);
                this.message = if !status.config_exists {
                    "首次使用：请创建默认配置，再登录 Codex 或输入 API key。"
                } else if status.initialized {
                    "已读取本地状态"
                } else {
                    "尚未初始化模式存档"
                }
                .into();
                this.directory_needs_refresh = false;
                this.status = Some(status);
            },
        );
    }

    pub(crate) fn initialize(
        &mut self,
        sources: ImportSources,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prepared = None;
        if !self.select_directory(window, cx) {
            return;
        }
        self.execute(
            window,
            cx,
            move |store| store.initialize(sources),
            |this, status, window, cx| {
                let config_was_missing = this
                    .status
                    .as_ref()
                    .is_some_and(|previous| !previous.config_exists);
                this.restart_needed |= config_was_missing && status.config_exists;
                this.sync_profiles(&status, window, cx);
                this.status = Some(status);
                this.message = "模式存档已更新，当前认证未切换".into();
            },
        );
    }

    pub(crate) fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.directory_needs_refresh {
            self.message = "配置目录已修改，请先刷新状态后再预览。".into();
            self.error = true;
            cx.notify();
            return;
        }
        self.prepared = None;
        if !self.select_directory(window, cx) {
            return;
        }
        let provider = if self.target == Mode::Api {
            self.api_provider.read(cx).value().to_string()
        } else {
            self.subscription_provider.read(cx).value().to_string()
        };
        let options = if self.target == Mode::Api {
            SwitchOptions {
                provider: format::nonempty(&provider),
                base_url: format::nonempty(self.endpoint.read(cx).value().as_ref()),
                api_key: format::nonempty(self.key.read(cx).value().as_ref()).map(Zeroizing::new),
            }
        } else {
            SwitchOptions {
                provider: format::nonempty(&provider),
                ..Default::default()
            }
        };
        let target = self.target;
        self.execute(
            window,
            cx,
            move |store| store.prepare(target, options),
            |this, prepared, _, _| {
                this.message = if prepared.preview.changed {
                    "预览已生成，尚未写入配置"
                } else {
                    "目标与当前配置一致"
                }
                .into();
                this.prepared = Some(prepared);
            },
        );
    }

    pub(crate) fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prepared) = self.prepared.take() else {
            return;
        };
        self.execute(
            window,
            cx,
            move |store| Ok(store.apply(prepared).map(|result| (result, store.status()))),
            |this, result, window, cx| {
                this.clear_key(window, cx);
                match result {
                    Ok((result, status)) => {
                        this.restart_needed |= result.restart_required;
                        this.message = if result.changed {
                            format!(
                                "已切换到 {}。重启 Codex 后请新建会话。",
                                format::mode_name(result.target)
                            )
                        } else {
                            "配置未变化".into()
                        };
                        if let Some(persisted) = result.environment_persisted {
                            if persisted {
                                this.message.push_str(
                                    " API key 已保存到当前用户环境变量 AUTOCODE_API_KEY。",
                                );
                            } else {
                                this.message.push_str(
                                    " 切换已完成，但无法保存当前用户环境变量 AUTOCODE_API_KEY。",
                                );
                                this.error = true;
                            }
                        }
                        match status {
                            Ok(status) => {
                                this.sync_profiles(&status, window, cx);
                                this.status = Some(status);
                            }
                            Err(error) => {
                                this.status = None;
                                this.message.push_str(&format!(
                                    " 状态读取失败：{}",
                                    format::user_error(&error)
                                ));
                                this.error = true;
                            }
                        }
                    }
                    Err(error) => {
                        this.message = format::user_error(&error);
                        this.error = true;
                        this.prepared = None;
                    }
                }
            },
        );
    }

    /// Clear the displayed value and replace its entity to release edit history.
    pub(crate) fn clear_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.key = input("API key", true, 16384, window, cx);
        self.key_subscription = Some(cx.subscribe(&self.key, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.prepared = None;
                cx.notify();
            }
        }));
    }

    pub(crate) fn import(&mut self, auth: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.prepared = None;
        self.busy = true;
        self.error = false;
        self.message = "正在打开文件选择器...".into();
        cx.notify();
        let target = self.target;
        let picker = self.file_dialogs.pick_file.clone();
        cx.spawn_in(window, async move |this, cx| {
            let selected = picker(auth).await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                if let Some(path) = selected {
                    let mut sources = ImportSources::default();
                    match (target, auth) {
                        (Mode::Subscription, false) => sources.subscription_config = Some(path),
                        (Mode::Subscription, true) => sources.subscription_auth = Some(path),
                        (Mode::Api, false) => sources.api_config = Some(path),
                        (Mode::Api, true) => sources.api_auth = Some(path),
                    }
                    this.initialize(sources, window, cx);
                } else {
                    this.message = "文件选择已取消，未修改本地文件。".into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.catalog.dirty {
            return;
        }
        self.prepared = None;
        self.busy = true;
        self.error = false;
        self.message = "正在打开目录选择器...".into();
        cx.notify();
        let picker = self.file_dialogs.pick_folder.clone();
        cx.spawn_in(window, async move |this, cx| {
            let selected = picker().await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                if let Some(path) = selected {
                    this.clear_key(window, cx);
                    this.directory.update(cx, |state, cx| {
                        state.set_value(path.to_string_lossy(), window, cx)
                    });
                    this.refresh(window, cx);
                } else {
                    this.message = "目录选择已取消，未修改本地文件。".into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let allowed = crate::restart::supported()
            && !self.busy
            && !self.catalog.dirty
            && !self.directory_needs_refresh
            && self.key.read(cx).value().is_empty()
            && self
                .prepared
                .as_ref()
                .is_none_or(|prepared| !prepared.preview.changed)
            && self
                .status
                .as_ref()
                .is_some_and(|status| status.config_exists && !status.recovery_pending);
        if !allowed {
            return;
        }
        self.restart_confirmation_open = false;
        self.execute(
            window,
            cx,
            |_| crate::restart::restart(),
            |this, (), _, _| {
                this.restart_needed = false;
                this.message = "Codex 桌面应用已重启。请新建会话以使用最新配置。".into();
            },
        );
    }
}

fn input(
    placeholder: impl Into<gpui_kit::SharedString>,
    masked: bool,
    limit: usize,
    window: &mut Window,
    cx: &mut Context<Switcher>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(masked)
            .validate(move |value, _| value.chars().count() <= limit)
    })
}

pub(crate) fn run(store: Store) -> anyhow::Result<()> {
    let icon = std::sync::Arc::new(
        image::load_from_memory(include_bytes!("../assets/codex-auth-switcher.png"))?.to_rgba8(),
    );
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Light, None, cx);
            let theme = gpui_kit::component::Theme::global_mut(cx);
            theme.primary = rgb(0x167454).into();
            theme.primary_hover = rgb(0x126247).into();
            theme.primary_active = rgb(0x0f513b).into();
            theme.tokens = (&theme.colors).into();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1040.), px(740.)),
                    cx,
                ))),
                window_min_size: Some(size(px(720.), px(560.))),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("Codex Auth Switcher".into()),
                    ..Default::default()
                }),
                icon: Some(icon),
                ..Default::default()
            };
            if gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Switcher::new(store, window, cx))
            })
            .is_err()
            {
                eprintln!("Cannot open the GPUI Kit window.");
                cx.quit();
            }
            cx.activate(true);
        });
    Ok(())
}
