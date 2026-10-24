use super::{Switcher, mode_name};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px, rgb,
};
use switcher_core::{ImportSources, Mode};

impl Switcher {
    fn mode_card(&self, mode: Mode, cx: &App) -> impl IntoElement {
        let saved = self
            .status
            .as_ref()
            .and_then(|s| s.modes.iter().find(|m| m.mode == mode));
        let active = self.status.as_ref().is_some_and(|s| s.mode == Some(mode));
        let ready = saved.is_some_and(|s| s.ready);
        div()
            .flex_1()
            .min_w_0()
            .p_4()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
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
                            .child(mode_name(mode)),
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
                saved.map(|s| s.provider.as_str()).unwrap_or("-"),
                cx,
            ))
            .child(detail(
                "模型",
                saved.map(|s| s.model.as_str()).unwrap_or("-"),
                cx,
            ))
            .child(detail(
                "推理级别",
                saved.map(|s| s.effort.as_str()).unwrap_or("-"),
                cx,
            ))
            .when_some(saved, |card, saved| {
                card.child(
                    div()
                        .mt_3()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("配置：{}", saved.config_source)),
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
                                &saved.auth_source
                            }
                        )),
                )
                .when(!saved.issue.is_empty(), |card| {
                    card.child(
                        div()
                            .mt_2()
                            .text_xs()
                            .text_color(rgb(0xa34d20))
                            .child(saved.issue.clone()),
                    )
                })
            })
    }
}

impl Render for Switcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let initialized = self.status.as_ref().is_some_and(|s| s.initialized);
        let preview = self.prepared.as_ref().map(|p| p.preview.clone());
        let provider = if self.target == Mode::Api {
            &self.api_provider
        } else {
            &self.subscription_provider
        };
        let key_hint = masked_hint(self.key.read(cx).value().as_ref());
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(14.0))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
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
                    .when(self.demo, |row| {
                        row.child(pill("隔离演示", rgb(0x2764a6).into()))
                    })
                    .child(pill("v0.2.0", cx.theme().muted_foreground)),
            )
            .child(
                div()
                    .id("workspace-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_6()
                    .py_5()
                    .child(
                        div()
                            .text_xs()
                            .mb_2()
                            .text_color(cx.theme().muted_foreground)
                            .child("配置目录"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.directory).disabled(self.busy)),
                            )
                            .child(
                                Button::new("browse")
                                    .icon(IconName::FolderOpen)
                                    .tooltip("选择配置目录")
                                    .disabled(self.busy)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.browse(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("refresh")
                                    .icon(IconName::RefreshCw)
                                    .tooltip("刷新本地状态")
                                    .disabled(self.busy)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .mt_5()
                            .mb_3()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("模式存档"))
                            .child(div().flex_1())
                            .child(
                                Button::new("initialize")
                                    .small()
                                    .icon(IconName::Inbox)
                                    .label(if initialized {
                                        "补齐模式"
                                    } else {
                                        "初始化模式"
                                    })
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.initialize(ImportSources::default(), window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(self.mode_card(Mode::Subscription, cx))
                            .child(self.mode_card(Mode::Api, cx)),
                    )
                    .when_some(self.status.as_ref(), |view, status| {
                        view.child(
                            div()
                                .mt_3()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "当前：{}  ·  {}  ·  {}  ·  {}",
                                    status.mode.map(mode_name).unwrap_or("未知"),
                                    status.provider,
                                    status.model,
                                    status.effort
                                )),
                        )
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
                                    .gap_3()
                                    .mb_4()
                                    .child(
                                        div().font_weight(FontWeight::SEMIBOLD).child("目标模式"),
                                    )
                                    .child(
                                        ButtonGroup::new("target")
                                            .small()
                                            .disabled(self.busy)
                                            .child(
                                                Button::new("subscription")
                                                    .icon(IconName::CircleUser)
                                                    .label("订阅")
                                                    .selected(self.target == Mode::Subscription),
                                            )
                                            .child(
                                                Button::new("api")
                                                    .icon(IconName::Globe)
                                                    .label("API key")
                                                    .selected(self.target == Mode::Api),
                                            )
                                            .on_click(cx.listener(
                                                |this, selected: &Vec<usize>, _, cx| {
                                                    this.target = if selected.contains(&0) {
                                                        Mode::Subscription
                                                    } else {
                                                        Mode::Api
                                                    };
                                                    this.prepared = None;
                                                    cx.notify();
                                                },
                                            )),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("import-config")
                                            .small()
                                            .ghost()
                                            .icon(IconName::FileText)
                                            .tooltip("导入目标模式 TOML")
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.import(false, window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("import-auth")
                                            .small()
                                            .ghost()
                                            .icon(IconName::User)
                                            .tooltip("导入目标模式认证 JSON")
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.import(true, window, cx)
                                            })),
                                    ),
                            )
                            .child(field(
                                "Provider ID",
                                Input::new(provider).disabled(self.busy),
                            ))
                            .when(self.target == Mode::Api, |view| {
                                view.child(div().mt_3().child(field(
                                    "Base URL",
                                    Input::new(&self.endpoint).disabled(self.busy),
                                )))
                                .child(div().mt_3().child(field(
                                    "API key",
                                    Input::new(&self.key).disabled(self.busy),
                                )))
                                .child(
                                    div()
                                        .mt_1()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(key_hint),
                                )
                            }),
                    )
                    .when_some(preview.as_ref(), |view, preview| {
                        view.child(
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
                                                .w(px(175.0))
                                                .flex_shrink_0()
                                                .text_xs()
                                                .child(change.field.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(change.before.clone()),
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
                                                .child(change.after.clone()),
                                        )
                                }))
                                .child(
                                    div()
                                        .mt_2()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if preview.auth_changes {
                                            "认证：替换为目标模式的已保存认证"
                                        } else {
                                            "认证：无变化"
                                        }),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .px_6()
                    .py_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .mb_3()
                            .text_color(cx.theme().muted_foreground)
                            .child("应用前关闭 Codex。应用后重启，并新建会话。"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .items_center()
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
                                    .disabled(self.busy || !initialized)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.preview(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("apply")
                                    .primary()
                                    .icon(IconName::Check)
                                    .label("应用切换")
                                    .disabled(self.busy || preview.is_none())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.apply(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}

fn field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(div().w(px(95.0)).flex_shrink_0().text_sm().child(label))
        .child(div().min_w_0().flex_1().child(control))
}
fn detail(label: &'static str, value: &str, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .gap_3()
        .py_1()
        .text_sm()
        .child(
            div()
                .w(px(70.0))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(value.to_string()))
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
fn masked_hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    if chars.len() < 10 {
        return "已输入密钥".into();
    }
    format!(
        "{}······{}",
        chars.iter().take(4).collect::<String>(),
        chars.iter().skip(chars.len() - 4).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn key_hint_never_reveals_short_keys_or_the_middle() {
        assert_eq!(super::masked_hint(""), "");
        assert_eq!(super::masked_hint("abc"), "已输入密钥");
        assert_eq!(super::masked_hint("synthetic-demo-key"), "synt······-key");
        assert!(!super::masked_hint("synthetic-demo-key").contains("hetic-demo"));
    }
}
