use gpui_kit::base::{InteractiveElementExt as _, Scrollbar, ScrollbarMode};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px, rgb,
};
use switcher_core::{Mode, Preview, Status};

use super::{MainTab, Switcher};

impl Render for Switcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let initialized = self
            .status
            .as_ref()
            .is_some_and(|status| status.initialized);
        let recovery_pending = self
            .status
            .as_ref()
            .is_some_and(|status| status.recovery_pending);
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
        let can_restart = crate::restart::supported()
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
        let wide = window.viewport_size().width >= px(900.);

        let mut root = div()
            .id("switcher-root")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.header(cx))
            .child(self.tabs(cx));

        root = match self.tab {
            MainTab::Switch => {
                root.child(self.switch_workspace(wide, initialized, recovery_pending, cx))
            }
            MainTab::Models => root.child(self.catalog_workspace(wide, cx)),
        };

        root = root.child(self.footer(can_preview, can_apply, can_restart, cx));
        if self.restart_confirmation_open {
            root = root.child(self.restart_modal(window, cx));
        }
        root
    }
}

impl Switcher {
    fn header(&self, cx: &Context<Self>) -> impl IntoElement {
        let (status_text, status_color) = if self.busy {
            ("处理中", rgb(0x2c71a0))
        } else if self
            .status
            .as_ref()
            .is_some_and(|status| status.recovery_pending)
        {
            ("需要恢复", rgb(0xa36624))
        } else if self.directory_needs_refresh {
            ("等待刷新", rgb(0xa36624))
        } else if self.status.is_none() {
            ("读取中", rgb(0x6b7280))
        } else if self
            .status
            .as_ref()
            .is_some_and(|status| !status.initialized)
        {
            ("未初始化", rgb(0xa36624))
        } else {
            ("已就绪", rgb(0x2f7d54))
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .size_9()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0xe5f3ed))
                    .text_color(rgb(0x167454))
                    .child(Icon::new(IconName::Replace).size_5()),
            )
            .child(
                div()
                    .flex_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Codex Auth Switcher"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("认证与模型配置"),
                    ),
            )
            .child(pill(status_text, status_color.into()))
            .child(pill(
                concat!("v", env!("CARGO_PKG_VERSION")),
                cx.theme().muted_foreground,
            ))
    }

    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_6()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("switch-tab")
                    .compact()
                    .icon(IconName::Replace)
                    .label("切换配置")
                    .accessibility_label("切换配置")
                    .selected(self.tab == MainTab::Switch)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tab = MainTab::Switch;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("models-tab")
                    .compact()
                    .icon(IconName::BookOpen)
                    .label("模型目录")
                    .accessibility_label("模型目录")
                    .selected(self.tab == MainTab::Models)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tab = MainTab::Models;
                        cx.notify();
                    })),
            )
    }

    fn switch_workspace(
        &self,
        wide: bool,
        initialized: bool,
        recovery_pending: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let directory = Input::new(&self.directory)
            .id("directory-input")
            .aria_label("Codex 配置目录")
            .disabled(self.busy || self.catalog.dirty)
            .cleanable(true);
        let provider = if self.target == Mode::Api {
            &self.api_provider
        } else {
            &self.subscription_provider
        };
        let provider_input = Input::new(provider)
            .id("provider-input")
            .aria_label("Provider ID")
            .disabled(self.busy);
        let endpoint_input = Input::new(&self.endpoint)
            .id("endpoint-input")
            .aria_label("Base URL")
            .disabled(self.busy);
        let key_input = Input::new(&self.key)
            .id("api-key-input")
            .aria_label("API key")
            .disabled(self.busy);
        let profile_fields = if self.target == Mode::Api {
            if wide {
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(field(
                        "Provider ID",
                        provider_input,
                        wide,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(field("Base URL", endpoint_input, wide)),
                    )
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(field("Provider ID", provider_input, wide))
                    .child(field("Base URL", endpoint_input, wide))
            }
        } else {
            div().child(field("Provider ID", provider_input, wide))
        };
        let mode_cards = if wide {
            div()
                .flex()
                .gap_3()
                .child(self.mode_card(Mode::Subscription, cx))
                .child(self.mode_card(Mode::Api, cx))
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(self.mode_card(Mode::Subscription, cx))
                .child(self.mode_card(Mode::Api, cx))
        };
        let content = div()
            .id("switch-workspace")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.switch_scroll)
            .lock_scroll_axis()
            .px_6()
            .py_5()
            .child(section_label("配置目录", cx))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(directory))
                    .child(
                        Button::new("browse")
                            .icon(IconName::FolderOpen)
                            .tooltip("选择配置目录")
                            .accessibility_label("选择目录")
                            .disabled(self.busy || self.catalog.dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))),
                    )
                    .child(
                        Button::new("refresh")
                            .icon(IconName::RefreshCw)
                            .tooltip("刷新本地状态")
                            .accessibility_label("刷新状态")
                            .disabled(self.busy || self.catalog.dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    ),
            )
            .when(self.directory_needs_refresh, |view| {
                view.child(hint("配置目录已修改，请先刷新状态后再预览切换。", false))
            })
            .when(self.catalog.dirty, |view| {
                view.child(hint("模型目录有未保存改动；保存或放弃改动后才能切换配置目录。", false))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mt_6()
                    .mb_3()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("模式存档"))
                    .child(div().flex_1())
                    .child(
                        Button::new("initialize")
                            .compact()
                            .icon(IconName::Inbox)
                            .label(if initialized { "补齐模式" } else { "初始化模式" })
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.initialize(Default::default(), window, cx)
                            })),
                    ),
            )
            .when(recovery_pending, |view| {
                view.child(hint("检测到上次切换未完成。请先点击“补齐模式”恢复，再预览或应用切换。", true))
            })
            .child(mode_cards)
            .when_some(self.status.as_ref(), |view, status| {
                view.child(status_summary(status, cx))
            })
            .child(
                div()
                    .mt_6()
                    .pt_5()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .mb_4()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("目标模式"))
                            .child(div().flex_1())
                            .child(
                                Button::new("import-config")
                                    .compact()
                                    .ghost()
                                    .icon(IconName::FileText)
                                    .label("导入配置 TOML")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("import-auth")
                                    .compact()
                                    .ghost()
                                    .icon(IconName::User)
                                    .label("导入认证 JSON")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import(true, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mb_3()
                            .child(
                                Button::new("subscription")
                                    .compact()
                                    .icon(IconName::CircleUser)
                                    .label("订阅")
                                    .selected(self.target == Mode::Subscription)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.target = Mode::Subscription;
                                        this.prepared = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("api")
                                    .compact()
                                    .icon(IconName::Globe)
                                    .label("API key")
                                    .selected(self.target == Mode::Api)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.target = Mode::Api;
                                        this.prepared = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(profile_fields)
                    .when(self.target == Mode::Api, |view| {
                        view.child(div().mt_3().child(field("API key", key_input, wide)))
                            .child(
                                div()
                                    .mt_1()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(super::format::masked_hint(
                                        self.key.read(cx).value().as_ref(),
                                    )),
                            )
                            .child(hint("留空时使用目标模式的已保存认证；输入新 key 后会在应用时保存到当前用户环境。", false))
                    })
                    .when_some(self.prepared.as_ref(), |view, prepared| {
                        view.child(preview_view(&prepared.preview, cx))
                    }),
            );
        div()
            .id("switch-workspace-container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(content)
            .child(scrollbar_overlay(
                "switch-workspace-scrollbar",
                &self.switch_scroll,
            ))
    }

    fn mode_card(&self, mode: Mode, cx: &Context<Self>) -> impl IntoElement {
        let saved = self
            .status
            .as_ref()
            .and_then(|status| status.modes.iter().find(|profile| profile.mode == mode));
        let active = self
            .status
            .as_ref()
            .is_some_and(|status| status.mode == Some(mode));
        let ready = saved.is_some_and(|profile| profile.ready);
        div()
            .flex_1()
            .min_w_0()
            .p_4()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_3()
                    .child(
                        Icon::new(if mode == Mode::Api {
                            IconName::Globe
                        } else {
                            IconName::CircleUser
                        })
                        .size_4(),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(super::format::mode_name(mode)),
                    )
                    .child(div().flex_1())
                    .when(active, |row| row.child(pill("当前", rgb(0x167454).into())))
                    .child(pill(
                        if ready { "已就绪" } else { "待补齐" },
                        if ready {
                            rgb(0x167454).into()
                        } else {
                            rgb(0x9b6916).into()
                        },
                    )),
            )
            .child(detail(
                "Provider",
                saved
                    .map(|profile| profile.provider.as_str())
                    .unwrap_or("-"),
                cx,
            ))
            .child(detail(
                "模型",
                saved.map(|profile| profile.model.as_str()).unwrap_or("-"),
                cx,
            ))
            .child(detail(
                "推理级别",
                saved.map(|profile| profile.effort.as_str()).unwrap_or("-"),
                cx,
            ))
            .when_some(saved, move |card, saved| {
                card.child(
                    div()
                        .mt_3()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "配置：{}",
                            super::format::config_source_name(&saved.config_source)
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "认证：{}",
                            if saved.auth_source.is_empty() {
                                "缺失"
                            } else {
                                saved.auth_source.as_str()
                            }
                        )),
                )
                .when(!saved.issue.is_empty(), |card| {
                    let issue = if saved.issue
                        == "Target authentication is missing. Import a matching auth file."
                    {
                        if mode == Mode::Api {
                            "尚未保存 API key，请输入密钥或导入认证。".to_string()
                        } else {
                            "尚未保存订阅认证，请在 Codex 登录或导入认证。".to_string()
                        }
                    } else {
                        saved.issue.clone()
                    };
                    card.child(
                        div()
                            .mt_2()
                            .text_xs()
                            .text_color(rgb(0xa34d20))
                            .child(issue),
                    )
                })
            })
    }

    fn catalog_workspace(&self, wide: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = self.catalog.catalog.as_ref();
        let catalog_path = catalog.map(|catalog| catalog.path.clone());
        let selected = self.catalog.selected_model;
        let enabled = !self.busy;
        let model_buttons = catalog
            .map(|catalog| {
                catalog
                    .models
                    .iter()
                    .enumerate()
                    .map(|(index, model)| {
                        let is_selected = selected == index;
                        Button::new(format!("catalog-model-{index}"))
                            .compact()
                            .selected(is_selected)
                            .label(format!("{} ({})", model.display_name, model.slug))
                            .accessibility_label(format!("模型 {}", model.display_name))
                            .disabled(!enabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.catalog.select(index, window, cx);
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let editor = if catalog.is_some() {
            let model_list = div()
                .id("catalog-model-list-container")
                .relative()
                .h(px(if wide { 410. } else { 170. }))
                .min_h_0()
                .child(
                    div()
                        .id("catalog-model-list")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.model_list_scroll)
                        .lock_scroll_axis()
                        .children(model_buttons),
                )
                .child(scrollbar_overlay(
                    "catalog-model-list-scrollbar",
                    &self.model_list_scroll,
                ));
            let model_form = div()
                .flex()
                .flex_col()
                .gap_3()
                .child(field(
                    "模型 ID",
                    Input::new(&self.catalog.model_id)
                        .id("model-id")
                        .aria_label("模型 ID")
                        .disabled(!enabled),
                    wide,
                ))
                .child(field(
                    "显示名称",
                    Input::new(&self.catalog.display_name)
                        .id("model-name")
                        .aria_label("显示名称")
                        .disabled(!enabled),
                    wide,
                ))
                .child(field(
                    "可见性",
                    Input::new(&self.catalog.visibility)
                        .id("model-visibility")
                        .aria_label("可见性")
                        .disabled(!enabled),
                    wide,
                ))
                .child(field(
                    "优先级",
                    Input::new(&self.catalog.priority)
                        .id("model-priority")
                        .aria_label("优先级")
                        .disabled(!enabled),
                    wide,
                ))
                .child(field(
                    "默认推理级别",
                    Input::new(&self.catalog.default_reasoning)
                        .id("model-default-reasoning")
                        .aria_label("默认推理级别")
                        .disabled(!enabled),
                    wide,
                ))
                .child(field(
                    "支持的推理级别",
                    Input::new(&self.catalog.supported_reasoning)
                        .id("model-supported-reasoning")
                        .aria_label("支持的推理级别")
                        .disabled(!enabled),
                    wide,
                ))
                .child(hint(
                    "推理级别用英文逗号分隔；保存时检查模型 ID、名称、可见性和推理级别。",
                    false,
                ));
            if wide {
                div()
                    .flex()
                    .gap_4()
                    .child(div().w(px(230.)).flex_shrink_0().child(model_list))
                    .child(div().flex_1().min_w_0().child(model_form))
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(model_list)
                    .child(model_form)
            }
        } else {
            div().flex_1().p_8().child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("尚未读取模型目录。可以读取已配置目录，或选择现有 JSON 文件。若 config.toml 不存在，请先初始化配置。"),
            )
        };
        let content = div()
            .id("catalog-workspace")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.catalog_scroll)
            .lock_scroll_axis()
            .px_6()
            .py_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_4()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Codex 模型目录"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("catalog-choose")
                            .compact()
                            .label("选择 JSON 文件")
                            .disabled(!enabled || self.catalog.dirty)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_model_catalog(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_4()
                    .child(
                        Button::new("catalog-load")
                            .compact()
                            .icon(IconName::RefreshCw)
                            .label(if self.catalog.has_catalog() {
                                "重新读取已配置目录"
                            } else {
                                "读取已配置目录"
                            })
                            .disabled(!enabled || self.catalog.dirty)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.load_catalog(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("catalog-clone")
                            .compact()
                            .icon(IconName::Copy)
                            .label("复制模型")
                            .disabled(!enabled || !self.catalog.has_catalog())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clone_model(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("catalog-save")
                            .compact()
                            .primary()
                            .icon(IconName::Check)
                            .label("保存")
                            .disabled(!enabled || !self.catalog.can_save())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_catalog(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("catalog-discard")
                            .compact()
                            .ghost()
                            .label("放弃")
                            .disabled(!enabled || !self.catalog.dirty)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.discard_catalog(window, cx)),
                            ),
                    ),
            )
            .when(catalog_path.is_some(), |view| {
                view.child(hint(
                    format!("当前模型目录：{}", catalog_path.clone().unwrap_or_default()),
                    false,
                ))
            })
            .when(self.catalog.dirty, |view| {
                view.child(hint("有未保存改动。保存前不能重新读取或切换目录。", true))
            })
            .child(editor);
        div()
            .id("catalog-workspace-container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(content)
            .child(scrollbar_overlay(
                "catalog-workspace-scrollbar",
                &self.catalog_scroll,
            ))
    }

    fn footer(
        &self,
        can_preview: bool,
        can_apply: bool,
        can_restart: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .px_6()
            .py_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(if self.error {
                                rgb(0xad3b32).into()
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(self.message.clone()),
                    )
                    .child(
                        Button::new("preview")
                            .icon(IconName::Eye)
                            .label("预览变更")
                            .disabled(!can_preview)
                            .on_click(cx.listener(|this, _, window, cx| this.preview(window, cx))),
                    )
                    .child(
                        Button::new("apply")
                            .primary()
                            .icon(IconName::Check)
                            .label("应用切换")
                            .disabled(!can_apply)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    )
                    .child(
                        Button::new("restart")
                            .icon(IconName::RefreshCw)
                            .label("重启 Codex")
                            .disabled(!can_restart)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.restart_confirmation_open = true;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("应用前关闭 Codex。应用后重启，并新建会话。"),
            )
    }

    fn restart_modal(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("restart-modal")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x000000).opacity(0.28))
            .child(
                div()
                    .w(px(390.))
                    .p_5()
                    .rounded_lg()
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("确认重启 Codex"),
                    )
                    .child(div().mt_3().text_sm().child(
                        "这会关闭并重新打开 Codex 桌面应用。正在生成的回复会中断，请先保存工作。",
                    ))
                    .child(
                        div()
                            .mt_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("重启后请新建会话，以使用最新配置。"),
                    )
                    .child(
                        div()
                            .mt_5()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel-restart")
                                    .label("取消")
                                    .ghost()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.restart_confirmation_open = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("confirm-restart")
                                    .primary()
                                    .label("确认重启")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.restart(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}

fn section_label(label: &'static str, cx: &Context<Switcher>) -> impl IntoElement {
    div()
        .text_xs()
        .mb_2()
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

fn scrollbar_overlay(id: &'static str, handle: &gpui_kit::ScrollHandle) -> impl IntoElement {
    div().absolute().inset_0().child(
        Scrollbar::vertical(handle)
            .id(id)
            .mode(ScrollbarMode::Always),
    )
}

fn field(label: &'static str, control: impl IntoElement, wide: bool) -> impl IntoElement {
    if wide {
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().w(px(112.)).flex_shrink_0().text_sm().child(label))
            .child(div().min_w_0().flex_1().child(control))
    } else {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_sm().child(label))
            .child(div().min_w_0().w_full().child(control))
    }
}

fn detail(label: &'static str, value: &str, cx: &Context<Switcher>) -> impl IntoElement {
    div()
        .flex()
        .gap_3()
        .py_1()
        .text_sm()
        .child(
            div()
                .w(px(70.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(value.to_string()))
}

fn status_summary(status: &Status, cx: &Context<Switcher>) -> impl IntoElement {
    div()
        .mt_3()
        .p_3()
        .rounded_md()
        .bg(cx.theme().muted)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "当前：{}  ·  {}  ·  {}  ·  {}",
                    status
                        .mode
                        .map(super::format::mode_name)
                        .unwrap_or("尚未登录"),
                    status.provider,
                    super::format::setting_value(&status.model),
                    super::format::setting_value(&status.effort)
                )),
        )
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "上下文：{} · 详细度：{} · 服务层级：{}",
                    super::format::setting_value(&status.context_window),
                    super::format::setting_value(&status.verbosity),
                    super::format::setting_value(&status.service_tier),
                )),
        )
}

