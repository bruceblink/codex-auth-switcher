use eframe::egui;
mod format;
#[cfg(test)]
mod tests;
mod ui;

#[cfg(test)]
use self::format::masked_hint;
use self::format::{mode_name, nonempty, user_error};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Duration,
};
use switcher_core::{
    ImportSources, Mode, Paths, PreparedSwitch, Status, Store, SwitchOptions, SwitchResult,
};
use zeroize::{Zeroize, Zeroizing};

type CoreResult<T> = switcher_core::Result<T>;

#[derive(Clone)]
struct FileDialogs {
    pick_file: Arc<dyn Fn(bool) -> Option<PathBuf> + Send + Sync>,
    pick_folder: Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>,
}

impl FileDialogs {
    fn system() -> Self {
        Self {
            pick_file: Arc::new(|auth| {
                let dialog = rfd::FileDialog::new().set_title(if auth {
                    "导入认证 JSON"
                } else {
                    "导入模式 TOML"
                });
                if auth {
                    dialog.add_filter("JSON", &["json"]).pick_file()
                } else {
                    dialog.add_filter("TOML", &["toml"]).pick_file()
                }
            }),
            pick_folder: Arc::new(|| {
                rfd::FileDialog::new()
                    .set_title("选择 Codex 配置目录")
                    .pick_folder()
            }),
        }
    }
}

enum WorkerOutput {
    Status(CoreResult<Status>),
    Prepared(CoreResult<PreparedSwitch>),
    Applied(CoreResult<(SwitchResult, CoreResult<Status>)>),
    PickFile {
        auth: bool,
        target: Mode,
        path: Option<PathBuf>,
    },
    PickFolder(Option<PathBuf>),
}

/// Keeps file and crypto work on a worker thread while the egui event loop stays responsive.
struct SwitcherApp {
    store: Store,
    demo: bool,
    dialogs: FileDialogs,
    target: Mode,
    status: Option<Status>,
    prepared: Option<PreparedSwitch>,
    directory: String,
    subscription_provider: String,
    api_provider: String,
    endpoint: String,
    key: Zeroizing<String>,
    key_epoch: u64,
    busy: bool,
    message: String,
    error: bool,
    worker: Option<Receiver<WorkerOutput>>,
}

impl SwitcherApp {
    fn new(store: Store, demo: bool) -> Self {
        Self::with_dialogs(store, demo, FileDialogs::system())
    }

    fn with_dialogs(store: Store, demo: bool, dialogs: FileDialogs) -> Self {
        let mut app = Self {
            directory: store.paths.home.to_string_lossy().into_owned(),
            store,
            demo,
            dialogs,
            target: Mode::Api,
            status: None,
            prepared: None,
            subscription_provider: "openai".into(),
            api_provider: "OpenAI".into(),
            endpoint: "https://gateway.example.com/v1".into(),
            key: Zeroizing::new(String::new()),
            key_epoch: 0,
            busy: false,
            message: String::new(),
            error: false,
            worker: None,
        };
        app.refresh();
        app
    }

    fn start_work(&mut self, job: impl FnOnce(Store) -> WorkerOutput + Send + 'static) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = false;
        self.message = "正在处理...".into();
        let store = self.store.clone();
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("codex-auth-switcher-work".into())
            .spawn(move || {
                let _ = sender.send(job(store));
            }) {
            Ok(_) => self.worker = Some(receiver),
            Err(_) => {
                self.busy = false;
                self.message = "无法启动后台操作。请重试。".into();
                self.error = true;
            }
        }
    }

