use gpui_kit::TestSupportExt as _;
use gpui_kit::Window;
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    progress::Progress,
    text::TextView,
};
use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, rgb,
};

use super::{
    Switcher,
    update::{
        AvailableView, UpdatePackageKind, UpdateStatus, format_bytes, format_last_check,
        platform_name, progress_percent, progress_summary, update_package_kind,
    },
    view::scrollbar_overlay,
};

impl Switcher {
    pub(crate) fn update_workspace(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let check = &self.updater;
        let mut content = div()
            .id("update-workspace")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.update_scroll)
            .px_6()
            .py_5()
            .child(
                div()
                    .id("update-page-heading")
                    .test_support()
                    .mb_5()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("软件更新"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("检查稳定版更新，并查看适用于当前平台的安装包。"),
                    ),
            )
            .child(
                div()
                    .id("update-current-version")
                    .test_support()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xe5e9ef))
                    .bg(rgb(0xf7f9fb))
                    .px_4()
                    .py_3()
                    .child(update_section_label("当前版本"))
                    .child(
                        div()
                            .mt_3()
                            .flex()
                            .gap_6()
                            .child(div().flex_1().min_w_0().child(update_value(
                                "已安装版本",
                                format!("v{}", env!("CARGO_PKG_VERSION")),
                                cx,
                            )))
                            .child(div().flex_1().min_w_0().child(update_value(
                                "当前平台",
                                platform_name(),
                                cx,
                            ))),
                    ),
            )
            .child(
                div()
                    .id("update-status-panel")
                    .test_support()
                    .mt_4()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xe5e9ef))
                    .bg(rgb(0xf7f9fb))
                    .px_4()
                    .py_3()
                    .child(update_section_label("检查结果"))
                    .child(update_status_summary(&check.status, cx)),
            );

        if let Some(release) = check.release() {
            content = content
                .child(
                    div()
                        .mt_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(update_section_label("版本说明"))
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(release.published_at.clone()),
                        ),
                )
                .child(
                    div()
                        .id("update-release-notes-panel")
                        .test_support()
                        .mt_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0xe5e9ef))
                        .bg(rgb(0xf7f9fb))
                        .px_4()
                        .py_3()
                        .min_w_0()
                        .child(
                            TextView::markdown(
                                "update-release-notes-content",
                                release.notes.clone(),
                            )
                            .selectable(true),
                        ),
                );
            if let UpdateStatus::Available(update)
            | UpdateStatus::Downloading { update, .. }
            | UpdateStatus::Cancelling { update, .. }
            | UpdateStatus::VerifyingInstall { update }
            | UpdateStatus::Ready { update, .. }
            | UpdateStatus::FailedDownload { update, .. } = &check.status
            {
                content = content.child(package_summary(update, &check.status, cx));
            }
        }

        content = content.child(
            div()
                .id("update-preferences")
                .test_support()
                .mt_5()
                .pt_4()
                .border_t_1()
                .border_color(rgb(0xe5e9ef))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("偏好设置"),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("自动检查只会提醒你，不会自动下载或安装更新。"),
                )
                .child(
                    div()
                        .mt_3()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(
                            Checkbox::new("update-auto-check")
                                .label("自动检查稳定版更新")
                                .accessibility_label("自动检查稳定版更新")
                                .tooltip("启动后延迟检查；已是最新版时，24 小时内不重复自动检查。")
                                .checked(check.preferences.auto_check)
                                .disabled(!check.preferences_loaded || check.preferences_saving)
                                .on_click(cx.listener(|this, checked, window, cx| {
                                    this.set_auto_check(*checked, window, cx);
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if check.preferences_saving {
                                    "正在保存设置…"
                                } else {
                                    "上次成功检查"
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format_last_check(
                                    check.preferences.last_successful_check.as_ref(),
                                )),
                        ),
                ),
        );

        div()
            .id("update-workspace-container")
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(content)
            .child(scrollbar_overlay(
                "update-workspace-scrollbar",
                &self.update_scroll,
            ))
    }

    pub(crate) fn update_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let status = &self.updater.status;
        let active = self.updater.status_is_active();
        let message = self.updater.message();
        let install_block_reason = if matches!(status, UpdateStatus::Ready { .. }) {
            self.install_block_reason(cx)
        } else {
            None
        };
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2();

        if self.updater.release().is_some() {
            actions = actions.child(
                Button::new("update-release-notes")
                    .compact()
                    .ghost()
                    .icon(IconName::ExternalLink)
                    .tooltip("打开项目 GitHub 发布说明")
                    .accessibility_label("打开发布说明")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_update_release_notes(cx);
                    })),
            );
        }

        if let UpdateStatus::Available(update) = status {
            let update = update.clone();
            actions = actions.child(
                Button::new("update-download")
                    .compact()
                    .primary()
                    .icon(IconName::Download)
                    .label("下载")
                    .accessibility_label("下载并校验更新安装包")
                    .tooltip("下载与当前平台匹配的安装包，并校验大小和 SHA-256。")
                    .disabled(active)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_update_download(update.clone(), window, cx);
                    })),
            );
        } else if let UpdateStatus::Downloading { .. } = status {
            actions = actions.child(
                Button::new("update-cancel")
                    .compact()
                    .ghost()
                    .icon(IconName::Close)
                    .label("取消下载")
                    .accessibility_label("取消更新下载")
                    .tooltip("取消下载并清理未完成的安装包。")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_update_download(cx);
                    })),
            );
        } else if let UpdateStatus::Cancelling { .. } = status {
            actions = actions.child(
                Button::new("update-cancelling")
                    .compact()
                    .ghost()
                    .icon(IconName::Close)
                    .label("正在取消")
                    .accessibility_label("正在取消更新下载")
                    .disabled(true),
            );
        } else if let UpdateStatus::VerifyingInstall { .. } = status {
            actions = actions.child(
                Button::new("update-installing")
                    .compact()
                    .ghost()
                    .icon(IconName::Loader)
                    .label("正在校验")
                    .accessibility_label("正在重新校验更新安装包")
                    .disabled(true),
            );
        } else if let UpdateStatus::Ready { update, .. } = status {
            let package_kind = update_package_kind(&update.package.name);
            let (label, accessibility_label, default_tooltip) = match package_kind {
                UpdatePackageKind::Installer => (
                    "安装更新",
                    "重新校验并启动更新安装程序",
                    "确认后重新校验安装包，再由系统打开。",
                ),
                UpdatePackageKind::AppImage => (
                    "打开目录",
                    "重新校验并打开 AppImage 安装包目录",
                    "确认后重新校验并打开目录；请关闭旧版并手动替换 AppImage 文件。",
                ),
                UpdatePackageKind::Dmg => (
                    "打开 DMG",
                    "重新校验并打开 DMG，以手动替换应用程序",
                    "确认后重新校验并打开 DMG，再手动替换 Applications 中的应用。",
                ),
            };
            let tooltip = install_block_reason
                .clone()
                .unwrap_or_else(|| default_tooltip.to_owned());
            actions = actions.child(
                Button::new("update-install")
                    .compact()
                    .primary()
                    .icon(IconName::Download)
                    .label(label)
                    .accessibility_label(accessibility_label)
                    .tooltip(tooltip)
                    .disabled(active || install_block_reason.is_some())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.request_update_install(window, cx);
                    })),
            );
            actions = actions.child(
                Button::new("update-open-package")
                    .compact()
                    .primary()
                    .icon(IconName::FolderOpen)
                    .label("打开安装包目录")
                    .accessibility_label("打开已校验安装包所在目录")
                    .tooltip("仅显示已下载并通过校验的安装包。")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.reveal_update_package(window, cx);
                    })),
            );
        } else if let UpdateStatus::FailedDownload { update, .. } = status {
            let update = update.clone();
            actions = actions.child(
                Button::new("update-retry-download")
                    .compact()
                    .primary()
                    .icon(IconName::RefreshCw)
                    .label("重试下载")
                    .accessibility_label("重试更新下载")
                    .tooltip("重新下载并校验当前平台安装包。")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_update_download(update.clone(), window, cx);
                    })),
            );
            actions = actions.child(
                Button::new("update-check-again")
                    .compact()
                    .ghost()
                    .icon(IconName::RefreshCw)
                    .tooltip("手动再次检查更新")
                    .accessibility_label("再次检查更新")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_update_check(window, cx);
                    })),
            );
        } else if let UpdateStatus::FailedCheck { .. } = status {
            actions = actions.child(
                Button::new("update-retry-check")
                    .compact()
                    .primary()
                    .icon(IconName::RefreshCw)
                    .label("重试检查")
                    .accessibility_label("重试检查更新")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.retry_update(window, cx);
                    })),
            );
        } else if let UpdateStatus::Checking = status {
            actions = actions.child(
                Button::new("update-checking")
                    .compact()
                    .ghost()
                    .icon(IconName::Loader)
                    .label("正在检查")
                    .accessibility_label("正在检查更新")
                    .disabled(true),
            );
        } else {
            actions = actions.child(
                Button::new("update-check")
                    .compact()
                    .primary()
                    .icon(IconName::RefreshCw)
                    .label("检查更新")
                    .accessibility_label("手动检查稳定版更新")
                    .tooltip("手动检查不受自动检查的 24 小时限制。")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_update_check(window, cx);
                    })),
            );
        }

        if matches!(
            status,
            UpdateStatus::Available(_) | UpdateStatus::Ready { .. }
        ) {
            actions = actions.child(
                Button::new("update-check-again")
                    .compact()
                    .ghost()
                    .icon(IconName::RefreshCw)
                    .tooltip("手动再次检查更新")
                    .accessibility_label("再次检查更新")
                    .disabled(active)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_update_check(window, cx);
                    })),
            );
        }

        div()
            .id("update-footer")
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .id("update-message")
                            .test_support()
                            .w_full()
                            .min_w_0()
                            .text_sm()
                            .whitespace_normal()
                            .text_color(
                                if self.updater.preferences_error.is_some()
                                    || self.updater.operation_error
                                    || matches!(
                                        status,
                                        UpdateStatus::FailedCheck { .. }
                                            | UpdateStatus::FailedDownload { .. }
                                    )
                                {
                                    rgb(0xad3b32).into()
                                } else {
                                    cx.theme().muted_foreground
                                },
                            )
                            .child(message),
                    )
                    .child(actions),
            )
            .child(
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
            .child("下载只保存已校验的安装包；macOS DMG 和 AppImage 需要手动替换。Codex 不会自动重启。"),
            )
            .children(install_block_reason.map(|reason| {
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(rgb(0x9b6916))
                    .child(reason)
            }))
    }

    fn request_update_install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let UpdateStatus::Ready { update, .. } = self.updater.status.clone() else {
            return;
        };
        if let Some(reason) = self.install_block_reason(cx) {
            self.updater.operation_notice = Some(reason);
            self.updater.operation_error = false;
            cx.notify();
            return;
        }

        let package_kind = update_package_kind(&update.package.name);
        let (title, prompt, confirm_label) = match package_kind {
            UpdatePackageKind::Installer => (
                format!("确认启动 v{} 更新", update.release.version),
                "确认后会重新校验文件，并由系统打开安装程序。请先保存其他未应用的工作。Codex 不会自动重启。",
                "确认启动",
            ),
            UpdatePackageKind::AppImage => (
                format!("确认打开 v{} AppImage 目录", update.release.version),
                "确认后会重新校验文件并打开所在目录。请关闭旧版并手动替换 AppImage 文件；程序不会执行它。",
                "打开目录",
            ),
            UpdatePackageKind::Dmg => (
                format!("确认打开 v{} 的 DMG", update.release.version),
                "确认后会重新校验文件并挂载 DMG。请关闭旧版应用，再把新应用拖入 Applications 并替换。",
                "打开 DMG",
            ),
        };
        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let confirm_switcher = weak.clone();
            let cancel_switcher = weak.clone();
            dialog
                .title(title.clone())
                .child(
                    div()
                        .text_sm()
                        .child(format!("安装包：{}", update.package.name)),
                )
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(rgb(0x65707d))
                        .child(prompt),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            div()
                                .id("cancel-update-install")
                                .child(DialogClose::new().trigger(|button| button.label("取消"))),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("confirm-update-install")
                                    .primary()
                                    .label(confirm_label)
                                    .accessibility_label(confirm_label),
                            ),
                        ),
                )
                .overlay_closable(false)
                .keyboard(true)
                .close_button(false)
                .on_ok(move |_, window, cx| {
                    confirm_switcher
                        .update(cx, |this, cx| this.begin_update_install(window, cx))
                        .is_ok()
                })
                .on_cancel(move |_, _, cx| {
                    let _ = cancel_switcher.update(cx, |this, cx| {
                        this.updater.operation_notice = Some("已取消启动更新。".to_owned());
                        this.updater.operation_error = false;
                        cx.notify();
                    });
                    true
                })
        });
    }
}

