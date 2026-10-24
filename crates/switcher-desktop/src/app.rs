//! Kit owns rendering and input; the core owns validation and persistent writes.
mod view;
use gpui_kit::component::{
    Theme, ThemeMode,
    input::{InputEvent, InputState},
};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, Subscription, Window, WindowBounds, WindowOptions,
    px, rgb, size,
};
use std::path::PathBuf;
use switcher_core::{ImportSources, Mode, Paths, PreparedSwitch, Status, Store, SwitchOptions};
use zeroize::Zeroizing;

pub fn run(store: Store, demo: bool) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            let theme = Theme::global_mut(cx);
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
                    size(px(1050.0), px(830.0)),
                    cx,
                ))),
                window_min_size: Some(size(px(720.0), px(600.0))),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("Codex Auth Switcher v2".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            if gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Switcher::new(store, demo, window, cx))
            })
            .is_err()
            {
                eprintln!("Cannot open the GPUI Kit window.");
                cx.quit();
            }
            cx.activate(true);
        });
}

struct Switcher {
    store: Store,
    demo: bool,
    target: Mode,
    status: Option<Status>,
    prepared: Option<PreparedSwitch>,
    directory: Entity<InputState>,
    subscription_provider: Entity<InputState>,
    api_provider: Entity<InputState>,
    endpoint: Entity<InputState>,
    key: Entity<InputState>,
    busy: bool,
    message: String,
    error: bool,
    _subscriptions: Vec<Subscription>,
    key_subscription: Option<Subscription>,
}

impl Switcher {
    fn new(store: Store, demo: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let directory = input("Codex 目录", false, 4096, window, cx);
        directory.update(cx, |state, cx| {
            state.set_value(store.paths.home.to_string_lossy().into_owned(), window, cx)
        });
        let subscription_provider = input("openai", false, 128, window, cx);
        let api_provider = input("OpenAI", false, 128, window, cx);
        let endpoint = input("https://gateway.example.com/v1", false, 2048, window, cx);
        let key = input("API key", true, 16384, window, cx);
        let mut subscriptions = Vec::new();
        for field in [&directory, &subscription_provider, &api_provider, &endpoint] {
            subscriptions.push(cx.subscribe(field, |this, _, event: &InputEvent, cx| {
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
        let mut this = Self {
            store,
            demo,
            target: Mode::Api,
            status: None,
            prepared: None,
            directory,
            subscription_provider,
            api_provider,
            endpoint,
            key,
            busy: false,
            message: String::new(),
            error: false,
            _subscriptions: subscriptions,
            key_subscription,
        };
        this.refresh(window, cx);
        this
    }

    /// Filesystem work runs off the UI thread. Only one operation runs at a time.
    fn execute<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        job: impl FnOnce(Store) -> switcher_core::Result<T> + Send + 'static,
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
                        this.message = error.to_string();
                        this.error = true;
                        this.prepared = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_directory(&mut self, cx: &mut Context<Self>) -> bool {
        let home = self.directory.read(cx).value();
        if home.trim().is_empty() {
            self.message = "请选择 Codex 配置目录".into();
            self.error = true;
            cx.notify();
            return false;
        }
        match Paths::resolve(Some(PathBuf::from(home.trim()))) {
            Ok(paths) => {
                self.store = if self.demo {
                    Store::isolated(paths)
                } else {
                    Store::new(paths)
                };
                true
            }
            Err(error) => {
                self.message = error.to_string();
                self.error = true;
                cx.notify();
                false
            }
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepared = None;
        if !self.select_directory(cx) {
            return;
        }
        self.status = None;
        self.execute(
            window,
            cx,
            |store| store.status(),
            |this, status, _, _| {
                this.target = if status.mode == Some(Mode::Api) {
                    Mode::Subscription
                } else {
                    Mode::Api
                };
                this.message = if status.initialized {
                    "已读取本地状态"
                } else {
                    "尚未初始化模式存档"
                }
                .into();
                this.status = Some(status);
            },
        );
    }

    fn initialize(&mut self, sources: ImportSources, window: &mut Window, cx: &mut Context<Self>) {
        self.prepared = None;
        if !self.select_directory(cx) {
            return;
        }
        self.execute(
            window,
            cx,
            move |store| store.initialize(sources),
            |this, status, _, _| {
                this.status = Some(status);
                this.message = "模式存档已更新，当前认证未切换".into();
            },
        );
    }

    fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepared = None;
        if !self.select_directory(cx) {
            return;
        }
        let provider = if self.target == Mode::Api {
            &self.api_provider
        } else {
            &self.subscription_provider
        };
        let provider = nonempty(provider.read(cx).value().as_ref());
        let options = if self.target == Mode::Api {
            SwitchOptions {
                provider,
                base_url: nonempty(self.endpoint.read(cx).value().as_ref()),
                api_key: nonempty(self.key.read(cx).value().as_ref()).map(Zeroizing::new),
            }
        } else {
            SwitchOptions {
                provider,
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

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prepared) = self.prepared.take() else {
            return;
        };
        self.execute(
            window,
            cx,
            move |store| {
                let result = store.apply(prepared)?;
                Ok((result, store.status()))
            },
            |this, (result, status), window, cx| {
                this.clear_key(window, cx);
                this.message = if result.changed {
                    format!(
                        "已切换到 {}。重启 Codex 后请新建会话。",
                        mode_name(result.target)
                    )
                } else {
                    "配置未变化".into()
                };
                match status {
                    Ok(status) => this.status = Some(status),
                    Err(error) => {
                        this.status = None;
                        this.message.push_str(&format!(" 状态读取失败：{error}"));
                        this.error = true;
                    }
                }
            },
        );
    }

    /// Replace the input entity so its undo history cannot reveal the applied key.
    fn clear_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key = input("API key", true, 16384, window, cx);
        self.key_subscription = Some(cx.subscribe(&self.key, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.prepared = None;
                cx.notify();
            }
        }));
    }

    fn import(&mut self, auth: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.prepared = None;
        self.busy = true;
        cx.notify();
        let target = self.target;
        cx.spawn_in(window, async move |this, cx| {
            let dialog = rfd::AsyncFileDialog::new().set_title(if auth {
                "导入认证 JSON"
            } else {
                "导入模式 TOML"
            });
            let selected = if auth {
                dialog.add_filter("JSON", &["json"]).pick_file().await
            } else {
                dialog.add_filter("TOML", &["toml"]).pick_file().await
            };
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                if let Some(file) = selected {
                    let mut sources = ImportSources::default();
                    match (target, auth) {
                        (Mode::Subscription, false) => {
                            sources.subscription_config = Some(file.path().into())
                        }
                        (Mode::Subscription, true) => {
                            sources.subscription_auth = Some(file.path().into())
                        }
                        (Mode::Api, false) => sources.api_config = Some(file.path().into()),
                        (Mode::Api, true) => sources.api_auth = Some(file.path().into()),
                    }
                    this.initialize(sources, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.prepared = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let selected = rfd::AsyncFileDialog::new()
                .set_title("选择 Codex 配置目录")
                .pick_folder()
                .await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                if let Some(file) = selected {
                    this.clear_key(window, cx);
                    this.directory.update(cx, |state, cx| {
                        state.set_value(file.path().to_string_lossy().into_owned(), window, cx)
                    });
                    this.refresh(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn input<T: 'static>(
    placeholder: &'static str,
    masked: bool,
    limit: usize,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(masked)
            .validate(move |value, _| value.len() <= limit)
    })
}
fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}
fn mode_name(mode: Mode) -> &'static str {
    if mode == Mode::Api {
        "API key"
    } else {
        "订阅"
    }
}