fn preview_view(preview: &Preview, cx: &Context<Switcher>) -> impl IntoElement {
    div()
        .mt_5()
        .pt_4()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .mb_3()
                .child("变更预览"),
        )
        .children(preview.changes.iter().map(|change| {
            div()
                .flex()
                .gap_3()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .w(px(165.))
                        .flex_shrink_0()
                        .text_xs()
                        .child(super::format::config_field_name(&change.field).to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(super::format::setting_value(&change.before).to_string()),
                )
                .child(
                    Icon::new(IconName::ArrowRight)
                        .size_3()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .child(super::format::setting_value(&change.after).to_string()),
                )
        }))
        .when(
            preview.changes.is_empty() && !preview.auth_changes,
            |view| view.child(hint("没有检测到配置或认证变化。", false)),
        )
        .child(
            div()
                .mt_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(if preview.auth_changes {
                    format!(
                        "认证：替换为目标认证（{}）",
                        super::format::auth_source_name(&preview.auth_source)
                    )
                } else {
                    "认证：无变化".into()
                }),
        )
}

fn hint(text: impl Into<SharedString>, warning: bool) -> impl IntoElement {
    div()
        .mt_2()
        .p_2()
        .rounded_sm()
        .text_xs()
        .text_color(if warning {
            rgb(0x8e5414)
        } else {
            rgb(0x65707d)
        })
        .bg(if warning {
            rgb(0xfff4e0)
        } else {
            rgb(0xf1f5f8)
        })
        .child(text.into())
}

fn pill(label: impl Into<SharedString>, color: gpui_kit::Hsla) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_xs()
        .text_color(color)
        .bg(color.opacity(0.08))
        .child(label.into())
}
