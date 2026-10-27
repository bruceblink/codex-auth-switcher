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
                    ui.weak("切换时使用对应的参数和认证");
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
                    egui::Frame::group(ui.style())
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.weak("当前使用");
                                ui.strong(status.mode.map(mode_name).unwrap_or("未知"));
                                ui.label("·");
                                ui.label(&status.provider);
                                ui.label("·");
                                ui.label(&status.model);
                                ui.label("·");
                                ui.label(&status.effort);
                            });
                        });
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
                    ui.weak("留空时使用目标模式的已保存认证；输入新 key 后会在应用时保存到当前用户环境。");
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

                ui.add_space(8.0);
                ui.separator();
                ui.small("应用前关闭 Codex。应用后重启，并新建会话。");
                // Keep actions in the scroll region so taller cards cannot clip them below the window.
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
            });

        self.model_catalog_editor(ui.ctx(), enabled);
    }

    fn mode_card(&self, ui: &mut egui::Ui, mode: Mode) {
        let saved = self
            .status
            .as_ref()
            .and_then(|status| status.modes.iter().find(|profile| profile.mode == mode));
        egui::Frame::group(ui.style())
            .inner_margin(10.0)
            .show(ui, |ui| {
                let card_width = ui.available_width();
                ui.set_min_width(card_width);
                ui.set_max_width(card_width);
                ui.horizontal(|ui| {
                    ui.strong(mode_name(mode));
                    ui.add_space(8.0);
                    let is_current = self
                        .status
                        .as_ref()
                        .is_some_and(|status| status.mode == Some(mode));
                    if is_current {
                        ui.colored_label(Color32::from_rgb(44, 113, 160), "当前");
                    }
                    if saved.is_some_and(|profile| profile.ready) {
                        ui.colored_label(Color32::from_rgb(47, 125, 84), "已就绪");
                    } else {
                        ui.colored_label(Color32::from_rgb(163, 102, 36), "待补齐");
                    }
                });
                if let Some(saved) = saved {
                    ui.add_space(6.0);
                    ui.separator();
                    mode_card_field(ui, "Provider", &saved.provider);
                    mode_card_field(ui, "模型", &saved.model);
                    mode_card_field(ui, "推理级别", &saved.effort);
                    mode_card_field(ui, "配置", &saved.config_source);
                    mode_card_field(
                        ui,
                        "认证",
                        if saved.auth_source.is_empty() {
                            "缺失"
                        } else {
                            &saved.auth_source
                        },
                    );
                    if !saved.issue.is_empty() {
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&saved.issue).color(Color32::from_rgb(163, 77, 32)),
                            )
                            .wrap(),
                        );
                    }
                } else {
                    ui.add_space(6.0);
                    ui.weak("尚未保存此模式。初始化或导入配置后，可在这里查看模式参数。");
                }
            });
    }

    fn model_catalog_editor(&mut self, ctx: &egui::Context, enabled: bool) {
        let mut open = self.catalog_window_open;
        egui::Window::new("Codex 模型目录")
            .default_size([780.0, 540.0])
            .min_size([680.0, 460.0])
            .resizable(true)
            .collapsible(false)
            .constrain(true)
            .open(&mut open)
            .show(ctx, |ui| {
                // Preserve a useful editor area even before a catalog is loaded.
                ui.set_min_height(400.0);
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
                    ui.add_space(6.0);
                    if self.catalog_dirty {
                        ui.colored_label(Color32::from_rgb(163, 77, 32), "有未保存的改动");
                    } else if self.model_catalog.is_some() {
                        ui.weak("已读取");
                    }
                });

                egui::Frame::group(ui.style())
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                "这里只修改当前 Codex 目录中的模型选项。复制模型时会保留功能参数，请确认这些参数符合新模型的实际能力。",
                            )
                            .wrap(),
                        );
                    });

                let mut selected_changed = false;
                let (catalog_slot, selected_slot) =
                    (&mut self.model_catalog, &mut self.selected_model);
                if let Some(catalog) = catalog_slot.as_mut() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.weak("文件");
                        let path_response = ui.add_sized(
                            [ui.available_width(), ui.spacing().interact_size.y],
                            egui::Label::new(&catalog.path).truncate(),
                        );
                        path_response.on_hover_text(&catalog.path);
                    });
                    ui.separator();

                    let selected_label = catalog
                        .models
                        .get(*selected_slot)
                        .map(|model| format!("{} ({})", model.display_name, model.slug))
                        .unwrap_or_else(|| "选择模型".into());
                    let editor_width = ui.available_width();
                    egui::ScrollArea::vertical()
                        .id_salt("model-catalog-editor-scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_min_width(editor_width);
                            egui::Frame::group(ui.style())
                                .inner_margin(10.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.strong("模型选项");
                                        ui.weak(format!("{} 个模型", catalog.models.len()));
                                    });
                                    ui.add_space(6.0);

                                    let mut changed = false;
                                    ui.add_enabled_ui(enabled, |ui| {
                                        egui::Grid::new("model-catalog-fields")
                                            .num_columns(2)
                                            .spacing([12.0, 8.0])
                                            .show(ui, |ui| {
                                                let label = ui.label("当前模型");
                                                let response = egui::ComboBox::from_id_salt(
                                                    "model-catalog-selected",
                                                )
                                                    .width(ui.available_width())
                                                    .selected_text(selected_label)
                                                    .show_ui(ui, |ui| {
                                                    for (index, model) in
                                                        catalog.models.iter().enumerate()
                                                    {
                                                        let response = ui.selectable_value(
                                                            selected_slot,
                                                            index,
                                                            format!(
                                                                "{} ({})",
                                                                model.display_name, model.slug
                                                            ),
                                                        );
                                                        selected_changed |= response.changed();
                                                    }
                                                });
                                                response.response.labelled_by(label.id);
                                                ui.end_row();

                                                if let Some(model) =
                                                    catalog.models.get_mut(*selected_slot)
                                                {
                                                    let label = ui.label("模型 ID");
                                                    changed |= ui
                                                        .add(
                                                            TextEdit::singleline(&mut model.slug)
                                                                .desired_width(f32::INFINITY),
                                                        )
                                                        .labelled_by(label.id)
                                                        .changed();
                                                    ui.end_row();

                                                    let label = ui.label("显示名称");
                                                    changed |= ui
                                                        .add(
                                                            TextEdit::singleline(
                                                                &mut model.display_name,
                                                            )
                                                            .desired_width(f32::INFINITY),
                                                        )
                                                        .labelled_by(label.id)
                                                        .changed();
                                                    ui.end_row();

                                                    let label = ui.label("可见性");
                                                    changed |= ui
                                                        .add(
                                                            TextEdit::singleline(
                                                                &mut model.visibility,
                                                            )
                                                            .desired_width(f32::INFINITY),
                                                        )
                                                        .labelled_by(label.id)
                                                        .changed();
                                                    ui.end_row();

                                                    let label = ui.label("优先级");
                                                    changed |= ui
                                                        .add(egui::DragValue::new(
                                                            &mut model.priority,
                                                        ))
                                                        .labelled_by(label.id)
                                                        .changed();
                                                    ui.end_row();

                                                    let label = ui.label("默认推理级别");
                                                    let response = egui::ComboBox::from_id_salt((
                                                        "default-reasoning",
                                                        *selected_slot,
                                                    ))
                                                    .width(ui.available_width())
                                                    .selected_text(
                                                        &model.default_reasoning_level,
                                                    )
                                                    .show_ui(ui, |ui| {
                                                        for level in
                                                            &model.supported_reasoning_levels
                                                        {
                                                            changed |= ui
                                                                .selectable_value(
                                                                    &mut model
                                                                        .default_reasoning_level,
                                                                    level.effort.clone(),
                                                                    &level.effort,
                                                                )
                                                                .changed();
                                                        }
                                                    });
                                                    response.response.labelled_by(label.id);
                                                    ui.end_row();

                                                    let label = ui.label("支持的推理级别");
                                                    let response = ui.add(
                                                        TextEdit::singleline(
                                                            &mut self.reasoning_levels_text,
                                                        )
                                                        .desired_width(f32::INFINITY),
                                                    );
                                                    let response = response.labelled_by(label.id);
                                                    if response.changed() {
                                                        update_reasoning_levels(
                                                            model,
                                                            &self.reasoning_levels_text,
                                                        );
                                                        changed = true;
                                                    }
                                                    ui.end_row();
                                                }
                                            });
                                    });

                                    if selected_changed
                                        && let Some(model) =
                                            catalog.models.get(*selected_slot)
                                    {
                                        self.reasoning_levels_text = reasoning_levels_text(model);
                                    }
                                    if changed {
                                        self.catalog_dirty = true;
                                    }
                                    ui.add_space(6.0);
                                    ui.small("推理级别用英文逗号分隔。已有说明会保留；新增级别的说明为空。");
                                });
                        });
                } else {
                    ui.add_space(8.0);
                    let empty_state_width = ui.available_width();
                    egui::Frame::group(ui.style())
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.set_min_width(empty_state_width);
                            ui.vertical_centered(|ui| {
                                ui.strong("尚未读取模型目录");
                                ui.add_space(4.0);
                                ui.add(
                                    egui::Label::new(
                                        "读取 models-codex.json 后，可编辑模型名称、可见性、优先级和推理级别。",
                                    )
                                    .wrap(),
                                );
                            });
                        });
                }
            });
        self.catalog_window_open = open;
    }
}

fn mode_card_field(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [76.0, ui.spacing().interact_size.y],
            egui::Label::new(RichText::new(label).weak()),
        );
        ui.add(egui::Label::new(value).wrap());
    });
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
