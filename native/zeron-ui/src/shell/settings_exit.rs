//! Exit decisions aggregate retained settings owners without taking over their drafts.
use super::*;
use gpui_kit::component::{
    Sizable, WindowExt,
    button::{Button, ButtonVariants},
    notification::Notification,
};

impl Shell {
    pub(crate) fn reset_exit_attempt(&mut self, cx: &mut Context<Self>) {
        self.discard_settings_on_exit = false;
        self.pending_exit = None;
        cx.notify();
    }

    pub(super) fn protect_settings_exit(
        &mut self,
        action: PendingExit,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut dirty = Vec::new();
        let mut busy = Vec::new();
        macro_rules! inspect {
            ($page:expr, $label:literal, $busy:expr) => {
                if let Some(page) = &$page {
                    let page = page.read(cx);
                    if $busy(page, cx) {
                        busy.push($label);
                    }
                    if page.has_unsaved_changes(cx) {
                        dirty.push($label);
                    }
                }
            };
        }
        inspect!(
            self.provider_page,
            "模型",
            |p: &crate::providers::ProviderSettings, _: &App| p.is_busy()
        );
        inspect!(
            self.search_page,
            "网络搜索",
            |p: &settings::search::SearchPage, _: &App| p.is_busy()
        );
        inspect!(
            self.memory_page,
            "记忆",
            |p: &settings::memory::MemoryPage, _: &App| p.is_busy()
        );
        inspect!(
            self.cli_page,
            "本地 CLI",
            |p: &settings::cli::CliPage, _: &App| p.is_busy()
        );
        inspect!(
            self.backup_page,
            "备份与同步",
            |p: &settings::backup::BackupPage, cx: &App| p.is_busy(cx)
        );
        inspect!(
            self.kivio_settings_page,
            "设置",
            |p: &crate::kivio_settings::SettingsPage, cx: &App| p.is_busy(cx)
        );
        inspect!(
            self.hooks_page,
            "Hooks",
            |p: &settings::hooks::HooksPage, cx: &App| p.is_busy(cx)
        );
        inspect!(
            self.task_center,
            "任务",
            |p: &crate::task_center::TaskCenter, _: &App| p.is_busy()
        );
        inspect!(
            self.library_page,
            "对话库",
            |p: &settings::library::LibraryPage, _: &App| p.is_busy()
        );
        if busy.is_empty() && (dirty.is_empty() || self.discard_settings_on_exit) {
            return true;
        }
        let Some((handle, _)) = crate::shell_windows(cx)
            .into_iter()
            .find(|(_, shell)| shell.entity_id() == cx.entity_id())
        else {
            return false;
        };
        if !busy.is_empty() {
            // Never stack a modal above a saving editor: its completion closes
            // the top Kit dialog, which must remain its own editor.
            let message = format!(
                "{}仍在执行操作。请等待完成或停止操作后再试。",
                busy.join("、")
            );
            crate::app_menus::cancel_exit_attempt(cx);
            cx.defer(move |cx| {
                let _ = cx.update_window(handle.into(), |_, window, cx| {
                    window.push_notification(Notification::warning(message), cx);
                });
            });
            return false;
        }
        if self.settings_exit_dialog {
            return false;
        }
        self.settings_exit_dialog = true;
        let owner = cx.weak_entity();
        let message = format!("{}有未保存的修改。继续将丢弃这些修改。", dirty.join("、"));
        cx.defer(move |cx| {
            let _ = cx.update_window(handle.into(), |_, window, cx| {
                window.open_dialog(cx, move |dialog, _, _| {
                    let close_owner = owner.clone();
                    let cancel_owner = owner.clone();
                    let confirm_owner = owner.clone();
                    let action = action.clone();
                    dialog
                        .title("保留设置修改？")
                        .width(px(440.))
                        .overlay_closable(false)
                        .on_close(move |_, _, cx| {
                            crate::app_menus::cancel_exit_attempt(cx);
                            let _ = close_owner.update(cx, |this, cx| {
                                this.settings_exit_dialog = false;
                                cx.notify();
                            });
                        })
                        .child(div().text_sm().child(message.clone()))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("cancel-settings-exit")
                                        .small()
                                        .label("继续编辑")
                                        .on_click(move |_, window, cx| {
                                            window.close_dialog(cx);
                                            crate::app_menus::cancel_exit_attempt(cx);
                                            let _ = cancel_owner.update(cx, |this, cx| {
                                                this.settings_exit_dialog = false;
                                                cx.notify();
                                            });
                                        }),
                                )
                                .map(|view| {
                                    view.child(
                                        Button::new("discard-settings-exit")
                                            .small()
                                            .danger()
                                            .label(match action {
                                                PendingExit::Restart => "放弃并重启",
                                                PendingExit::CloseWindow => "放弃并关闭",
                                                _ => "放弃并退出",
                                            })
                                            .on_click(move |_, window, cx| {
                                                window.close_dialog(cx);
                                                let _ = confirm_owner.update(cx, |this, _| {
                                                    this.discard_settings_on_exit = true;
                                                    this.settings_exit_dialog = false;
                                                });
                                                match &action {
                                                    PendingExit::Quit => {
                                                        crate::app_menus::continue_quit(cx)
                                                    }
                                                    PendingExit::Restart => {
                                                        crate::app_menus::continue_restart(cx)
                                                    }
                                                    PendingExit::CloseWindow => {
                                                        if confirm_owner
                                                            .update(cx, |this, cx| {
                                                                this.prepare_window_close(cx)
                                                            })
                                                            .unwrap_or(false)
                                                        {
                                                            window.remove_window();
                                                        }
                                                    }
                                                    PendingExit::RuntimeChange => {
                                                        let _ =
                                                            confirm_owner.update(cx, |this, cx| {
                                                                this.quit_for_runtime_change(cx)
                                                            });
                                                    }
                                                    PendingExit::InstallUpdate(path) => {
                                                        let _ =
                                                            confirm_owner.update(cx, |this, cx| {
                                                                this.apply_staged_update(
                                                                    path.clone(),
                                                                    cx,
                                                                )
                                                            });
                                                    }
                                                }
                                            }),
                                    )
                                }),
                        )
                });
            });
        });
        false
    }
}
