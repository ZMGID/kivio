use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, RopeExt, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    input::{InputEvent, MoveDown, MoveUp, Textarea, TextareaState},
};
use gpui_kit::{assets::IconName, prelude::FluentBuilder, *};

use crate::transcript::{Changed, RewindRequested, Transcript};

pub enum ConversationEvent {
    FirstMessage(SharedString),
    StatusChanged,
    Submitted,
}

/// A retained conversation pane: switching conversations never reconstructs
/// its draft, scroll state, transcript or in-flight task.
pub struct Preview {
    transcript: Entity<Transcript>,
    momo: Entity<crate::momo::Momo>,
    input: Entity<TextareaState>,
    attachments: Vec<std::path::PathBuf>,
    attachment_error: Option<SharedString>,
    picker: Option<Task<()>>,
    history: Option<(Vec<SharedString>, usize, SharedString)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConversationEvent> for Preview {}

impl Preview {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let transcript = cx.new(Transcript::new);
        transcript.update(cx, |view, cx| {
            view.load(0, cx);
            view.watch_window(window, cx);
        });
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("有问题尽管问…")
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                        InputEvent::Change => {
                            if this.history.as_ref().is_some_and(|(entries, index, _)| {
                                this.input.read(cx).value() != entries[*index]
                            }) {
                                this.history = None;
                            }
                            cx.notify();
                        }
                        InputEvent::Focus | InputEvent::Blur => cx.notify(),
                        _ => {}
                    }
                }),
                cx.subscribe_in(
                    &transcript,
                    window,
                    |this, _, request: &RewindRequested, window, cx| {
                        this.confirm_rewind(request.0, window, cx);
                    },
                ),
                cx.subscribe(&transcript, |_, _, _: &Changed, cx| {
                    cx.emit(ConversationEvent::StatusChanged);
                    cx.notify();
                }),
            ];
        input.update(cx, |state, cx| state.focus(window, cx));
        Self {
            transcript,
            momo: cx.new(|_| crate::momo::Momo::default()),
            input,
            attachments: Vec::new(),
            attachment_error: None,
            picker: None,
            history: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub fn load_sample(&self, count: usize, cx: &mut Context<Self>) {
        self.transcript.update(cx, |view, cx| view.load(count, cx));
    }

    pub fn remeasure(&self, cx: &mut Context<Self>) {
        self.transcript.update(cx, |view, cx| view.remeasure(cx));
        // Retained entity views also cache their own layout. Invalidate the
        // composer when the shell changes its available width.
        cx.notify();
    }

    pub fn is_streaming(&self, cx: &App) -> bool {
        self.transcript.read(cx).is_streaming()
    }

    fn add_attachments(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            if !self.attachments.contains(path) {
                self.attachments.push(path.clone());
            }
        }
        self.attachment_error = None;
        cx.notify();
    }

    fn input_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        if self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let key = &event.keystroke;
        if key.key == "escape" && self.is_streaming(cx) {
            self.transcript.update(cx, |view, cx| view.stop(cx));
            cx.stop_propagation();
        }
    }

    fn browse_history(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input.read(cx).focus_handle(cx).is_focused(window)
            || self.input.update(cx, |input, cx| {
                input.marked_text_range(window, cx).is_some()
            })
        {
            return;
        }
        let input = self.input.read(cx);
        let value = input.value();
        let selection = input.selected_range();
        if !selection.is_empty()
            || (value.contains('\n')
                && if up {
                    selection.start != 0
                } else {
                    selection.end != value.len()
                })
        {
            return;
        }
        if self.history.is_none() && up {
            let entries = self.transcript.read(cx).input_history();
            if entries.is_empty() {
                return;
            }
            let index = entries.len();
            self.history = Some((entries, index, value));
        }
        let Some((entries, index, draft)) = self.history.as_mut() else {
            return;
        };
        *index = if up {
            index.saturating_sub(1)
        } else {
            (*index + 1).min(entries.len())
        };
        let next = entries.get(*index).unwrap_or(draft).clone();
        if *index == entries.len() {
            self.history = None;
        }
        self.input.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            let end = input.text().offset_to_position(input.value().len());
            input.set_cursor_position(end, window, cx);
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn rewind(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(body) = self.transcript.update(cx, |view, cx| view.rewind(id, cx)) {
            self.history = None;
            self.input.update(cx, |input, cx| {
                input.set_value(body, window, cx);
                let end = input.text().offset_to_position(input.value().len());
                input.set_cursor_position(end, window, cx);
            });
        }
        window.close_dialog(cx);
    }

    fn confirm_rewind(&self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let keyboard = owner.clone();
            let confirm = owner.clone();
            dialog
                .title("回到这条提问？")
                .child(
                    "将移除这条提问及之后的回复，并把提问放回输入框。当前未发送的草稿也会被替换。",
                )
                .on_ok(move |_, window, cx| {
                    let _ = keyboard.update(cx, |this, cx| this.rewind(id, window, cx));
                    false
                })
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-rewind")
                                .ghost()
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-rewind")
                                .primary()
                                .label("回到这里")
                                .on_click(move |_, window, cx| {
                                    let _ =
                                        confirm.update(cx, |this, cx| this.rewind(id, window, cx));
                                }),
                        ),
                )
        });
    }

    fn choose_attachments(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("添加附件".into()),
        });
        self.picker = Some(cx.spawn(async move |this, cx| {
            let result = rx.await;
            let _ = this.update(cx, |this, cx| {
                this.picker = None;
                match result {
                    Ok(Ok(Some(paths))) => {
                        this.add_attachments(&paths, cx);
                    }
                    Ok(Ok(None)) => {}
                    _ => this.attachment_error = Some("无法打开文件选择器，请重试。".into()),
                }
                cx.notify();
            });
        }));
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.attachments.is_empty() {
            return;
        }
        let first = self.transcript.read(cx).count() == 0;
        let body = self.input.read(cx).value();
        if self
            .transcript
            .update(cx, |view, cx| view.send(body.clone(), cx))
        {
            if first {
                cx.emit(ConversationEvent::FirstMessage(body));
            } else {
                cx.emit(ConversationEvent::Submitted);
            }
            self.history = None;
            self.input.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
        }
    }

    fn heading(&self) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_1()
            .mb_6()
            .child(self.momo.clone())
            .child(
                div()
                    .text_size(rems(1.57))
                    .font_weight(FontWeight::MEDIUM)
                    .child("今天想聊点什么"),
            )
            .into_any_element()
    }

    fn composer(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let streaming = self.is_streaming(cx);
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        div()
            .v_flex()
            .w_full()
            .gap_2()
            .when(!self.attachments.is_empty(), |column| {
                column
                    .child(
                        div()
                            .id("attachment-strip")
                            .flex()
                            .gap_2()
                            .overflow_x_scroll()
                            .children(self.attachments.iter().map(|path| {
                                let remove = path.clone();
                                let name = path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                div()
                                    .flex()
                                    .items_center()
                                    .flex_shrink_0()
                                    .gap_2()
                                    .p_2()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().muted)
                                    .child(Icon::new(IconName::File).small())
                                    .child(
                                        div()
                                            .v_flex()
                                            .max_w(rems(14.))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .child(name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("待接入附件解析"),
                                            ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "attachment-{}",
                                            path.display()
                                        )))
                                        .small()
                                        .ghost()
                                        .icon(IconName::X)
                                        .tooltip("移除附件")
                                        .accessibility_label(format!("移除附件 {name}"))
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.attachments.retain(|path| path != &remove);
                                                cx.notify();
                                            }),
                                        ),
                                    )
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("当前仅预览附件；移除后可发送离线示例。"),
                    )
            })
            .when_some(self.attachment_error.clone(), |column, error| {
                column.child(div().text_xs().text_color(cx.theme().danger).child(error))
            })
            .child(
                div()
                    .id("composer-surface")
                    .drag_over::<ExternalPaths>(|style, _, _, cx| {
                        style.bg(cx.theme().muted).border_color(cx.theme().primary)
                    })
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.add_attachments(paths.paths(), cx);
                        this.input.update(cx, |input, cx| input.focus(window, cx));
                    }))
                    .flex()
                    .items_end()
                    .w_full()
                    .min_h(rems(4.))
                    .p_2()
                    .gap_2()
                    .rounded(rems(0.85))
                    .border_1()
                    .border_color(if focused {
                        cx.theme().primary.opacity(0.55)
                    } else {
                        cx.theme().input
                    })
                    .bg(cx.theme().background)
                    .shadow_xs()
                    .child(
                        div().flex_1().min_w_0().py_1().text_size(rems(1.07)).child(
                            Textarea::new(&self.input)
                                .appearance(false)
                                .bordered(false)
                                .aria_label("消息"),
                        ),
                    )
                    .child(div().pb_1().child(if streaming {
                        Button::new("stop")
                            .small()
                            .icon(IconName::Square)
                            .accessibility_label("停止生成")
                            .tooltip("停止生成 · Esc")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.transcript.update(cx, |view, cx| view.stop(cx));
                            }))
                    } else {
                        Button::new("send")
                            .small()
                            .primary()
                            .icon(IconName::ArrowUp)
                            .accessibility_label("发送消息")
                            .tooltip("发送 · Enter")
                            .disabled(
                                self.input.read(cx).value().trim().is_empty()
                                    || !self.attachments.is_empty(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("attachments")
                            .small()
                            .ghost()
                            .icon(IconName::Plus)
                            .accessibility_label("添加附件")
                            .tooltip("添加附件，也可拖入输入框")
                            .disabled(self.picker.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.choose_attachments(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if streaming {
                                "Esc 停止 · 可继续编辑草稿"
                            } else {
                                "Shift + Enter 换行"
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.transcript.read(cx).count() == 0;
        div()
            .v_flex()
            .size_full()
            .min_w_0()
            .capture_key_down(cx.listener(Self::input_key))
            .capture_action(
                cx.listener(|this, _: &MoveUp, window, cx| this.browse_history(true, window, cx)),
            )
            .capture_action(
                cx.listener(|this, _: &MoveDown, window, cx| {
                    this.browse_history(false, window, cx)
                }),
            )
            .on_action(|_: &gpui_kit::base::input::Enter, _, _| {})
            .child(if empty {
                div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .justify_center()
                    .items_center()
                    .px_6()
                    .pb_16()
                    .child(
                        div()
                            .v_flex()
                            .w_full()
                            .max_w(rems(56.))
                            .child(self.heading())
                            .child(self.composer(window, cx)),
                    )
                    .into_any_element()
            } else {
                div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_h_0().child(self.transcript.clone()))
                    .child(
                        div().px_6().pt_3().pb_4().flex().justify_center().child(
                            div()
                                .w_full()
                                .max_w(rems(56.))
                                .child(self.composer(window, cx)),
                        ),
                    )
                    .into_any_element()
            })
    }
}
#[cfg(test)]
mod tests {
    use super::Preview;
    use gpui_kit::{AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, px, size};

    // The application's actual input subscription must submit once, preserve a
    // draft while busy, and treat Shift+Enter as editing rather than sending.
    #[gpui_kit::test]
    fn keyboard_submission_preserves_the_next_draft(cx: &mut TestAppContext) {
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(900.), px(680.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Preview::new(window, cx)),
            )
            .unwrap()
        });
        cx.simulate_input(window, "你好 🌿");
        cx.simulate_keystrokes(window, "shift-enter");
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(view.transcript.read(cx).count(), 0);
            assert_eq!(view.input.read(cx).value(), "你好 🌿\n");
        });
        cx.simulate_input(window, "第二行");
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(view.transcript.read(cx).count(), 2);
            assert!(view.transcript.read(cx).is_streaming());
            assert_eq!(view.input.read(cx).value(), "");
        });
        cx.simulate_input(window, "下一条草稿");
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(view.transcript.read(cx).count(), 2);
            assert_eq!(view.input.read(cx).value(), "下一条草稿");
        });
    }
    // Original composer contract: history browsing preserves the unfinished
    // draft; Esc only cancels generation when the editor owns focus.
    #[gpui_kit::test]
    fn history_and_rewind_preserve_text_until_explicit_confirmation(cx: &mut TestAppContext) {
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Preview::new(window, cx))
            })
            .unwrap()
        });
        cx.simulate_input(window, "第一条提问\n包含第二行 🌿");
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        cx.simulate_keystrokes(window, "escape");
        view.read_with(cx, |view, cx| assert!(!view.is_streaming(cx)));
        cx.simulate_input(window, "未发送的草稿");
        cx.simulate_keystrokes(window, "up");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.input.read(cx).value(), "第一条提问\n包含第二行 🌿")
        });
        cx.simulate_keystrokes(window, "down");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.input.read(cx).value(), "未发送的草稿")
        });
        window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.confirm_rewind(0, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "escape");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.transcript.read(cx).count(), 2);
            assert_eq!(view.input.read(cx).value(), "未发送的草稿");
        });
        window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.confirm_rewind(0, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "enter");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.transcript.read(cx).count(), 0);
            assert_eq!(view.input.read(cx).value(), "第一条提问\n包含第二行 🌿");
        });
        cx.simulate_keystrokes(window, "enter");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.transcript.read(cx).count(), 2)
        });
    }
}
