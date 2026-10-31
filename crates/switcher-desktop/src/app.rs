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
    CodexModel, ImportSources, Mode, ModelCatalog, ModelCatalogSaveResult, Paths, PreparedSwitch,
    ReasoningLevel, Status, Store, SwitchOptions, SwitchResult, config,
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
    CatalogLoaded(CoreResult<ModelCatalog>),
    CatalogSaved(CoreResult<(ModelCatalog, ModelCatalogSaveResult)>),
    PickFile {
        auth: bool,
        target: Mode,
        path: Option<PathBuf>,
    },
    PickFolder(Option<PathBuf>),
    Restarted(CoreResult<()>),
}

/// Keeps file and crypto work on a worker thread while the egui event loop stays responsive.
struct SwitcherApp {
    store: Store,
    dialogs: FileDialogs,
    target: Mode,
    status: Option<Status>,
    prepared: Option<PreparedSwitch>,
    model_catalog: Option<ModelCatalog>,
    selected_model: usize,
    reasoning_levels_text: String,
    catalog_dirty: bool,
    catalog_window_open: bool,
    restart_confirmation_open: bool,
    restart_needed: bool,
    directory: String,
    directory_needs_refresh: bool,
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
    fn new(store: Store) -> Self {
        Self::with_dialogs(store, FileDialogs::system())
    }