    fn poll_worker(&mut self) {
        let Some(receiver) = self.worker.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(output) => self.complete(output),
            Err(TryRecvError::Empty) => self.worker = Some(receiver),
            Err(TryRecvError::Disconnected) => {
                self.busy = false;
                self.message = "后台操作已中断。请刷新状态后重试。".into();
                self.error = true;
                self.prepared = None;
            }
        }
    }

    fn complete(&mut self, output: WorkerOutput) {
        self.busy = false;
        match output {
            WorkerOutput::Status(result) => match result {
                Ok(status) => {
                    self.target = if status.mode == Some(Mode::Api) {
                        Mode::Subscription
                    } else {
                        Mode::Api
                    };
                    self.sync_profiles(&status);
                    self.message = if status.initialized {
                        "已读取本地状态".into()
                    } else {
                        "尚未初始化模式存档".into()
                    };
                    self.status = Some(status);
                }
                Err(error) => self.show_error(&error),
            },
            WorkerOutput::Prepared(result) => match result {
                Ok(prepared) => {
                    self.message = if prepared.preview.changed {
                        "预览已生成，尚未写入配置".into()
                    } else {
                        "目标与当前配置一致".into()
                    };
                    self.prepared = Some(prepared);
                }
                Err(error) => self.show_error(&error),
            },
            WorkerOutput::Applied(result) => match result {
                Ok((applied, status)) => {
                    self.clear_key();
                    self.message = if applied.changed {
                        format!(
                            "已切换到 {}。重启 Codex 后请新建会话。",
                            mode_name(applied.target)
                        )
                    } else {
                        "配置未变化".into()
                    };
                    match status {
                        Ok(status) => {
                            self.sync_profiles(&status);
                            self.status = Some(status);
                        }
                        Err(error) => {
                            self.status = None;
                            self.message
                                .push_str(&format!(" 状态读取失败：{}", user_error(&error)));
                            self.error = true;
                        }
                    }
                }
                Err(error) => {
                    self.clear_key();
                    self.show_error(&error);
                }
            },
            WorkerOutput::PickFile { auth, target, path } => {
                if let Some(path) = path {
                    let mut sources = ImportSources::default();
                    match (target, auth) {
                        (Mode::Subscription, false) => sources.subscription_config = Some(path),
                        (Mode::Subscription, true) => sources.subscription_auth = Some(path),
                        (Mode::Api, false) => sources.api_config = Some(path),
                        (Mode::Api, true) => sources.api_auth = Some(path),
                    }
                    self.initialize(sources);
                } else {
                    self.message = "文件选择已取消，未修改本地文件。".into();
                }
            }
            WorkerOutput::PickFolder(path) => {
                if let Some(path) = path {
                    self.clear_key();
                    self.directory = path.to_string_lossy().into_owned();
                    self.refresh();
                } else {
                    self.message = "目录选择已取消，未修改本地文件。".into();
                }
            }
        }
    }

    fn show_error(&mut self, error: &switcher_core::Error) {
        self.message = user_error(error);
        self.error = true;
        self.prepared = None;
    }

    fn select_directory(&mut self) -> bool {
        if self.directory.trim().is_empty() {
            self.message = "请选择 Codex 配置目录".into();
            self.error = true;
            return false;
        }
        match Paths::resolve(Some(PathBuf::from(self.directory.trim()))) {
            Ok(paths) => {
                self.store = if self.demo {
                    Store::isolated(paths)
                } else {
                    Store::new(paths)
                };
                true
            }
            Err(error) => {
                self.show_error(&error);
                false
            }
        }
    }

    fn sync_profiles(&mut self, status: &Status) {
        if let Some(profile) = status
            .modes
            .iter()
            .find(|profile| profile.mode == Mode::Subscription)
        {
            self.subscription_provider.clone_from(&profile.provider);
        }
        if let Some(profile) = status
            .modes
            .iter()
            .find(|profile| profile.mode == Mode::Api)
        {
            self.api_provider.clone_from(&profile.provider);
            self.endpoint.clone_from(&profile.base_url);
        }
    }

    fn refresh(&mut self) {
        self.prepared = None;
        if !self.select_directory() {
            return;
        }
        self.status = None;
        self.start_work(|store| WorkerOutput::Status(store.status()));
    }

    fn initialize(&mut self, sources: ImportSources) {
        self.prepared = None;
        if !self.select_directory() {
            return;
        }
        self.start_work(move |store| WorkerOutput::Status(store.initialize(sources)));
    }

    fn preview(&mut self) {
        self.prepared = None;
        if !self.select_directory() {
            return;
        }
        let provider = if self.target == Mode::Api {
            nonempty(&self.api_provider)
        } else {
            nonempty(&self.subscription_provider)
        };
        let options = if self.target == Mode::Api {
            SwitchOptions {
                provider,
                base_url: nonempty(&self.endpoint),
                api_key: nonempty(&self.key).map(Zeroizing::new),
            }
        } else {
            SwitchOptions {
                provider,
                ..Default::default()
            }
        };
        let target = self.target;
        self.start_work(move |store| WorkerOutput::Prepared(store.prepare(target, options)));
    }

    fn apply(&mut self) {
        let Some(prepared) = self.prepared.take() else {
            return;
        };
        self.start_work(move |store| {
            WorkerOutput::Applied(store.apply(prepared).map(|applied| {
                let status = store.status();
                (applied, status)
            }))
        });
    }

    fn clear_key(&mut self) {
        self.key.zeroize();
        self.key = Zeroizing::new(String::new());
        self.key_epoch = self.key_epoch.wrapping_add(1);
        self.prepared = None;
    }

    fn pick_file(&mut self, auth: bool) {
        if self.busy {
            return;
        }
        self.prepared = None;
        self.busy = true;
        self.error = false;
        self.message = "正在打开文件选择器...".into();
        let picker = self.dialogs.pick_file.clone();
        let target = self.target;
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("codex-auth-switcher-file-dialog".into())
            .spawn(move || {
                let _ = sender.send(WorkerOutput::PickFile {
                    auth,
                    target,
                    path: picker(auth),
                });
            }) {
            Ok(_) => self.worker = Some(receiver),
            Err(_) => {
                self.busy = false;
                self.message = "无法打开文件选择器。请重试。".into();
                self.error = true;
            }
        }
    }

    fn pick_folder(&mut self) {
        if self.busy {
            return;
        }
        self.prepared = None;
        self.busy = true;
        self.error = false;
        self.message = "正在打开目录选择器...".into();
        let picker = self.dialogs.pick_folder.clone();
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("codex-auth-switcher-folder-dialog".into())
            .spawn(move || {
                let _ = sender.send(WorkerOutput::PickFolder(picker()));
            }) {
            Ok(_) => self.worker = Some(receiver),
            Err(_) => {
                self.busy = false;
                self.message = "无法打开目录选择器。请重试。".into();
                self.error = true;
            }
        }
    }
}

impl eframe::App for SwitcherApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| self.run_ui(ui));
    }
}

pub fn run(store: Store, demo: bool) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Codex Auth Switcher v2")
            .with_inner_size([920.0, 720.0])
            .with_min_inner_size([720.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Codex Auth Switcher v2",
        options,
        Box::new(move |creation| {
            creation.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::new(SwitcherApp::new(store, demo)))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Cannot open the egui window: {error}"))
}
