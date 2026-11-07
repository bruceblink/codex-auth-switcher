use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, Subscription, Window};
use switcher_core::{CodexModel, ModelCatalog, ReasoningLevel};

use super::{Switcher, input};

const MAX_CATALOG_MODELS: usize = 512;

#[derive(Clone, Copy)]
enum ModelField {
    Id,
    Name,
    Visibility,
    Priority,
    DefaultReasoning,
    SupportedReasoning,
}

/// Editable fields are stable GPUI input entities. Rendering only reads them;
/// input subscriptions apply edits to the selected in-memory catalog entry.
pub(crate) struct CatalogEditor {
    pub(crate) catalog: Option<ModelCatalog>,
    baseline: Option<ModelCatalog>,
    reasoning_templates: Vec<Vec<ReasoningLevel>>,
    pub(crate) selected_model: usize,
    pub(crate) dirty: bool,
    pub(crate) configured_path: Entity<InputState>,
    pub(crate) search: Entity<InputState>,
    pub(crate) model_id: Entity<InputState>,
    pub(crate) display_name: Entity<InputState>,
    pub(crate) visibility: Entity<InputState>,
    pub(crate) priority: Entity<InputState>,
    pub(crate) default_reasoning: Entity<InputState>,
    pub(crate) supported_reasoning: Entity<InputState>,
    pub(crate) priority_valid: bool,
    _subscriptions: Vec<Subscription>,
}

