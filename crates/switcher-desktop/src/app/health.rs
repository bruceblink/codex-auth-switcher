use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogFooter},
};
use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled,
    TestSupportExt as _, Window, div, prelude::*, rgb,
};
use std::sync::Arc;
use switcher_core::config_check::{ConfigCheck, ConfigIssue, ConfigIssueSeverity};

use super::Switcher;

impl Switcher {
    pub(crate) fn configuration_workspace(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .id("config-workspace")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.health_scroll)
            .px_6()
            .py_5()
            .child(
                div()
                    .id("config-page-heading")
                    .test_support()
                    .mb_5()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("配置检查"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("检查常见配置项，并查看可安全清理的已确认设置。"),
                    ),
            )
            .child(
                div()
                    .id("config-check-scope")
                    .test_support()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xe5e9ef))
                    .bg(rgb(0xf7f9fb))
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("检查范围"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "这是针对已核实规则的有限检查，不是 Codex 完整配置格式检查。未知设置不会删除。",
                            ),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "清理只修改当前 config.toml；已保存的模式不变，切换后可重新检查。",
                            ),
                    ),
            );

        let content = if let Some(report) = &self.health_report {
            content.child(config_report(report, cx))
        } else {
            content.child(
                div()
                    .id("config-check-empty")
                    .test_support()
                    .mt_4()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xe5e9ef))
                    .bg(rgb(0xffffff))
                    .px_4()
                    .py_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.busy {
                        "正在检查配置…"
                    } else {
                        "尚未检查配置。点击“检查配置”读取本地 config.toml。"
                    }),
            )
        };

        div()
            .id("config-workspace-container")
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(content)
    }

    pub(crate) fn configuration_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_key = !self.key.read(cx).value().is_empty();
        let can_clean = !self.busy
            && !self.catalog.dirty
            && !has_key
            && self.prepared.is_none()
            && !self.directory_needs_refresh
            && self
                .health_report
                .as_ref()
                .is_some_and(|report| report.can_cleanup());
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2()
            .child(
                Button::new("config-check")
                    .primary()
                    .icon(IconName::RefreshCw)
                    .label(if self.health_report.is_some() {
                        "重新检查"
                    } else {
                        "检查配置"
                    })
                    .accessibility_label("检查本地 Codex 配置")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.check_configuration(window, cx);
                    })),
            )
            .child(
                Button::new("config-clean")
                    .icon(IconName::Check)
                    .label("清理已确认设置")
                    .accessibility_label("清理检查器确认可移除的设置")
                    .disabled(!can_clean)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clean_configuration(window, cx);
                    })),
            );

        div()
            .id("config-footer")
            .test_support()
            .bg(rgb(0xffffff))
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(rgb(0xe5e9ef))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .id("config-message")
                            .test_support()
                            .min_w_0()
                            .text_sm()
                            .whitespace_normal()
                            .text_color(if self.error {
                                rgb(0xad3b32).into()
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(self.message.clone()),
                    )
                    .child(actions),
            )
    }

    pub(crate) fn check_configuration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.select_directory(window, cx) {
            return;
        }
        self.health_report = None;
        self.execute(
            window,
            cx,
            |store| store.check_config(),
            |this, report, _, _| {
                let issue_count = report.issues.len();
                this.health_report = Some(Arc::new(report));
                this.message = if issue_count == 0 {
                    "检查完成。未发现检查器覆盖范围内的问题。".into()
                } else {
                    format!("检查完成。发现 {issue_count} 项，请查看报告。")
                };
                this.error = false;
            },
        );
    }

    pub(crate) fn clean_configuration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let blocked = self.catalog.dirty
            || !self.key.read(cx).value().is_empty()
            || self.prepared.is_some()
            || self.directory_needs_refresh;
        let Some(report) = self
            .health_report
            .as_ref()
            .filter(|report| report.can_cleanup())
            .cloned()
        else {
            return;
        };
        if blocked {
            self.message =
                "请先保存或放弃模型改动、清空未应用的 API key，并处理待确认的切换预览。".into();
            self.error = true;
            cx.notify();
            return;
        }

        let cleanup_keys = report
            .issues
            .iter()
            .filter(|issue| issue.cleanup)
            .map(|issue| issue.key.clone())
            .collect::<Vec<_>>();
        let cleanup_count = cleanup_keys.len();
        let weak = cx.weak_entity();
        let cleanup_keys_for_dialog = cleanup_keys.clone();
        let report_for_dialog = report.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let confirm_switcher = weak.clone();
            let cleanup_keys = cleanup_keys_for_dialog.clone();
            let report = report_for_dialog.clone();
            dialog
                .title("确认清理配置")
                .child(div().text_sm().child(format!(
                    "将从当前 config.toml 清理 {cleanup_count} 项已确认设置："
                )))
                .child(
                    div()
                        .id("config-clean-confirmation-items")
                        .test_support()
                        .mt_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(
                            cleanup_keys
                                .into_iter()
                                .map(|key| div().text_xs().text_color(rgb(0x65707d)).child(key)),
                        ),
                )
                .child(
                    div()
                        .mt_3()
                        .text_xs()
                        .text_color(rgb(0x65707d))
                        .child("这里只显示设置名称，不显示配置值。其他设置和模式存档不会修改。"),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("config-clean-cancel")
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("config-clean-confirm")
                                    .primary()
                                    .label("确认清理")
                                    .accessibility_label("确认清理已列出的配置设置"),
                            ),
                        ),
                )
                .overlay_closable(false)
                .keyboard(true)
                .close_button(false)
                .on_ok(move |_, window, cx| {
                    confirm_switcher
                        .update(cx, |this, cx| {
                            this.perform_configuration_cleanup(report.clone(), window, cx)
                        })
                        .is_ok()
                })
                .on_cancel(|_, _, _| true)
        });
    }

    fn perform_configuration_cleanup(
        &mut self,
        report: Arc<ConfigCheck>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy
            || self.catalog.dirty
            || !self.key.read(cx).value().is_empty()
            || self.prepared.is_some()
            || self.directory_needs_refresh
        {
            self.message = "当前有未完成的输入或预览。未清理配置，请处理后重新检查。".into();
            self.error = true;
            cx.notify();
            return;
        }

        self.health_report = None;
        self.execute(
            window,
            cx,
            move |store| {
                let cleanup = store.clean_config(&report)?;
                let checked = store.check_config();
                Ok((cleanup, checked))
            },
            |this, (cleanup, refreshed), _, _| {
                this.prepared = None;
                if cleanup.changed {
                    this.restart_needed = true;
                }
                let refreshed_ok = refreshed.is_ok();
                match refreshed {
                    Ok(report) => this.health_report = Some(Arc::new(report)),
                    Err(_) => this.health_report = None,
                }
                this.message = match (cleanup.changed, refreshed_ok) {
                    (true, true) => format!(
                        "已清理 {} 项设置。备份只包含被移除的设置；Codex 需要重启后读取最新配置。",
                        cleanup.removed.len()
                    ),
                    (true, false) => format!(
                        "已清理 {} 项设置，但无法刷新报告。请重新检查配置。",
                        cleanup.removed.len()
                    ),
                    (false, true) => "配置没有变化。报告已刷新。".into(),
                    (false, false) => "配置没有变化，但无法刷新报告。请重新检查配置。".into(),
                };
                this.error = !refreshed_ok;
            },
        );
    }
}

