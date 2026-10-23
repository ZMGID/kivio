use super::*;
use crate::preferences::{AppearanceMode, Preferences, TextSize};
use gpui_kit::component::notification::Notification;

impl Shell {
    pub(super) fn load_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_preferences(None, window, cx);
    }

    fn current_preferences(&self, cx: &App) -> Preferences {
        Preferences {
            mode: if self.follow_system {
                AppearanceMode::System
            } else if cx.theme().is_dark() {
                AppearanceMode::Dark
            } else {
                AppearanceMode::Light
            },
            text_size: if self.font_size == 13. {
                TextSize::Compact
            } else if self.font_size == 16. {
                TextSize::Comfortable
            } else {
                TextSize::Default
            },
            reduce_motion: self.reduce_motion_override,
        }
    }

    // One task owns loading/writing. Publish the change only after durable save;
    // a failure leaves the controls and the effective appearance unchanged.
    fn update_preferences(
        &mut self,
        next: Option<Preferences>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preferences_busy || (next.is_some() && !self.preferences_ready) {
            return;
        }
        self.preferences_busy = true;
        let path = self.catalog_dir.join("appearance.json");
        let work = cx.background_spawn(async move {
            match next {
                Some(preferences) => preferences.save(&path),
                None => Preferences::load(&path),
            }
        });
        self.preferences_task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.preferences_busy = false;
                match result {
                    Ok(preferences) => {
                        this.preferences_ready = true;
                        this.font_size = preferences.text_size.points();
                        this.reduce_motion_override = preferences.reduce_motion;
                        if let Some(reduce) = preferences.reduce_motion {
                            cx.set_reduce_motion(reduce);
                        } else {
                            gpui_kit::base::apply_system_reduce_motion(cx);
                        }
                        this.appearance(
                            match preferences.mode {
                                AppearanceMode::System => None,
                                AppearanceMode::Light => Some(ThemeMode::Light),
                                AppearanceMode::Dark => Some(ThemeMode::Dark),
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(Notification::error(error), cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn setting_row(
        &self,
        title: &'static str,
        detail: &'static str,
        control: impl IntoElement,
        compact: bool,
        cx: &App,
    ) -> AnyElement {
        div()
            .flex()
            .when(compact, |row| row.flex_col().items_start())
            .when(!compact, |row| row.items_center().justify_between())
            .gap_3()
            .py_4()
            .when(compact, |row| row.gap_2().py_3())
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail),
                    ),
            )
            .child(div().flex_shrink_0().child(control))
            .into_any_element()
    }

    pub(super) fn render_settings(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pane_width = window.bounds().size.width
            - if self.sidebar_visible(window) {
                self.panels
                    .read(cx)
                    .sizes()
                    .first()
                    .copied()
                    .unwrap_or(px(240.))
            } else {
                px(0.)
            };
        let compact = pane_width < px(620.);
        let page = self.settings.unwrap_or(SettingsPage::Appearance);
        let (title, description) = match page {
            SettingsPage::Providers => {
                ("供应商与模型", "配置模型连接，并选择新对话使用的默认模型。")
            }
            SettingsPage::Appearance => ("外观", "让工作空间更适合你的阅读习惯。"),
            SettingsPage::Shortcuts => ("快捷键", "常用操作，保持双手留在键盘上。"),
            SettingsPage::About => ("关于 Kivio", "一个安静、专注的 AI 工作空间。"),
        };
        let mut content = div()
            .v_flex()
            .w_full()
            .max_w(rems(53.))
            .gap_1()
            .child(
                div()
                    .text_size(rems(1.43))
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .mb_6()
                    .when(compact, |text| text.mb_3())
                    .child(description),
            );
        match page {
            SettingsPage::Providers => {
                content = content.child(self.providers.clone());
            }
            SettingsPage::Appearance => {
                let disabled = self.preferences_busy || !self.preferences_ready;
                if self.preferences_busy {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if self.preferences_ready {
                                "正在保存外观设置…"
                            } else {
                                "正在读取外观设置…"
                            }),
                    );
                } else if !self.preferences_ready {
                    content = content.child(
                        Button::new("retry-appearance")
                            .label("重新读取外观设置")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_preferences(window, cx);
                            })),
                    );
                }
                let modes = TabBar::new("appearance-mode")
                    .segmented()
                    .small()
                    .selected_index(if self.follow_system {
                        0
                    } else if cx.theme().is_dark() {
                        2
                    } else {
                        1
                    })
                    .children([
                        Tab::new().label("跟随系统").disabled(disabled),
                        Tab::new().label("浅色").disabled(disabled),
                        Tab::new().label("深色").disabled(disabled),
                    ])
                    .on_click(cx.listener(|this, index, window, cx| {
                        let mut next = this.current_preferences(cx);
                        next.mode = [
                            AppearanceMode::System,
                            AppearanceMode::Light,
                            AppearanceMode::Dark,
                        ][*index];
                        this.update_preferences(Some(next), window, cx);
                    }));
                content = content.child(self.setting_row(
                    "主题",
                    "选择浅色、深色，或随系统切换。",
                    modes,
                    compact,
                    cx,
                ));
                let sizes = TabBar::new("appearance-size")
                    .segmented()
                    .small()
                    .selected_index(if self.font_size == 13. {
                        0
                    } else if self.font_size == 16. {
                        2
                    } else {
                        1
                    })
                    .children([
                        Tab::new().label("紧凑").disabled(disabled),
                        Tab::new().label("默认").disabled(disabled),
                        Tab::new().label("宽松").disabled(disabled),
                    ])
                    .on_click(cx.listener(|this, index, window, cx| {
                        let mut next = this.current_preferences(cx);
                        next.text_size =
                            [TextSize::Compact, TextSize::Default, TextSize::Comfortable][*index];
                        this.update_preferences(Some(next), window, cx);
                    }));
                content = content.child(self.setting_row(
                    "文字大小",
                    "调整界面与对话的阅读尺寸。",
                    sizes,
                    compact,
                    cx,
                ));
                content = content.child(
                    self.setting_row(
                        "减少动态效果",
                        "关闭文字渐显和过渡动画。",
                        Switch::new("reduce-motion")
                            .checked(cx.reduce_motion())
                            .disabled(disabled)
                            .accessibility_label("减少动态效果")
                            .on_change(cx.listener(|this, checked, window, cx| {
                                let mut next = this.current_preferences(cx);
                                next.reduce_motion = Some(*checked);
                                this.update_preferences(Some(next), window, cx);
                            })),
                        compact,
                        cx,
                    ),
                );
            }
            SettingsPage::Shortcuts => {
                let command = if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                };
                for (action, key) in [
                    ("新建对话", format!("{command} N")),
                    ("搜索对话", format!("{command} F")),
                    ("命令与对话", format!("{command} K")),
                    ("打开 / 关闭终端", format!("{command} J")),
                    ("切换侧栏", format!("{command} B")),
                    ("打开设置", format!("{command} ,")),
                    ("发送消息", "Enter".into()),
                    ("插入换行", "Shift + Enter".into()),
                    ("返回对话 / 取消重命名", "Esc".into()),
                ] {
                    content = content.child(
                        div()
                            .flex()
                            .justify_between()
                            .py_4()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().text_sm().child(action))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(key),
                            ),
                    );
                }
            }
            SettingsPage::About => {
                content = content.child(div().text_sm().line_height(relative(1.6))
                    .child("当前为 Kivio 原生前端预览。你可以创建和切换对话、保留草稿、搜索与重命名，也可以体验流式回复与长对话阅读。"))
                    .child(div().mt_5().text_sm().line_height(relative(1.6)).text_color(cx.theme().muted_foreground)
                        .child("原生预览使用独立数据目录，支持真实模型对话；会话、草稿、分组与外观设置会在重启后保留。完整 Agent 与工作台仍在迁移中。"))
                    .child(div().mt_8().text_xs().text_color(cx.theme().muted_foreground).child("Kivio Native · 0.1.0"));
            }
        }
        div()
            .id("settings-content")
            .size_full()
            .overflow_y_scroll()
            .when(compact, |pane| pane.p_5())
            .when(!compact, |pane| pane.p_8())
            .flex()
            .justify_center()
            .child(content)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use crate::{preferences::Preferences, shell::Shell};
    use gpui_kit::{AppContext, TestAppContext, WindowOptions, test::TestWindowExt};

    #[gpui_kit::test]
    fn appearance_control_saves_reopens_and_retains_selection_on_failure(cx: &mut TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("appearance.json");
        std::fs::write(&path, r#"{"reduceMotion":false}"#).unwrap();
        let (handle, shell) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(data.path().to_owned(), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(handle, "secondary-comma");
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click("通用", cx);
                window.click("reduce-motion", cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(Preferences::load(&path).unwrap().reduce_motion, Some(true));
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.reduce_motion_override, Some(true));
            assert!(cx.reduce_motion());
        });

        let (_handle, reopened) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(data.path().to_owned(), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        reopened.read_with(cx, |shell, _| {
            assert_eq!(shell.reduce_motion_override, Some(true));
        });

        // A filesystem rejection must not leave the control claiming it saved.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        handle
            .update(cx, |_, window, cx| window.click("reduce-motion", cx))
            .unwrap();
        cx.run_until_parked();
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.reduce_motion_override, Some(true));
            assert!(cx.reduce_motion());
            assert!(!shell.preferences_busy);
        });
        assert!(path.is_dir());
    }
}