impl CatalogEditor {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Switcher>) -> Self {
        let configured_path = input("Codex 内置模型目录", false, 4096, window, cx);
        let search = input("搜索模型名称或 ID", false, 128, window, cx);
        let model_id = input("model-id", false, 128, window, cx);
        let display_name = input("模型名称", false, 120, window, cx);
        let visibility = input("list", false, 32, window, cx);
        let priority = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("0")
                .validate(|value, _| {
                    value.is_empty() || value == "-" || value.parse::<i64>().is_ok()
                })
        });
        let default_reasoning = input("high", false, 32, window, cx);
        let supported_reasoning = input("high, medium, low", false, 512, window, cx);
        let mut subscriptions = Vec::new();
        for (entity, field) in [
            (&model_id, ModelField::Id),
            (&display_name, ModelField::Name),
            (&visibility, ModelField::Visibility),
            (&priority, ModelField::Priority),
            (&default_reasoning, ModelField::DefaultReasoning),
            (&supported_reasoning, ModelField::SupportedReasoning),
        ] {
            subscriptions.push(
                cx.subscribe(entity, move |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.catalog.edit(field, cx);
                        this.prepared = None;
                        cx.notify();
                    }
                }),
            );
        }
        subscriptions.push(cx.subscribe(&search, |_, _, _: &InputEvent, cx| {
            cx.notify();
        }));
        Self {
            catalog: None,
            baseline: None,
            reasoning_templates: Vec::new(),
            selected_model: 0,
            dirty: false,
            configured_path,
            search,
            model_id,
            display_name,
            visibility,
            priority,
            default_reasoning,
            supported_reasoning,
            priority_valid: true,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn has_catalog(&self) -> bool {
        self.catalog.is_some()
    }

    pub(crate) fn can_save(&self) -> bool {
        self.catalog.is_some() && self.dirty && self.priority_valid
    }

    pub(crate) fn can_add(&self) -> bool {
        self.has_catalog() && self.model_count() < MAX_CATALOG_MODELS && self.priority_valid
    }

    pub(crate) fn at_model_limit(&self) -> bool {
        self.model_count() >= MAX_CATALOG_MODELS
    }

    pub(crate) fn model_count(&self) -> usize {
        self.catalog
            .as_ref()
            .map(|catalog| catalog.models.len())
            .unwrap_or(0)
    }

    pub(crate) fn load(
        &mut self,
        catalog: ModelCatalog,
        window: &mut Window,
        cx: &mut Context<Switcher>,
    ) {
        let path_changed = self
            .catalog
            .as_ref()
            .is_none_or(|current| current.path != catalog.path);
        self.configured_path
            .update(cx, |state, cx| state.set_value(&catalog.path, window, cx));
        let selected_slug = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.models.get(self.selected_model))
            .map(|model| model.slug.as_str());
        let previous_index = self.selected_model;
        self.selected_model = selected_slug
            .and_then(|slug| catalog.models.iter().position(|model| model.slug == slug))
            .unwrap_or_else(|| previous_index.min(catalog.models.len().saturating_sub(1)));
        self.baseline = Some(catalog.clone());
        self.reasoning_templates = catalog
            .models
            .iter()
            .map(|model| model.supported_reasoning_levels.clone())
            .collect();
        self.catalog = Some(catalog);
        self.dirty = false;
        self.priority_valid = true;
        if path_changed {
            self.search
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.sync_fields(window, cx);
    }

    pub(crate) fn select(
        &mut self,
        selected_model: usize,
        window: &mut Window,
        cx: &mut Context<Switcher>,
    ) {
        if !self.priority_valid
            || selected_model >= self.model_count()
            || selected_model == self.selected_model
        {
            return;
        }
        self.selected_model = selected_model;
        self.priority_valid = true;
        self.sync_fields(window, cx);
    }

    pub(crate) fn clear_loaded(&mut self, window: &mut Window, cx: &mut Context<Switcher>) {
        self.catalog = None;
        self.baseline = None;
        self.reasoning_templates.clear();
        self.selected_model = 0;
        self.dirty = false;
        self.priority_valid = true;
        for entity in [
            &self.configured_path,
            &self.search,
            &self.model_id,
            &self.display_name,
            &self.visibility,
            &self.priority,
            &self.default_reasoning,
            &self.supported_reasoning,
        ] {
            entity.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    pub(crate) fn discard_edits(&mut self, window: &mut Window, cx: &mut Context<Switcher>) {
        if let Some(baseline) = self.baseline.clone() {
            self.load(baseline, window, cx);
        }
    }

    pub(crate) fn clone_selected(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Switcher>,
    ) -> bool {
        if !self.can_add() {
            return false;
        }
        let Some(catalog) = self.catalog.as_mut() else {
            return false;
        };
        let source_index = self.selected_model;
        let Some(mut model) = catalog
            .models
            .get(source_index)
            .or_else(|| catalog.models.first())
            .cloned()
        else {
            return false;
        };
        let Some(slug) = (1..=catalog.models.len().saturating_add(1)).find_map(|suffix| {
            let candidate = format!("new-model-{suffix}");
            (!catalog
                .models
                .iter()
                .any(|existing| existing.slug == candidate))
            .then_some(candidate)
        }) else {
            return false;
        };
        let reasoning_template = self
            .reasoning_templates
            .get(source_index)
            .cloned()
            .unwrap_or_else(|| model.supported_reasoning_levels.clone());
        model.slug = slug;
        model.display_name = "新模型".into();
        model.priority = catalog
            .models
            .iter()
            .map(|item| item.priority)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        catalog.models.push(model);
        self.reasoning_templates.push(reasoning_template);
        self.selected_model = catalog.models.len() - 1;
        self.dirty = true;
        self.priority_valid = true;
        self.sync_fields(window, cx);
        true
    }

    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Switcher>) {
        let Some(model) = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.models.get(self.selected_model))
        else {
            return;
        };
        let fields = [
            (&self.model_id, model.slug.clone()),
            (&self.display_name, model.display_name.clone()),
            (&self.visibility, model.visibility.clone()),
            (&self.priority, model.priority.to_string()),
            (
                &self.default_reasoning,
                model.default_reasoning_level.clone(),
            ),
            (&self.supported_reasoning, reasoning_levels_text(model)),
        ];
        for (entity, value) in fields {
            entity.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        self.priority_valid = true;
    }

    fn edit(&mut self, field: ModelField, cx: &Context<Switcher>) {
        let Some(catalog) = self.catalog.as_mut() else {
            return;
        };
        let Some(model) = catalog.models.get_mut(self.selected_model) else {
            return;
        };
        match field {
            ModelField::Id => {
                let value = self.model_id.read(cx).value().to_string();
                model.slug = value;
            }
            ModelField::Name => {
                let value = self.display_name.read(cx).value().to_string();
                model.display_name = value;
            }
            ModelField::Visibility => {
                let value = self.visibility.read(cx).value().to_string();
                model.visibility = value;
            }
            ModelField::Priority => {
                let value = self.priority.read(cx).value().to_string();
                self.priority_valid = value.parse::<i64>().is_ok();
                if let Ok(value) = value.parse::<i64>() {
                    model.priority = value;
                }
            }
            ModelField::DefaultReasoning => {
                let value = self.default_reasoning.read(cx).value().to_string();
                model.default_reasoning_level = value;
            }
            ModelField::SupportedReasoning => {
                let value = self.supported_reasoning.read(cx).value().to_string();
                if reasoning_levels_text(model) != value
                    && let Some(template) = self.reasoning_templates.get_mut(self.selected_model)
                {
                    update_reasoning_levels(model, &value, template);
                }
            }
        }
        // An unfinished numeric input is not yet represented in the model.
        // Keep it dirty until the user corrects it or discards the draft.
        self.dirty = !self.priority_valid
            || self
                .baseline
                .as_ref()
                .is_none_or(|baseline| catalog.models != baseline.models);
    }
}

impl Switcher {
    pub(crate) fn load_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.catalog.dirty {
            self.message = "有未保存的模型目录改动。请先保存或放弃，再重新读取。".into();
            self.error = true;
            cx.notify();
            return;
        }
        if !self.select_directory(window, cx) {
            return;
        }
        self.execute(
            window,
            cx,
            |store| store.load_model_catalog(),
            |this, catalog, window, cx| {
                this.catalog.load(catalog, window, cx);
                this.message = "已读取 Codex 模型目录".into();
                this.error = false;
            },
        );
    }

    pub(crate) fn choose_model_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.catalog.dirty {
            self.message = "有未保存的模型目录改动。请先保存或放弃，再选择文件。".into();
            self.error = true;
            cx.notify();
            return;
        }
        if !self.select_directory(window, cx) {
            return;
        }

        self.busy = true;
        self.error = false;
        self.message = "正在打开模型目录 JSON 选择器...".into();
        cx.notify();
        let picker = self.file_dialogs.pick_model_catalog.clone();
        cx.spawn_in(window, async move |this, cx| {
            let selected = picker().await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                if let Some(path) = selected {
                    this.execute(
                        window,
                        cx,
                        move |store| {
                            let catalog = store.configure_model_catalog(&path)?;
                            Ok((catalog, store.status()))
                        },
                        |this, (catalog, status), window, cx| {
                            let path = catalog.path.clone();
                            this.catalog.load(catalog, window, cx);
                            this.prepared = None;
                            this.restart_needed = true;
                            this.message = format!(
                                "已将模型目录关联到 config.toml：{path}。重启 Codex 后生效。"
                            );
                            this.error = false;
                            match status {
                                Ok(status) => {
                                    this.status = Some(status);
                                }
                                Err(error) => {
                                    this.status = None;
                                    this.message.push_str(&format!(
                                        "。状态读取失败：{}",
                                        super::format::user_error(&error)
                                    ));
                                    this.error = true;
                                }
                            }
                        },
                    );
                } else {
                    this.message = "文件选择已取消，未修改配置。".into();
                    this.error = false;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn clone_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if !self.catalog.priority_valid {
            self.message = "优先级必须是整数。请修正或放弃当前输入后再复制模型。".into();
            self.error = true;
            cx.notify();
            return;
        }
        if !self.catalog.clone_selected(window, cx) {
            self.message = if self.catalog.at_model_limit() {
                "模型目录最多支持 512 个模型，无法继续复制。".into()
            } else {
                "无法复制模型。请先打开有效的模型目录。".into()
            };
            self.error = true;
            cx.notify();
            return;
        }
        self.message = "已复制模型条目。请确认新模型的能力参数。".into();
        self.error = false;
        cx.notify();
    }

    pub(crate) fn add_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.catalog.has_catalog() {
            return;
        }
        if !self.catalog.priority_valid {
            self.message = "优先级必须是整数。请修正或放弃当前输入后再新增模型。".into();
            self.error = true;
            cx.notify();
            return;
        }
        if !self.catalog.clone_selected(window, cx) {
            self.message = if self.catalog.at_model_limit() {
                "模型目录最多支持 512 个模型，无法新增。".into()
            } else {
                "无法新增模型。请先打开有效的模型目录。".into()
            };
            self.error = true;
            cx.notify();
            return;
        }
        self.catalog
            .search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.message = "已新增模型草稿。能力参数沿用所选模型，请核对后保存到当前 JSON。".into();
        self.error = false;
        cx.notify();
    }

    pub(crate) fn choose_model_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.catalog.dirty {
            return;
        }
        self.busy = true;
        self.error = false;
        self.message = "请选择要编辑的模型 JSON，打开文件不会改动 Codex 配置。".into();
        cx.notify();
        let picker = self.file_dialogs.pick_model_catalog.clone();
        cx.spawn_in(window, async move |this, cx| {
            let selected = picker().await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                if let Some(path) = selected {
                    this.execute(
                        window,
                        cx,
                        move |store| store.open_model_catalog(&path),
                        |this, catalog, window, cx| {
                            this.catalog.load(catalog, window, cx);
                            this.message =
                                "已打开模型 JSON。编辑后保存到该文件；Codex 配置未修改。".into();
                            this.error = false;
                        },
                    );
                } else {
                    this.message = "已取消打开文件，未修改配置或模型。".into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn link_model_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.catalog.dirty {
            return;
        }
        let Some(catalog) = self.catalog.catalog.as_ref() else {
            return;
        };
        let path = std::path::PathBuf::from(&catalog.path);
        self.execute(
            window,
            cx,
            move |store| {
                let catalog = store.configure_model_catalog(&path)?;
                Ok((catalog, store.status()))
            },
            |this, (catalog, status), window, cx| {
                this.catalog.load(catalog, window, cx);
                this.prepared = None;
                this.restart_needed = true;
                this.message = "已将当前 JSON 关联到当前 Codex 模式。重启 Codex 后生效。".into();
                match status {
                    Ok(status) => this.status = Some(status),
                    Err(error) => {
                        this.status = None;
                        this.error = true;
                        this.message.push_str(&super::format::user_error(&error));
                    }
                }
            },
        );
    }

    pub(crate) fn save_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.catalog.priority_valid {
            self.message = "优先级必须是整数。请修正后再保存。".into();
            self.error = true;
            cx.notify();
            return;
        }
        let Some(catalog) = self.catalog.catalog.clone() else {
            return;
        };
        self.execute(
            window,
            cx,
            move |store| {
                let linked = catalog.is_linked();
                let saved = store.save_model_catalog(catalog)?;
                let status = linked.then(|| store.status());
                Ok((saved, status))
            },
            |this, ((catalog, saved), status), window, cx| {
                let linked = catalog.is_linked();
                this.catalog.load(catalog, window, cx);
                this.restart_needed |= linked && saved.backup.is_some();
                this.message = match saved.backup {
                    Some(_) => format!("已保存 {} 个模型，已创建备份。", saved.model_count),
                    None => "模型目录没有变化".into(),
                };
                this.error = false;
                match status {
                    Some(Ok(status)) => this.status = Some(status),
                    Some(Err(error)) => {
                        this.status = None;
                        this.message.push_str(&format!(
                            "。状态读取失败：{}",
                            super::format::user_error(&error)
                        ));
                        this.error = true;
                    }
                    None => {}
                }
            },
        );
    }

    pub(crate) fn discard_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.catalog.discard_edits(window, cx);
        self.message = "已放弃未保存改动，模型目录已恢复到上次读取状态。".into();
        self.error = false;
        cx.notify();
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

fn update_reasoning_levels(model: &mut CodexModel, text: &str, template: &mut Vec<ReasoningLevel>) {
    let old_levels = std::mem::take(&mut model.supported_reasoning_levels);
    let mut next = Vec::new();
    for effort in text
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // Removing and retyping a level must not discard its saved metadata.
        let existing = old_levels
            .iter()
            .find(|level| level.effort == effort)
            .or_else(|| template.iter().find(|level| level.effort == effort));
        if let Some(existing) = existing {
            next.push(existing.clone());
        } else {
            next.push(ReasoningLevel::new(effort.to_string(), String::new()));
        }
    }
    for level in old_levels.iter().chain(&next) {
        if !template
            .iter()
            .any(|existing| existing.effort == level.effort)
        {
            template.push(level.clone());
        }
    }
    model.supported_reasoning_levels = next;
}