fn config_report(report: &ConfigCheck, cx: &Context<Switcher>) -> impl IntoElement {
    let issue_count = report.issues.len();
    let cleanup_count = report.issues.iter().filter(|issue| issue.cleanup).count();
    let issues = if report.issues.is_empty() {
        div()
            .id("config-issues-empty")
            .test_support()
            .mt_3()
            .rounded_md()
            .bg(rgb(0xe5f3ed))
            .px_4()
            .py_3()
            .text_sm()
            .text_color(rgb(0x167454))
            .child("未发现检查器覆盖范围内的问题。")
            .into_any_element()
    } else {
        div()
            .id("config-issues")
            .test_support()
            .mt_3()
            .flex()
            .flex_col()
            .gap_2()
            .children(report.issues.iter().map(|issue| config_issue(issue, cx)))
            .into_any_element()
    };

    div()
        .id("config-check-report")
        .test_support()
        .mt_4()
        .child(
            div()
                .rounded_md()
                .border_1()
                .border_color(rgb(0xe5e9ef))
                .bg(rgb(0xffffff))
                .px_4()
                .py_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("已检查文件"),
                )
                .child(
                    div()
                        .id("config-report-path")
                        .test_support()
                        .mt_1()
                        .min_w_0()
                        .text_sm()
                        .child(report.path.display().to_string()),
                ),
        )
        .child(
            div()
                .mt_4()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("检查结果"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{issue_count} 项发现 · {cleanup_count} 项可清理")),
                ),
        )
        .child(issues)
}

fn config_issue(issue: &ConfigIssue, cx: &Context<Switcher>) -> impl IntoElement {
    let (label, color) = match issue.severity {
        ConfigIssueSeverity::Info => ("提示", rgb(0x2c71a0)),
        ConfigIssueSeverity::Warning => ("警告", rgb(0x9b6916)),
        ConfigIssueSeverity::Error => ("错误", rgb(0xad3b32)),
    };
    div()
        .id(format!("config-issue-{}", issue.key))
        .test_support()
        .rounded_md()
        .border_1()
        .border_color(rgb(0xe5e9ef))
        .bg(rgb(0xffffff))
        .px_4()
        .py_3()
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(Icon::new(match issue.severity {
                    ConfigIssueSeverity::Info => IconName::Info,
                    ConfigIssueSeverity::Warning => IconName::CircleAlert,
                    ConfigIssueSeverity::Error => IconName::CircleAlert,
                }))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color)
                        .child(label),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(issue.title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(issue.key.clone()),
                )
                .when(issue.cleanup, |row| {
                    row.child(
                        div()
                            .rounded_md()
                            .bg(rgb(0xe5f3ed))
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(rgb(0x167454))
                            .child("可清理"),
                    )
                }),
        )
        .child(
            div()
                .mt_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(issue.detail.clone()),
        )
}
