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
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};
use switcher_core::{ImportSources, Mode, Paths, PreparedSwitch, Status, Store, SwitchOptions};
use zeroize::Zeroizing;

type DialogFuture = Pin<Box<dyn Future<Output = Option<PathBuf>> + Send + 'static>>;

#[derive(Clone)]
struct FileDialogs {
    pick_file: Arc<dyn Fn(bool) -> DialogFuture + Send + Sync>,
    pick_folder: Arc<dyn Fn() -> DialogFuture + Send + Sync>,
}

impl FileDialogs {
    fn system() -> Self {
        Self {
            pick_file: Arc::new(|auth| {
                Box::pin(async move {
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
                    selected.map(|file| file.path().to_path_buf())
                })
            }),
            pick_folder: Arc::new(|| {
                Box::pin(async move {
                    rfd::AsyncFileDialog::new()
                        .set_title("选择 Codex 配置目录")
                        .pick_folder()
                        .await
                        .map(|folder| folder.path().to_path_buf())
                })
            }),
        }
    }
}

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
    file_dialogs: FileDialogs,
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
        Self::with_dialogs(store, demo, FileDialogs::system(), window, cx)
    }

    fn with_dialogs(
        store: Store,
        demo: bool,
        file_dialogs: FileDialogs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
            file_dialogs,
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
                        this.message = user_error(&error);
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

    /// Keep editable fields aligned with imported or refreshed profile data.
    fn sync_profiles(&mut self, status: &Status, window: &mut Window, cx: &mut Context<Self>) {
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
            let base_url = profile.base_url.clone();
            self.api_provider
                .update(cx, |state, cx| state.set_value(provider, window, cx));
            self.endpoint
                .update(cx, |state, cx| state.set_value(base_url, window, cx));
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
            |this, status, window, cx| {
                this.target = if status.mode == Some(Mode::Api) {
                    Mode::Subscription
                } else {
                    Mode::Api
                };
                this.sync_profiles(&status, window, cx);
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
            |this, status, window, cx| {
                this.sync_profiles(&status, window, cx);
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
                    Ok(status) => {
                        this.sync_profiles(&status, window, cx);
                        this.status = Some(status);
                    }
                    Err(error) => {
                        this.status = None;
                        this.message
                            .push_str(&format!(" 状态读取失败：{}", user_error(&error)));
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
        let pick_file = self.file_dialogs.pick_file.clone();
        cx.spawn_in(window, async move |this, cx| {
            let selected = pick_file(auth).await;
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
        let pick_folder = self.file_dialogs.pick_folder.clone();
        cx.spawn_in(window, async move |this, cx| {
            let selected = pick_folder().await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                if let Some(path) = selected {
                    this.clear_key(window, cx);
                    this.directory.update(cx, |state, cx| {
                        state.set_value(path.to_string_lossy().into_owned(), window, cx)
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

/// Keep filesystem and recovery failures understandable at the Chinese UI boundary.
fn user_error(error: &switcher_core::Error) -> String {
    let message = match error.0 {
        "Encryption key is missing. Restore vault.key; existing ciphertext was not changed." => {
            "本地加密密钥缺失。请恢复 vault.key；工具未修改现有密文。"
        }
        "Cannot authenticate encrypted state. Check the key and ciphertext." => {
            "加密存档验证失败。请检查 vault.key 与密文；工具未修改现有存档。"
        }
        "Invalid local encryption key." | "Invalid encryption key." => {
            "本地加密密钥无效。请恢复正确的 vault.key。"
        }
        "Invalid encrypted state format or size." => "加密存档格式无效或大小超出限制。",
        "Encrypted state exceeds the size limit." => "加密存档超过大小限制。",
        "State encryption failed." => "本地状态加密失败。",
        "Cannot restrict local state permissions. Use a private NTFS user directory." => {
            "无法限制本地存档目录的访问权限。请使用仅当前用户可访问的 NTFS 目录。"
        }
        "Cannot restrict state directory permissions." => "无法设置本地存档目录权限。",
        "Cannot create the user state directory." => "无法创建本地存档目录。",
        "State directory must be a regular directory." => "本地存档路径必须是目录。",
        "Another switcher operation is running." => "已有切换操作正在运行，请稍后重试。",
        "Cannot open operation lock." => "无法取得切换操作锁，请检查目录权限。",
        "Version one has a pending write. Recover it with version one before continuing." => {
            "检测到第一版未完成的写入。请先使用第一版恢复，再继续操作。"
        }
        "Invalid encrypted recovery record." => "加密恢复记录无效，工具已停止恢复。",
        "Unsupported recovery record version." => "恢复记录版本不受支持。",
        "Mode vault is missing during recovery." => "恢复时缺少模式密钥库，请先恢复 vault.key。",
        "External changes block recovery. Preserve pending.enc and resolve the conflict." => {
            "检测到外部文件改动，恢复已停止。请保留 pending.enc 并先解决冲突。"
        }
        "Cannot remove completed recovery record." => "无法清除已完成的恢复记录。",
        "Cannot restore missing authentication state." => "无法恢复缺失的认证状态。",
        "Recovery is pending. Initialize modes to recover before previewing." => {
            "检测到未完成的恢复记录。请先执行初始化以恢复，再生成预览。"
        }
        "The preview belongs to a different directory or recovery is pending." => {
            "此预览对应其他配置目录，或存在未完成的恢复记录。请刷新并重新预览。"
        }
        "Files changed since preview. Refresh and preview again." => {
            "预览后文件已发生变化。请刷新并重新生成预览。"
        }
        "Invalid API base URL." => "Base URL 格式无效，请检查协议和主机名。",
        "API base URL must be HTTP(S) with a host and no credentials, query or fragment." => {
            "Base URL 必须使用 HTTP 或 HTTPS，并包含主机名；不能包含认证信息、查询参数或片段。"
        }
        "API key is missing. Enter a key before switching." => "缺少 API key。请输入密钥后再切换。",
        "API key is empty, too long or contains whitespace." => {
            "API key 为空、过长或包含空白字符。请检查输入。"
        }
        "Target authentication is missing. Import a matching auth file." => {
            "目标模式缺少认证。请导入对应模式的认证 JSON。"
        }
        "Current authentication is missing. Import valid authentication first." => {
            "当前活动认证缺失。请先导入有效认证。"
        }
        "Invalid TOML configuration. Original files were not changed." => {
            "TOML 配置无效；原文件未修改。"
        }
        "Invalid authentication JSON. Values are hidden." => "认证 JSON 无效。认证值已隐藏。",
        "Switch failed; previous configuration and authentication were restored." => {
            "切换失败。工具已恢复切换前的配置与认证。"
        }
        "Switch incomplete. Preserve pending.enc, close Codex and initialize to recover." => {
            "切换未完成。请保留 pending.enc，关闭 Codex 并执行初始化以恢复。"
        }
        message => return format!("操作失败：{message}"),
    };
    message.to_string()
}

#[cfg(test)]
mod tests;
