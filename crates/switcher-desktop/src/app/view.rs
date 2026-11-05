use gpui_kit::assets::IconName;
use gpui_kit::base::{InteractiveElementExt as _, Scrollbar, ScrollbarMode};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
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
            && !self.catalog.dirty
            && self
                .prepared
                .as_ref()
                .is_some_and(|prepared| prepared.preview.changed);
        let restart_block_reason = self.restart_block_reason(cx);
        let can_restart = restart_block_reason.is_none();
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
            MainTab::Review => root.child(self.review_workspace(can_apply, cx)),
            MainTab::Models => root.child(self.catalog_workspace(wide, cx)),
            MainTab::Update => root.child(self.update_workspace(cx)),
        };

        if self.tab == MainTab::Update {
            root.child(self.update_footer(cx))
        } else {
            root.child(self.footer(
                can_preview,
                can_apply,
                can_restart,
                restart_block_reason,
                cx,
            ))
        }
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
                    .selected(matches!(self.tab, MainTab::Switch | MainTab::Review))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tab = if this.prepared.is_some() {
                            MainTab::Review
                        } else {
                            MainTab::Switch
                        };
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
            .child(
                Button::new("update-tab")
                    .compact()
                    .icon(IconName::Download)
                    .label(
                        if self.updater.update_attention && self.tab != MainTab::Update {
                            "软件更新 •"
                        } else {
                            "软件更新"
                        },
                    )
                    .accessibility_label(
                        if self.updater.update_attention && self.tab != MainTab::Update {
                            "软件更新，有可用版本"
                        } else {
                            "软件更新"
                        },
                    )
                    .selected(self.tab == MainTab::Update)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tab = MainTab::Update;
                        this.updater.update_attention = false;
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
        let api_key_pending = !self.key.read(cx).value().is_empty();
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
        let mode_cards = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(self.mode_card(Mode::Subscription, cx))
            .child(self.mode_card(Mode::Api, cx));
        let initialize_label = self
            .status
            .as_ref()
            .map(|status| {
                if !status.config_exists {
                    "创建默认配置"
                } else if status.recovery_pending {
                    "恢复模式存档"
                } else if initialized {
                    "补齐模式"
                } else {
                    "初始化模式"
                }
            })
            .unwrap_or("创建默认配置");
        let current_status = self.status.as_ref().map(|status| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w_0().child(status_summary(status, cx)))
                .child(
                    Button::new("initialize")
                        .compact()
                        .icon(IconName::Inbox)
                        .label(initialize_label)
                        .disabled(self.busy || self.catalog.dirty)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.initialize(Default::default(), window, cx)
                        })),
                )
        });
        let target_settings = div()
            .mt_3()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_2()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("目标模式"),
                    )
                    .child(
                        Button::new("import-config")
                            .compact()
                            .ghost()
                            .icon(IconName::FileText)
                            .label("导入配置 TOML")
                            .disabled(self.busy || self.catalog.dirty)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.import(false, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("import-auth")
                            .compact()
                            .ghost()
                            .icon(IconName::User)
                            .label("导入认证 JSON")
                            .disabled(self.busy || self.catalog.dirty)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.import(true, window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_2()
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
                view.child(
                    div().mt_2().child(field(
                        "API key",
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(key_input))
                            .child(self.clear_key_button(api_key_pending, cx)),
                        wide,
                    )),
                )
                .child(
                    div()
                        .ml(px(if wide { 100. } else { 0. }))
                        .mt_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(super::format::masked_hint(
                            self.key.read(cx).value().as_ref(),
                        )),
                )
                .child(hint(
                    "留空时使用已保存的目标认证。新 key 会在应用切换时写入当前用户环境。",
                    false,
                ))
            })
            .when(
                self.target == Mode::Subscription && api_key_pending,
                |view| {
                    view.child(
                        div()
                            .mt_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .child(hint("有未应用的 API key。订阅模式不会使用它。", false)),
                            )
                            .child(self.clear_key_button(api_key_pending, cx)),
                    )
                },
            );
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
            .map(|profile| profile.issue.clone())
            .filter(|issue| !issue.is_empty());
        let content = div()
            .id("switch-workspace")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.switch_scroll)
            .lock_scroll_axis()
            .px_5()
            .py_3()
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
                view.child(hint(
                    "模型目录有未保存改动。请先保存或放弃，再切换配置目录或应用模式切换。",
                    true,
                ))
            })
            .when_some(current_status, |view, summary| view.child(summary))
            .when_some(target_issue, |view, issue| {
                view.child(hint(format!("目标模式暂不可用：{issue}"), true))
            })
            .when(self.status.is_none(), |view| {
                view.child(hint("正在读取当前 Codex 配置状态。", false))
            })
            .when(recovery_pending, |view| {
                view.child(hint(
                    "检测到上次切换未完成。请先恢复模式存档，再预览或应用切换。",
                    true,
                ))
            })
            .child(target_settings)
            .child(
                div()
                    .mt_3()
                    .child(section_label("模式存档", cx))
                    .child(mode_cards),
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
        let summary = saved
            .map(|profile| {
                format!(
                    "{} · {} · {}",
                    super::format::setting_value(&profile.provider),
                    super::format::setting_value(&profile.model),
                    super::format::setting_value(&profile.effort),
                )
            })
            .unwrap_or_else(|| "尚未保存".into());
        let issue = saved
            .filter(|profile| !profile.issue.is_empty())
            .map(|profile| profile.issue.clone());
        div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .py_1()
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
                            .w(px(60.))
                            .text_sm()
                            .child(super::format::mode_name(mode)),
                    )
                    .when(active, |row| row.child(pill("当前", rgb(0x167454).into())))
                    .child(pill(
                        if ready {
                            "已就绪"
                        } else if saved.is_some() {
                            "待补齐"
                        } else {
                            "未保存"
                        },
                        if ready {
                            rgb(0x167454).into()
                        } else {
                            rgb(0x9b6916).into()
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(summary),
                    ),
            )
            .when_some(issue, |card, issue| {
                card.child(hint(format!("无法切换：{issue}"), true))
            })
    }

    fn review_workspace(&self, can_apply: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let target = super::format::mode_name(self.target);
        let directory = self.store.paths.home.to_string_lossy().into_owned();
        let actual_mode = self
            .status
            .as_ref()
            .and_then(|status| status.mode)
            .map(super::format::mode_name)
            .unwrap_or("未识别");
        let actual_mode_color = if actual_mode == "未识别" {
            rgb(0x9b6916)
        } else {
            rgb(0x167454)
        };
        let content = div()
            .id("review-workspace")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.review_scroll)
            .lock_scroll_axis()
            .px_6()
            .py_4()
            .child(
                div().flex().items_center().gap_3().mb_3().child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("确认切换"),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .mb_3()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Codex 配置目录"),
                    )
                    .child(div().min_w_0().text_sm().child(directory))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("当前实际模式"),
                            )
                            .child(pill(actual_mode, actual_mode_color.into()))
                            .child(Icon::new(IconName::ArrowRight).size_3())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("目标模式"),
                            )
                            .child(pill(target, rgb(0x167454).into())),
                    ),
            )
            .child(hint("以下内容来自预览。确认前不会写入 Codex 配置。", false))
            .when(self.catalog.dirty, |view| {
                view.child(hint(
                    "模型目录有未保存改动。请先到模型目录页保存或放弃改动，再应用切换。",
                    true,
                ))
            })
            .when_some(self.prepared.as_ref(), |view, prepared| {
                view.child(preview_view(&prepared.preview, cx))
            })
            .when(self.prepared.is_none(), |view| {
                view.child(hint("预览已失效，请返回配置页重新预览。", true))
            })
            .when(
                self.prepared
                    .as_ref()
                    .is_some_and(|prepared| !prepared.preview.changed),
                |view| view.child(hint("目标配置与当前配置一致，因此无需应用切换。", false)),
            )
            .when(!can_apply && !self.busy && self.catalog.dirty, |view| {
                view.child(hint(
                    "“应用切换”暂不可用。请先保存或放弃模型目录改动。",
                    true,
                ))
            });
        div()
            .id("review-workspace-container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(content)
            .child(scrollbar_overlay(
                "review-workspace-scrollbar",
                &self.review_scroll,
            ))
    }

    fn catalog_workspace(&self, wide: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = self.catalog.catalog.as_ref();
        let directory = self.store.paths.home.to_string_lossy().into_owned();
        let (actual_mode, actual_state) = self
            .status
            .as_ref()
            .map(|status| {
                (
                    status
                        .mode
                        .map(super::format::mode_name)
                        .unwrap_or("未识别"),
                    if status.recovery_pending {
                        "需要恢复"
                    } else if !status.config_exists {
                        "尚未创建"
                    } else if status.initialized {
                        "已就绪"
                    } else {
                        "未初始化"
                    },
                )
            })
            .unwrap_or(("读取中", "读取中"));
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
        let editor = if let Some(catalog) = catalog {
            let model_list_height = if wide {
                px(410.)
            } else {
                px(if catalog.models.len() >= 6 {
                    170.
                } else {
                    catalog.models.len() as f32 * 32.
                })
            };
            let model_list = div()
                .id("catalog-model-list-container")
                .relative()
                .h(model_list_height)
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
                    .child("尚未读取模型目录。"),
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
            .py_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .mb_4()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .id("catalog-config-directory")
                            .flex()
                            .gap_2()
                            .text_xs()
                            .child(
                                div()
                                    .w(px(92.))
                                    .flex_shrink_0()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Codex 配置目录"),
                            )
                            .child(div().flex_1().min_w_0().child(directory)),
                    )
                    .child(
                        div()
                            .id("catalog-current-mode")
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .child(
                                div()
                                    .w(px(92.))
                                    .flex_shrink_0()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("当前实际模式"),
                            )
                            .child(div().child(actual_mode))
                            .child(pill(
                                actual_state,
                                if actual_state == "已就绪" {
                                    rgb(0x167454).into()
                                } else {
                                    rgb(0x9b6916).into()
                                },
                            )),
                    ),
            )
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
                            .icon(IconName::FolderOpen)
                            .label("选择并关联 JSON")
                            .accessibility_label("选择并关联模型目录 JSON")
                            .tooltip("关联到当前模式 config.toml 的 model_catalog_json")
                            .disabled(!enabled || self.catalog.dirty)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_model_catalog(window, cx)
                            })),
                    ),
            )
            .child(
                div().id("catalog-association").mb_4().child(field(
                    "关联模型文件",
                    Input::new(&self.catalog.configured_path)
                        .id("catalog-configured-path")
                        .aria_label("config.toml 关联的模型目录")
                        .readonly(true),
                    wide,
                )),
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
                    ),
            )
            .when(self.catalog.dirty, |view| {
                view.child(hint(
                    "有未保存改动。保存会写入模型目录；放弃会恢复上次读取的内容。",
                    true,
                ))
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
        restart_block_reason: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let apply_block_reason = if self.busy {
            Some("正在处理当前操作。")
        } else if self.catalog.dirty {
            Some("请先保存或放弃模型目录改动。")
        } else if self
            .prepared
            .as_ref()
            .is_some_and(|prepared| !prepared.preview.changed)
        {
            Some("目标与当前配置一致，无需应用切换。")
        } else if self.prepared.is_none() {
            Some("请先生成预览，再确认应用。")
        } else {
            None
        };
        let actions = match self.tab {
            MainTab::Switch => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("preview")
                        .icon(IconName::Eye)
                        .label("预览变更")
                        .disabled(!can_preview)
                        .on_click(cx.listener(|this, _, window, cx| this.preview(window, cx))),
                )
                .child(self.restart_button(can_restart, cx)),
            MainTab::Review => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("back-to-settings")
                        .label("返回修改")
                        .ghost()
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tab = MainTab::Switch;
                            this.message = "已返回配置页；输入内容已保留，预览仍可确认。".into();
                            this.error = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("cancel-preview")
                        .label("取消预览")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.prepared = None;
                            this.tab = MainTab::Switch;
                            this.message = "已取消预览，未修改 Codex 配置。".into();
                            this.error = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("apply")
                        .primary()
                        .icon(IconName::Check)
                        .label("应用切换")
                        .accessibility_label("应用切换")
                        .disabled(!can_apply)
                        .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                ),
            MainTab::Models => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("catalog-save")
                        .compact()
                        .primary()
                        .icon(IconName::Check)
                        .label("保存")
                        .disabled(self.busy || !self.catalog.can_save())
                        .on_click(cx.listener(|this, _, window, cx| this.save_catalog(window, cx))),
                )
                .child(
                    Button::new("catalog-discard")
                        .compact()
                        .ghost()
                        .label("放弃")
                        .disabled(self.busy || !self.catalog.dirty)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.discard_catalog(window, cx)),
                        ),
                )
                .child(self.restart_button(can_restart, cx)),
            MainTab::Update => div(),
        };
        let mut footer = div()
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
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
                    .child(actions),
            );
        match self.tab {
            MainTab::Switch => {
                footer = footer.child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("预览不会写入配置。应用切换后请重启 Codex，并新建会话。"),
                );
                if let Some(reason) = restart_block_reason {
                    footer = footer.child(hint(reason, true));
                }
            }
            MainTab::Review => {
                if let Some(reason) = apply_block_reason {
                    footer = footer.child(hint(reason, true));
                }
            }
            MainTab::Models => {
                if let Some(reason) = restart_block_reason {
                    footer = footer.child(hint(reason, true));
                }
            }
            MainTab::Update => {}
        }
        if self.restart_needed && self.tab != MainTab::Review {
            footer = footer.child(hint("配置已更新。重启 Codex 后将加载最新设置。", false));
        }
        footer
    }

    fn restart_button(&self, can_restart: bool, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("restart")
            .icon(IconName::RefreshCw)
            .label("重启 Codex")
            .disabled(!can_restart)
            .on_click(cx.listener(|_, _, window, cx| {
                let switcher = cx.weak_entity();
                window.open_dialog(cx, move |dialog, _, _| {
                    let restart_switcher = switcher.clone();
                    let cancel_switcher = switcher.clone();
                    dialog
                        .title("确认重启 Codex")
                        .child(
                            div()
                                .text_sm()
                                .child("这会关闭并重新打开 Codex 桌面应用。正在生成的回复会中断，请先保存工作。"),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_sm()
                                .text_color(rgb(0x65707d))
                                .child("重启后请新建会话，以使用最新配置。"),
                        )
                        .footer(
                            DialogFooter::new()
                                .child(
                                    div().id("cancel-restart").child(
                                        DialogClose::new().trigger(|button| button.label("取消")),
                                    ),
                                )
                                .child(DialogAction::new().child(
                                    Button::new("confirm-restart")
                                        .primary()
                                        .label("确认重启"),
                                )),
                        )
                        .overlay_closable(false)
                        .keyboard(true)
                        .close_button(false)
                        .on_ok(move |_, window, cx| {
                            restart_switcher
                                .update(cx, |this, cx| this.restart(window, cx))
                                .is_ok()
                        })
                        .on_cancel(move |_, _, cx| {
                            let _ = cancel_switcher.update(cx, |this, cx| {
                                this.message = "已取消重启，Codex 未关闭。".into();
                                this.error = false;
                                cx.notify();
                            });
                            true
                        })
                });
            }))
    }

    fn clear_key_button(&self, api_key_pending: bool, cx: &Context<Self>) -> impl IntoElement {
        Button::new("clear-api-key")
            .compact()
            .icon(IconName::Close)
            .tooltip("清空 API key")
            .accessibility_label("清空 API key")
            .disabled(self.busy || !api_key_pending)
            .on_click(cx.listener(|this, _, window, cx| this.clear_key(window, cx)))
    }
}

fn section_label(label: &'static str, cx: &Context<Switcher>) -> impl IntoElement {
    div()
        .text_xs()
        .mb_2()
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

pub(super) fn scrollbar_overlay(
    id: &'static str,
    handle: &gpui_kit::ScrollHandle,
) -> impl IntoElement {
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

fn status_summary(status: &Status, cx: &Context<Switcher>) -> impl IntoElement {
    let actual_mode = status
        .mode
        .map(super::format::mode_name)
        .unwrap_or("未识别");
    let state_text = if status.recovery_pending {
        "需要恢复"
    } else if !status.config_exists {
        "未创建配置"
    } else if !status.initialized {
        "未初始化"
    } else {
        "已就绪"
    };
    let state_color = if state_text == "已就绪" {
        rgb(0x167454)
    } else {
        rgb(0x9b6916)
    };
    div()
        .mt_2()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(cx.theme().muted)
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("当前实际模式"),
        )
        .child(pill(actual_mode, rgb(0x167454).into()))
        .child(pill(state_text, state_color.into()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "{} · {}",
                    super::format::setting_value(&status.provider),
                    super::format::setting_value(&status.model),
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
