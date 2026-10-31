use eframe::egui::{self, Color32, RichText, TextEdit};

use super::{
    ImportSources, Mode, PreparedSwitch, ReasoningLevel, SwitcherApp,
    format::{
        auth_source_name, config_field_name, config_source_name, masked_hint, mode_name,
        setting_value,
    },
};

impl SwitcherApp {
    pub(super) fn run_ui(&mut self, ui: &mut egui::Ui) {
        self.poll_worker();
        let initialized = self
            .status
            .as_ref()
            .is_some_and(|status| status.initialized);
        let recovery_pending = self
            .status
            .as_ref()
            .is_some_and(|status| status.recovery_pending);
        let target_ready = self.status.as_ref().is_some_and(|status| {
            status
                .modes
                .iter()
                .find(|profile| profile.mode == self.target)
                .is_some_and(|profile| profile.ready)
        });
        let directory_needs_refresh = self.directory_needs_refresh;
        let enabled = !self.busy;
        let config_missing = self
            .status
            .as_ref()
            .is_some_and(|status| !status.config_exists);

        // Reserve the action area before laying out scrollable content. A long
        // preview must never move the apply button or its result off-screen.
        egui::Panel::bottom("switch-actions")
            .frame(
                egui::Frame::new()
                    .inner_margin(10.0)
                    .fill(ui.visuals().panel_fill),
            )
            .show(ui, |ui| self.switch_actions(ui));

        ui.horizontal_wrapped(|ui| {
            ui.heading("Codex Auth Switcher");
            ui.label("认证与模型配置");
            ui.weak(concat!("v", env!("CARGO_PKG_VERSION")));
            let (status_text, status_color) = if self.busy {
                ("处理中", Color32::from_rgb(44, 113, 160))
            } else if recovery_pending {
                ("需要恢复", Color32::from_rgb(163, 102, 36))
            } else if directory_needs_refresh {
                ("等待刷新", Color32::from_rgb(163, 102, 36))
            } else if !initialized {
                ("未初始化", Color32::from_rgb(163, 102, 36))
            } else if !target_ready {
                ("目标待补齐", Color32::from_rgb(163, 102, 36))
            } else {
                ("已就绪", Color32::from_rgb(47, 125, 84))
            };
            ui.colored_label(status_color, status_text);
            if ui.button("模型目录").clicked() {
                self.catalog_window_open = true;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("workspace-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label("配置目录");
                ui.horizontal(|ui| {
                    let label = ui.label("Codex 目录");
                    // Reserve width for both actions so a long path cannot push them off-screen.
                    let path_width = (ui.available_width() - 180.0).max(120.0);
                    let response = ui
                        .add_enabled(
                            enabled && !self.catalog_dirty,
                            TextEdit::singleline(&mut self.directory).desired_width(path_width),
                        )
                        .labelled_by(label.id);
                    if response.changed() {
                        self.directory_needs_refresh = true;
                        self.status = None;
                        self.prepared = None;
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("选择目录"))
                        .clicked()
                    {
                        self.pick_folder();
                    }
                    if ui
                        .add_enabled(
                            enabled && !self.catalog_dirty,
                            egui::Button::new("刷新状态"),
                        )
                        .clicked()
                    {
                        self.refresh();
                    }
                });
                if directory_needs_refresh {
                    ui.small("配置目录已修改，请点击“刷新状态”读取新目录后再预览切换。");
                } else if self.catalog_dirty {
                    ui.small("模型目录有未保存改动；保存或放弃改动后才能切换配置目录。");
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.strong("模式存档");
                    ui.add_space(8.0);
                    ui.weak("切换时使用对应的参数和认证");
                    let label = if config_missing {
                        "创建默认配置"
                    } else if initialized {
                        "补齐模式"
                    } else {
                        "初始化模式"
                    };
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        self.initialize(ImportSources::default());
                    }
                });
                if recovery_pending {
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgb(255, 244, 224))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.colored_label(
                                Color32::from_rgb(142, 84, 20),
                                "检测到上次切换未完成。请先点击“补齐模式”恢复，再预览或应用切换。",
                            );
                        });
                } else if !initialized && !directory_needs_refresh {
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgb(236, 245, 252))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.colored_label(
                                Color32::from_rgb(38, 91, 125),
                                if config_missing {
                                    "首次使用：创建 Codex 官方默认配置。登录前不会生成认证文件。"
                                } else {
                                    "还没有模式存档。先点击“初始化模式”，再选择目标模式并预览切换。"
                                },
                            );
                        });
                }
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
                                ui.strong(status.mode.map(mode_name).unwrap_or("尚未登录"));
                                ui.label("·");
                                ui.label(&status.provider);
                                ui.label("·");
                                ui.label(setting_value(&status.model));
                                ui.label("·");
                                ui.label(setting_value(&status.effort));
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.weak("上下文");
                                ui.label(setting_value(&status.context_window));
                                ui.label("·");
                                ui.weak("详细度");
                                ui.label(setting_value(&status.verbosity));
                                ui.label("·");
                                ui.weak("服务层级");
                                ui.label(setting_value(&status.service_tier));
                            });
                        })
                        .response
                        .on_hover_text(format!("配置文件：{}", status.config_path));
                }

                ui.add_space(14.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
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
                let target_issue = self
                    .status
                    .as_ref()
                    .and_then(|status| {
                        status
                            .modes
                            .iter()
                            .find(|profile| profile.mode == self.target)
                    })
                    .filter(|profile| !profile.ready)
                    .map(|profile| profile.issue.clone());
                if let Some(issue) = target_issue {
                    let missing_auth = issue == "Target authentication is missing. Import a matching auth file.";
                    let issue = if missing_auth && self.target == Mode::Api {
                        "尚未保存 API key。输入密钥后可预览并应用，或导入已有认证。".to_string()
                    } else if missing_auth {
                        "尚未保存订阅认证。请在 Codex 中完成登录，或导入订阅认证。".to_string()
                    } else {
                        format!("目标模式还不能切换：{issue}")
                    };
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgb(255, 244, 224))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.colored_label(
                                Color32::from_rgb(142, 84, 20),
                                issue,
                            );
                        });
                }
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
                        let key_width = (ui.available_width() - 62.0).max(140.0);
                        let response = ui
                            .add_enabled(
                                enabled,
                                TextEdit::singleline(&mut *self.key)
                                    .id_salt(("api-key", self.key_epoch))
                                    .password(true)
                                    .desired_width(key_width),
                            )
                            .labelled_by(label.id);
                        if response.changed() {
                            self.prepared = None;
                        }
                        if ui
                            .add_enabled(enabled && !self.key.is_empty(), egui::Button::new("清空"))
                            .clicked()
                        {
                            self.clear_key();
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
                        .max_col_width((ui.available_width() - 32.0) / 3.0)
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("字段");
                            ui.strong("当前");
                            ui.strong("目标");
                            ui.end_row();
                            for change in &prepared.preview.changes {
                                ui.add(egui::Label::new(config_field_name(&change.field)).wrap());
                                ui.add(egui::Label::new(setting_value(&change.before)).wrap());
                                ui.add(egui::Label::new(setting_value(&change.after)).wrap());
                                ui.end_row();
                            }
                        });
                    if prepared.preview.changes.is_empty() && !prepared.preview.auth_changes {
                        ui.weak("没有检测到配置或认证变化。");
                    }
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
        if self.restart_confirmation_open {
            let response = egui::Modal::new(egui::Id::unique("restart-codex-confirmation"))
                .show(ui.ctx(), |ui| {
                    ui.set_width(360.0);
                    ui.heading("确认重启 Codex");
                    ui.label("这会关闭并重新打开 Codex 所在的桌面应用。正在生成的回复会中断，请先保存工作。");
                    ui.label("重启不会应用尚未确认的输入。");
                    ui.label("重启后请新建会话，以使用最新的配置。");
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            self.restart_confirmation_open = false;
                        }
                        if ui.button("确认重启").clicked() {
                            self.restart_codex();
                        }
                    });
                });
            if response.should_close() {
                self.restart_confirmation_open = false;
            }
        }
    }

    fn switch_actions(&mut self, ui: &mut egui::Ui) {
        let can_preview = !self.busy
            && !self.directory_needs_refresh
            && self
                .status
                .as_ref()
                .is_some_and(|status| status.initialized && !status.recovery_pending);
        let can_apply = !self.busy
            && self
                .prepared
                .as_ref()
                .is_some_and(|prepared| prepared.preview.changed);
        ui.horizontal(|ui| {
            ui.strong(format!("目标：{}", mode_name(self.target)));
            if self.restart_needed {
                ui.colored_label(Color32::from_rgb(142, 84, 20), "等待重启");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_restart = crate::restart::supported()
                    && !self.busy
                    && !self.catalog_dirty
                    && !self.directory_needs_refresh
                    && self.key.is_empty()
                    && self
                        .prepared
                        .as_ref()
                        .is_none_or(|prepared| !prepared.preview.changed)
                    && self
                        .status
                        .as_ref()
                        .is_some_and(|status| status.config_exists && !status.recovery_pending);
                if ui
                    .add_enabled(can_restart, egui::Button::new("重启 Codex"))
                    .on_hover_text(if crate::restart::supported() {
                        "关闭并重新打开 Codex 所在的桌面应用。请先保存工作。"
                    } else {
                        "当前系统暂不支持自动重启，请手动关闭并重新打开 Codex。"
                    })
                    .clicked()
                {
                    self.restart_confirmation_open = true;
                }
                if ui
                    .add_enabled(can_apply, egui::Button::new("应用切换"))
                    .clicked()
                {
                    self.apply();
                }
                if ui
                    .add_enabled(can_preview, egui::Button::new("预览变更"))
                    .clicked()
                {
                    self.preview();
                }
            });
        });
        ui.horizontal(|ui| {
            if self.busy {
                ui.spinner();
            }
            let color = if self.error {
                Color32::from_rgb(173, 59, 50)
            } else {
                ui.visuals().text_color()
            };
            ui.add(egui::Label::new(RichText::new(&self.message).color(color)).wrap());
        });
        ui.small("应用前关闭 Codex。应用后重启，并新建会话。");
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
                ui.horizontal_wrapped(|ui| {
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
                    mode_card_field(ui, "Provider", setting_value(&saved.provider));
                    mode_card_field(ui, "模型", setting_value(&saved.model));
                    mode_card_field(ui, "推理级别", setting_value(&saved.effort));
                    if !saved.base_url.is_empty() {
                        mode_card_field(ui, "Base URL", &saved.base_url);
                    }
                    ui.push_id(mode_name(mode), |ui| {
                        ui.collapsing("参数与来源", |ui| {
                            mode_card_field(ui, "上下文窗口", setting_value(&saved.context_window));
                            mode_card_field(ui, "模型详细度", setting_value(&saved.verbosity));
                            mode_card_field(ui, "服务层级", setting_value(&saved.service_tier));
                            mode_card_field(
                                ui,
                                "配置来源",
                                config_source_name(&saved.config_source),
                            );
                            mode_card_field(
                                ui,
                                "认证来源",
                                if saved.auth_source.is_empty() {
                                    "缺失"
                                } else {
                                    auth_source_name(&saved.auth_source)
                                },
                            );
                        });
                    });
                    if !saved.issue.is_empty() {
                        ui.add_space(4.0);
                        let issue = if saved.issue
                            == "Target authentication is missing. Import a matching auth file."
                        {
                            if mode == Mode::Api {
                                "等待 API key"
                            } else {
                                "等待订阅登录"
                            }
                        } else {
                            &saved.issue
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(issue).color(Color32::from_rgb(163, 77, 32)),
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
                    if ui
                        .add_enabled(
                            enabled && self.catalog_dirty,
                            egui::Button::new("放弃改动"),
                        )
                        .clicked()
                    {
                        self.discard_model_catalog();
                    }
                    ui.add_space(6.0);
                    if self.catalog_dirty {
                        ui.colored_label(
                            Color32::from_rgb(163, 77, 32),
                            "有未保存的改动",
                        );
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
