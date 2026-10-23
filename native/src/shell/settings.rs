use super::*;

impl Shell {
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
            SettingsPage::Appearance => {
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
                        Tab::new().label("跟随系统"),
                        Tab::new().label("浅色"),
                        Tab::new().label("深色"),
                    ])
                    .on_click(cx.listener(|this, index, _, cx| {
                        this.appearance(
                            [None, Some(ThemeMode::Light), Some(ThemeMode::Dark)][*index],
                            cx,
                        );
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
                        Tab::new().label("紧凑"),
                        Tab::new().label("默认"),
                        Tab::new().label("宽松"),
                    ])
                    .on_click(cx.listener(|this, index, _, cx| {
                        this.font_size = [13., 14., 16.][*index];
                        this.apply_font(cx);
                        this.remeasure(cx);
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
                            .accessibility_label("减少动态效果")
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.reduce_motion_override = Some(*checked);
                                cx.set_reduce_motion(*checked);
                                cx.notify();
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
                        .child("模型连接和会话存储尚未接入。对话、草稿与外观设置仅保留在本次窗口，关闭后清除；不会读取已有 Kivio 资料。"))
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