fn update_status_summary(status: &UpdateStatus, cx: &Context<Switcher>) -> impl IntoElement {
    let (title, details, color) = match status {
        UpdateStatus::Idle => (
            "等待检查",
            "手动检查或等待稳定版自动检查。".to_owned(),
            0x65707d,
        ),
        UpdateStatus::Checking => ("正在检查", "正在读取发布清单。".to_owned(), 0x2c71a0),
        UpdateStatus::Current { current_version } => (
            "已是最新版本",
            format!("当前版本：v{current_version}"),
            0x167454,
        ),
        UpdateStatus::Available(update) => (
            "发现新版",
            format!("可更新到 v{}", update.release.version),
            0x167454,
        ),
        UpdateStatus::LegacyRelease { release } => (
            "新版缺少更新清单",
            format!(
                "v{} · {}，无法安全选择安装包。",
                release.version, release.tag_name
            ),
            0x9b6916,
        ),
        UpdateStatus::UnsupportedPlatform { release } => (
            "当前平台没有安装包",
            format!("v{} 没有匹配的安装包。", release.version),
            0x9b6916,
        ),
        UpdateStatus::Downloading { update, .. } => (
            "正在下载",
            format!("正在获取 v{} 的安装包。", update.release.version),
            0x2c71a0,
        ),
        UpdateStatus::Cancelling { update, .. } => (
            "正在取消",
            format!("正在清理 v{} 的未完成下载。", update.release.version),
            0x9b6916,
        ),
        UpdateStatus::VerifyingInstall { update } => (
            "正在重新校验",
            format!("正在复核 v{} 的安装包。", update.release.version),
            0x2c71a0,
        ),
        UpdateStatus::Ready { update, .. } => (
            "下载完成",
            format!("v{} 的安装包已通过校验。", update.release.version),
            0x167454,
        ),
        UpdateStatus::FailedCheck { .. } => ("检查失败", "请检查网络后重试。".to_owned(), 0xad3b32),
        UpdateStatus::FailedDownload { .. } => (
            "安装包不可用",
            "下载或校验未完成，可以重试。".to_owned(),
            0xad3b32,
        ),
    };

    div()
        .mt_2()
        .px_4()
        .py_3()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Icon::new(match status {
                UpdateStatus::Checking
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::Cancelling { .. }
                | UpdateStatus::VerifyingInstall { .. } => IconName::Loader,
                UpdateStatus::Current { .. } | UpdateStatus::Ready { .. } => IconName::Check,
                UpdateStatus::FailedCheck { .. } | UpdateStatus::FailedDownload { .. } => {
                    IconName::CircleAlert
                }
                _ => IconName::Info,
            })
            .size_4()
            .text_color(rgb(color)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(color))
                        .child(title),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(details),
                ),
        )
}

