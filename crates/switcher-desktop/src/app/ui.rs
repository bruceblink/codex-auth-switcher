use eframe::egui::{self, Color32, RichText, TextEdit};

use super::{
    ImportSources, Mode, PreparedSwitch, ReasoningLevel, SwitcherApp,
    format::{auth_source_name, masked_hint, mode_name},
};

impl SwitcherApp {
    pub(super) fn run_ui(&mut self, ui: &mut egui::Ui) {
        self.poll_worker();
        let initialized = self
            .status
            .as_ref()
            .is_some_and(|status| status.initialized);
        let enabled = !self.busy;

        ui.horizontal(|ui| {
            ui.heading("Codex Auth Switcher");
            ui.label("认证与模型配置");
            ui.label("v0.2.0");
            if ui.button("模型目录").clicked() {
                self.catalog_window_open = true;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("workspace-scroll")
            .show(ui, |ui| {
                ui.label("配置目录");
                ui.horizontal(|ui| {
                    let label = ui.label("Codex 目录");
                    // Reserve width for both actions so a long path cannot push them off-screen.
                    let path_width = (ui.available_width() - 180.0).max(120.0);
                    let response = ui
                        .add_enabled(
                            enabled,
                            TextEdit::singleline(&mut self.directory).desired_width(path_width),
                        )
                        .labelled_by(label.id);
                    if response.changed() {
                        self.prepared = None;
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("选择目录"))
                        .clicked()
                    {
                        self.pick_folder();
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("刷新状态"))
                        .clicked()
                    {
                        self.refresh();
                    }
                });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.strong("模式存档");
                    ui.add_space(8.0);
                    let label = if initialized {
                        "补齐模式"
                    } else {
                        "初始化模式"
                    };
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        self.initialize(ImportSources::default());
                    }
                });
                ui.columns(2, |columns| {
                    self.mode_card(&mut columns[0], Mode::Subscription);
                    self.mode_card(&mut columns[1], Mode::Api);
                });
                if let Some(status) = &self.status {
                    ui.small(format!(
                        "当前：{}  ·  {}  ·  {}  ·  {}",
                        status.mode.map(mode_name).unwrap_or("未知"),
                        status.provider,
                        status.model,
                        status.effort
                    ));
                }

                ui.add_space(14.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("目标模式");
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new("订阅").selected(self.target == Mode::Subscription),
                        )
                        .clicked()
                    {
                        self.target = Mode::Subscription;
                        self.prepared = None;
                    }
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new("API key").selected(self.target == Mode::Api),
                        )
                        .clicked()
                    {
                        self.target = Mode::Api;
                        self.prepared = None;
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("导入配置 TOML"))
                        .clicked()
                    {
                        self.pick_file(false);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("导入认证 JSON"))
                        .clicked()
                    {
                        self.pick_file(true);
                    }
                });
                if self.target == Mode::Api {
                    text_field(
                        &mut self.prepared,
                        ui,
                        "Provider ID",
                        &mut self.api_provider,
                        enabled,
                    );
                } else {
                    text_field(
                        &mut self.prepared,
                        ui,
                        "Provider ID",
                        &mut self.subscription_provider,
                        enabled,
                    );
                }
                if self.target == Mode::Api {
                    text_field(
                        &mut self.prepared,
                        ui,
                        "Base URL",
                        &mut self.endpoint,
                        enabled,
                    );
                    ui.horizontal(|ui| {
                        let label = ui.label("API key");
                        let response = ui
                            .add_enabled(
                                enabled,
                                TextEdit::singleline(&mut *self.key)
                                    .id_salt(("api-key", self.key_epoch))
                                    .password(true)
                                    .desired_width(f32::INFINITY),
                            )
                            .labelled_by(label.id);
                        if response.changed() {
                            self.prepared = None;
                        }
                    });
                    ui.small(masked_hint(&self.key));
                }