    fn with_dialogs(store: Store, dialogs: FileDialogs) -> Self {
        let mut app = Self {
            directory: store.paths.home.to_string_lossy().into_owned(),
            store,
            dialogs,
            target: Mode::Api,
            status: None,
            prepared: None,
            model_catalog: None,
            selected_model: 0,
            reasoning_levels_text: String::new(),
            catalog_dirty: false,
            catalog_window_open: false,
            restart_confirmation_open: false,
            restart_needed: false,
            subscription_provider: "openai".into(),
            api_provider: "openai".into(),
            endpoint: config::DEFAULT_API_BASE_URL.into(),
            directory_needs_refresh: false,
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
                    if self
                        .status
                        .as_ref()
                        .is_some_and(|previous| !previous.config_exists)
                        && status.config_exists
                    {
                        self.restart_needed = true;
                    }
                    self.target = if status.mode == Some(Mode::Api) {
                        Mode::Subscription
                    } else {
                        Mode::Api
                    };
                    self.sync_profiles(&status);
                    self.message = if !status.config_exists {
                        "首次使用：请创建默认配置，再登录 Codex 或输入 API key。".into()
                    } else if status.auth_mode.is_none() {
                        "尚未登录。可输入 API key 并预览应用，或在 Codex 中完成订阅登录。".into()
                    } else if status.initialized {
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
                    if prepared.preview.auth_changes {
                        self.message.push_str(&format!(
                            "。认证来源：{}",
                            format::auth_source_name(&prepared.preview.auth_source)
                        ));
                    }
                    self.prepared = Some(prepared);
                }
                Err(error) => self.show_error(&error),
            },
            WorkerOutput::Applied(result) => match result {
                Ok((applied, status)) => {
                    self.restart_needed |= applied.restart_required;
                    self.clear_key();
                    self.message = if applied.changed {
                        format!(
                            "已切换到 {}。重启 Codex 后请新建会话。",
                            mode_name(applied.target)
                        )
                    } else {
                        "配置未变化".into()
                    };
                    match applied.environment_persisted {
                        Some(true) => {
                            self.message
                                .push_str(" API key 已保存到当前用户环境变量 AUTOCODE_API_KEY。");
                        }
                        Some(false) => {
                            self.message.push_str(
                                " 切换已完成，但无法保存当前用户环境变量 AUTOCODE_API_KEY。",
                            );
                            self.error = true;
                        }
                        None => {}
                    }
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
            WorkerOutput::CatalogLoaded(result) => match result {
                Ok(catalog) => {
                    self.selected_model = 0;
                    self.reasoning_levels_text = catalog
                        .models
                        .first()
                        .map(reasoning_levels_text)
                        .unwrap_or_default();
                    self.model_catalog = Some(catalog);
                    self.catalog_dirty = false;
                    self.message = "已读取 Codex 模型目录".into();
                    self.error = false;
                }
                Err(error) => self.show_error(&error),
            },
            WorkerOutput::CatalogSaved(result) => match result {
                Ok((catalog, saved)) => {
                    self.restart_needed |= saved.backup.is_some();
                    self.selected_model = self.selected_model.min(catalog.models.len() - 1);
                    self.reasoning_levels_text = catalog
                        .models
                        .get(self.selected_model)
                        .map(reasoning_levels_text)
                        .unwrap_or_default();
                    self.model_catalog = Some(catalog);
                    self.catalog_dirty = false;
                    self.message = match saved.backup {
                        Some(backup) => {
                            format!("已保存 {} 个模型选项。备份：{}", saved.model_count, backup)
                        }
                        None => "模型目录没有变化".into(),
                    };
                    self.error = false;
                }
                Err(error) => self.show_error(&error),
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
            WorkerOutput::Restarted(result) => match result {
                Ok(()) => {
                    self.restart_needed = false;
                    self.message = "Codex 桌面应用已重启。请新建会话以使用最新配置。".into();
                }
                Err(error) => self.show_error(&error),
            },
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
                if paths.home != self.store.paths.home {
                    if self.catalog_dirty {
                        self.message =
                            "请先保存或放弃当前模型目录改动，再选择其他配置目录。".into();
                        self.error = true;
                        return false;
                    }
                    self.model_catalog = None;
                    self.reasoning_levels_text.clear();
                    self.selected_model = 0;
                    self.status = None;
                }
                self.store = self.store.with_paths(paths);
                self.directory_needs_refresh = false;
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
        if self.directory_needs_refresh {
            self.message = "配置目录已修改，请先刷新状态后再预览。".into();
            self.error = true;
            return;
        }
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

    fn load_model_catalog(&mut self) {
        if self.catalog_dirty {
            self.message = "有未保存的模型目录改动。请先保存，再重新读取。".into();
            self.error = true;
            return;
        }
        if !self.select_directory() {
            return;
        }
        self.start_work(|store| WorkerOutput::CatalogLoaded(store.load_model_catalog()));
    }

    fn save_model_catalog(&mut self) {
        let Some(catalog) = self.model_catalog.clone() else {
            return;
        };
        self.start_work(move |store| WorkerOutput::CatalogSaved(store.save_model_catalog(catalog)));
    }

    fn add_model_from_selected(&mut self) {
        let Some(catalog) = self.model_catalog.as_mut() else {
            return;
        };
        let Some(template) = catalog
            .models
            .get(self.selected_model)
            .or_else(|| catalog.models.first())
            .cloned()
        else {
            return;
        };
        let mut model: CodexModel = template;
        let base = "new-model";
        let mut suffix = 1usize;
        while catalog
            .models
            .iter()
            .any(|item| item.slug == format!("{base}-{suffix}"))
        {
            suffix += 1;
        }
        model.slug = format!("{base}-{suffix}");
        model.display_name = "新模型".into();
        model.priority = catalog
            .models
            .iter()
            .map(|item| item.priority)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.reasoning_levels_text = reasoning_levels_text(&model);
        catalog.models.push(model);
        self.selected_model = catalog.models.len() - 1;
        self.catalog_dirty = true;
        self.message = "已添加模型条目。请修改模型 ID 和名称，并确认复制来的能力参数。".into();
        self.error = false;
    }

    fn discard_model_catalog(&mut self) {
        self.model_catalog = None;
        self.selected_model = 0;
        self.reasoning_levels_text.clear();
        self.catalog_dirty = false;
        self.message = "已放弃模型目录改动。重新读取后可继续编辑。".into();
        self.error = false;
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

    fn restart_codex(&mut self) {
        self.restart_confirmation_open = false;
        self.start_work(|_| WorkerOutput::Restarted(crate::restart::restart()));
        self.message = "正在关闭并重新启动 Codex 桌面应用...".into();
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
        if self.catalog_dirty {
            self.message = "请先保存模型目录改动，再选择其他配置目录。".into();
            self.error = true;
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

fn reasoning_levels_text(model: &CodexModel) -> String {
    model
        .supported_reasoning_levels
        .iter()
        .map(|level| level.effort.as_str())
        .collect::<Vec<_>>()
        .join(", ")
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

pub fn run(store: Store) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Codex Auth Switcher v2")
            .with_inner_size([920.0, 640.0])
            .with_min_inner_size([720.0, 560.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Codex Auth Switcher v2",
        options,
        Box::new(move |creation| {
            creation.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::new(SwitcherApp::new(store)))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Cannot open the egui window: {error}"))
}