fn package_summary(
    update: &AvailableView,
    status: &UpdateStatus,
    cx: &Context<Switcher>,
) -> impl IntoElement {
    let package = &update.package;
    let mut row = div()
        .mt_3()
        .pt_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(update_section_label("适用于当前平台的安装包"))
        .child(
            div()
                .mt_2()
                .flex()
                .items_center()
                .gap_2()
                .child(Icon::new(IconName::Package).size_4())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .child(package.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format_bytes(package.size)),
                ),
        )
        .child({
            let hash_groups = package
                .sha256
                .chars()
                .collect::<Vec<_>>()
                .chunks(8)
                .map(|group| group.iter().collect::<String>())
                .collect::<Vec<_>>();
            div()
                .id("update-package-sha256")
                .test_support()
                .mt_1()
                .ml_6()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("SHA-256")
                .children(hash_groups)
        });

    match status {
        UpdateStatus::Downloading { progress, .. } | UpdateStatus::Cancelling { progress, .. } => {
            row = row
                .child(div().mt_3().child(
                    Progress::new("update-download-progress").value(progress_percent(*progress)),
                ))
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(progress_summary(*progress)),
                );
        }
        UpdateStatus::VerifyingInstall { .. } => {
            row = row.child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("正在重新校验安装包大小、SHA-256 和缓存路径。"),
            );
        }
        UpdateStatus::Ready { path, .. } => {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| package.name.clone());
            row = row.child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("已校验文件：{file_name}")),
            );
        }
        UpdateStatus::FailedDownload { message, .. } => {
            row = row.child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(rgb(0xad3b32))
                    .child(message.clone()),
            );
        }
        _ => {}
    }
    row
}

fn update_value(
    label: &'static str,
    value: impl Into<SharedString>,
    cx: &Context<Switcher>,
) -> impl IntoElement {
    let value = value.into();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(value))
}

fn update_section_label(label: &'static str) -> impl IntoElement {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}