                if let Some(prepared) = &self.prepared {
                    ui.add_space(14.0);
                    ui.separator();
                    ui.strong("变更预览");
                    egui::Grid::new("preview-grid")
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            for change in &prepared.preview.changes {
                                ui.label(&change.field);
                                ui.label(&change.before);
                                ui.label(&change.after);
                                ui.end_row();
                            }
                        });
                    if prepared.preview.auth_changes {
                        ui.small("认证：替换为目标认证");
                        ui.small(format!(
                            "认证来源：{}",
                            auth_source_name(&prepared.preview.auth_source)
                        ));
                    } else {
                        ui.small("认证：无变化");
                    }
                }
            });

        self.model_catalog_editor(ui.ctx(), enabled);
        ui.separator();
        ui.small("应用前关闭 Codex。应用后重启，并新建会话。");
        ui.horizontal_wrapped(|ui| {
            let color = if self.error {
                Color32::from_rgb(173, 59, 50)
            } else {
                ui.visuals().text_color()
            };
            ui.label(RichText::new(&self.message).color(color));
            if ui
                .add_enabled(enabled && initialized, egui::Button::new("预览变更"))
                .clicked()
            {
                self.preview();
            }
            if ui
                .add_enabled(
                    enabled && self.prepared.is_some(),
                    egui::Button::new("应用切换"),
                )
                .clicked()
            {
                self.apply();
            }
        });
    }

    fn mode_card(&self, ui: &mut egui::Ui, mode: Mode) {
        let saved = self
            .status
            .as_ref()
            .and_then(|status| status.modes.iter().find(|profile| profile.mode == mode));
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_max_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.strong(mode_name(mode));
                ui.add_space(4.0);
                if self
                    .status
                    .as_ref()
                    .is_some_and(|status| status.mode == Some(mode))
                {
                    ui.label("当前");
                }
                if saved.is_some_and(|profile| profile.ready) {
                    ui.label("已就绪");
                } else {
                    ui.label("待补齐");
                }
            });
            for (label, value) in [
                ("Provider", saved.map(|profile| profile.provider.as_str())),
                ("模型", saved.map(|profile| profile.model.as_str())),
                ("推理级别", saved.map(|profile| profile.effort.as_str())),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(format!("{label}："));
                    ui.add(egui::Label::new(value.unwrap_or("-")).wrap());
                });
            }
            if let Some(saved) = saved {
                ui.add(egui::Label::new(format!("配置：{}", saved.config_source)).wrap());
                ui.add(
                    egui::Label::new(format!(
                        "认证：{}",
                        if saved.auth_source.is_empty() {
                            "缺失"
                        } else {
                            &saved.auth_source
                        }
                    ))
                    .wrap(),
                );
                if !saved.issue.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&saved.issue).color(Color32::from_rgb(163, 77, 32)),
                        )
                        .wrap(),
                    );
                }
            }
        });
    }

    fn model_catalog_editor(&mut self, ctx: &egui::Context, enabled: bool) {
        let mut open = self.catalog_window_open;
        egui::Window::new("Codex 模型目录")
            .default_size([760.0, 560.0])
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let load_label = if self.model_catalog.is_some() {
                        "重新读取 models-codex.json"
                    } else {
                        "读取 models-codex.json"
                    };
                    if ui
                        .add_enabled(enabled && !self.catalog_dirty, egui::Button::new(load_label))
                        .clicked()
                    {
                        self.load_model_catalog();
                    }
                    if ui
                        .add_enabled(enabled && self.model_catalog.is_some(), egui::Button::new("复制当前模型"))
                        .clicked()
                    {
                        self.add_model_from_selected();
                    }
                    if ui
                        .add_enabled(
                            enabled && self.catalog_dirty,
                            egui::Button::new("保存模型选项"),
                        )
                        .clicked()
                    {
                        self.save_model_catalog();
                    }
                });

                ui.small("只编辑当前配置目录中的模型选项。新增条目会复制当前模型的功能参数，请确认它符合新模型的实际能力。");
                let mut selected_changed = false;
                let (catalog_slot, selected_slot) =
                    (&mut self.model_catalog, &mut self.selected_model);
                if let Some(catalog) = catalog_slot.as_mut() {
                    ui.add(egui::Label::new(format!("文件：{}", catalog.path)).wrap());
                    let selected_label = catalog
                        .models
                        .get(*selected_slot)
                        .map(|model| format!("{} ({})", model.display_name, model.slug))
                        .unwrap_or_else(|| "选择模型".into());
                    egui::ComboBox::from_label("模型")
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for (index, model) in catalog.models.iter().enumerate() {
                                let response = ui.selectable_value(
                                    selected_slot,
                                    index,
                                    format!("{} ({})", model.display_name, model.slug),
                                );
                                selected_changed |= response.changed();
                            }
                        });

                    if let Some(model) = catalog.models.get_mut(*selected_slot) {
                        if selected_changed {
                            self.reasoning_levels_text = reasoning_levels_text(model);
                        }
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            let label = ui.label("模型 ID");
                            changed |= ui
                                .add_enabled(
                                    enabled,
                                    TextEdit::singleline(&mut model.slug)
                                        .desired_width(f32::INFINITY),
                                )
                                .labelled_by(label.id)
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            let label = ui.label("显示名称");
                            changed |= ui
                                .add_enabled(
                                    enabled,
                                    TextEdit::singleline(&mut model.display_name)
                                        .desired_width(f32::INFINITY),
                                )
                                .labelled_by(label.id)
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            let label = ui.label("可见性");
                            changed |= ui
                                .add_enabled(
                                    enabled,
                                    TextEdit::singleline(&mut model.visibility)
                                        .desired_width(f32::INFINITY),
                                )
                                .labelled_by(label.id)
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("优先级");
                            changed |= ui
                                .add_enabled(enabled, egui::DragValue::new(&mut model.priority))
                                .changed();
                        });
                        egui::ComboBox::from_id_salt(("default-reasoning", *selected_slot))
                            .selected_text(format!("默认推理级别：{}", model.default_reasoning_level))
                            .show_ui(ui, |ui| {
                                for level in &model.supported_reasoning_levels {
                                    changed |= ui
                                        .selectable_value(
                                            &mut model.default_reasoning_level,
                                            level.effort.clone(),
                                            &level.effort,
                                        )
                                        .changed();
                                }
                            });
                        ui.horizontal(|ui| {
                            let label = ui.label("支持的推理级别");
                            let response = ui
                                .add_enabled(
                                    enabled,
                                    TextEdit::singleline(&mut self.reasoning_levels_text)
                                        .desired_width(f32::INFINITY),
                                )
                                .labelled_by(label.id);
                            if response.changed() {
                                update_reasoning_levels(model, &self.reasoning_levels_text);
                                changed = true;
                            }
                        });
                        ui.small("推理级别用英文逗号分隔。保留已有说明；新增级别的说明为空。");
                        if changed {
                            self.catalog_dirty = true;
                        }
                    }
                } else {
                    ui.small("读取模型目录后可编辑模型名称、可见性、优先级和推理级别。");
                }
            });
        self.catalog_window_open = open;
    }
}

fn reasoning_levels_text(model: &switcher_core::CodexModel) -> String {
    model
        .supported_reasoning_levels
        .iter()
        .map(|level| level.effort.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn update_reasoning_levels(model: &mut switcher_core::CodexModel, text: &str) {
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
    if !next
        .iter()
        .any(|level| level.effort == model.default_reasoning_level)
        && let Some(first) = next.first()
    {
        model.default_reasoning_level = first.effort.clone();
    }
    model.supported_reasoning_levels = next;
}

fn text_field(
    prepared: &mut Option<PreparedSwitch>,
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    enabled: bool,
) {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        let response = ui
            .add_enabled(
                enabled,
                TextEdit::singleline(value).desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        if response.changed() {
            *prepared = None;
        }
    });
}
