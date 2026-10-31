use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, Subscription, Window};
use switcher_core::{CodexModel, ModelCatalog, ReasoningLevel};

use super::{Switcher, input};

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
    pub(crate) selected_model: usize,
    pub(crate) dirty: bool,
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
        Self {
            catalog: None,
            selected_model: 0,
            dirty: false,
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
        self.catalog = Some(catalog);
        self.selected_model = 0;
        self.dirty = false;
        self.priority_valid = true;
        self.sync_fields(window, cx);
    }

    pub(crate) fn select(
        &mut self,
        selected_model: usize,
        window: &mut Window,
        cx: &mut Context<Switcher>,
    ) {
        if selected_model >= self.model_count() || selected_model == self.selected_model {
            return;
        }
        self.selected_model = selected_model;
        self.priority_valid = true;
        self.sync_fields(window, cx);
    }

    pub(crate) fn clear_loaded(&mut self, window: &mut Window, cx: &mut Context<Switcher>) {
        self.catalog = None;
        self.selected_model = 0;
        self.dirty = false;
        self.priority_valid = true;
        for entity in [
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

    pub(crate) fn clone_selected(&mut self, window: &mut Window, cx: &mut Context<Switcher>) {
        let Some(catalog) = self.catalog.as_mut() else {
            return;
        };
        let Some(mut model) = catalog
            .models
            .get(self.selected_model)
            .or_else(|| catalog.models.first())
            .cloned()
        else {
            return;
        };
        let Some(slug) = (1..=catalog.models.len().saturating_add(1)).find_map(|suffix| {
            let candidate = format!("new-model-{suffix}");
            (!catalog
                .models
                .iter()
                .any(|existing| existing.slug == candidate))
            .then_some(candidate)
        }) else {
            return;
        };
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
        self.selected_model = catalog.models.len() - 1;
        self.dirty = true;
        self.priority_valid = true;
        self.sync_fields(window, cx);
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
                if model.slug != value {
                    model.slug = value;
                    self.dirty = true;
                }
            }
            ModelField::Name => {
                let value = self.display_name.read(cx).value().to_string();
                if model.display_name != value {
                    model.display_name = value;
                    self.dirty = true;
                }
            }
            ModelField::Visibility => {
                let value = self.visibility.read(cx).value().to_string();
                if model.visibility != value {
                    model.visibility = value;
                    self.dirty = true;
                }
            }
            ModelField::Priority => {
                let value = self.priority.read(cx).value().to_string();
                self.priority_valid = value.parse::<i64>().is_ok();
                if let Ok(value) = value.parse::<i64>() {
                    if model.priority != value {
                        model.priority = value;
                        self.dirty = true;
                    }
                } else if !value.is_empty() {
                    self.dirty = true;
                }
            }
            ModelField::DefaultReasoning => {
                let value = self.default_reasoning.read(cx).value().to_string();
                if model.default_reasoning_level != value {
                    model.default_reasoning_level = value;
                    self.dirty = true;
                }
            }
            ModelField::SupportedReasoning => {
                let value = self.supported_reasoning.read(cx).value().to_string();
                if reasoning_levels_text(model) != value {
                    update_reasoning_levels(model, &value);
                    self.dirty = true;
                }
            }
        }
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
                        move |store| store.configure_model_catalog(&path),
                        |this, catalog, window, cx| {
                            let path = catalog.path.clone();
                            this.catalog.load(catalog, window, cx);
                            this.prepared = None;
                            this.restart_needed = true;
                            this.message =
                                format!("已选择模型目录：{path}。请重启 Codex 让模型设置生效。");
                            this.error = false;
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
        self.catalog.clone_selected(window, cx);
        self.message = "已复制模型条目。请确认新模型的能力参数。".into();
        self.error = false;
        cx.notify();
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
            move |store| store.save_model_catalog(catalog),
            |this, (catalog, saved), window, cx| {
                this.catalog.load(catalog, window, cx);
                this.restart_needed |= saved.backup.is_some();
                this.message = match saved.backup {
                    Some(backup) => {
                        format!("已保存 {} 个模型选项。备份：{}", saved.model_count, backup)
                    }
                    None => "模型目录没有变化".into(),
                };
                this.error = false;
            },
        );
    }

    pub(crate) fn discard_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.catalog.clear_loaded(window, cx);
        self.message = "已放弃模型目录改动。重新读取后可继续编辑。".into();
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

fn update_reasoning_levels(model: &mut CodexModel, text: &str) {
    let old_levels = std::mem::take(&mut model.supported_reasoning_levels);
    let mut next = Vec::new();
    for effort in text
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(existing) = old_levels.iter().find(|level| level.effort == effort) {
            next.push(existing.clone());
        } else {
            next.push(ReasoningLevel::new(effort.to_string(), String::new()));
        }
    }
    model.supported_reasoning_levels = next;
}
