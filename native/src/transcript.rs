use std::time::Duration;

use crate::streaming_markdown::StreamingMarkdown;

use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    menu::{ContextMenuExt, PopupMenuItem},
    message_scroller::{MessageScroller, MessageScrollerState},
    text::{TextView, TextViewStyle},
};
use gpui_kit::{assets::IconName, prelude::FluentBuilder, *};

const RESPONSE: &str = "这是离线生成的示例回复，用于观察原生界面的输入、滚动和流式呈现。\n\n你可以在回复过程中向上滚动，阅读位置会保留；点击“回到最新”后继续跟随输出。停止后，已经显示的文字仍然保留。\n\n### 原生交互\n\n- 中文、Emoji 和多行文本由原生输入控件处理。\n- 历史消息使用动态高度虚拟列表。\n- 更新只作用于当前回复，输入区可以继续编辑。\n\n```rust\nfn main() {\n    println!(\"你好，Kivio\");\n}\n```\n\n这段内容不代表模型回答，也不会连接供应商或访问你的资料。";

struct Message {
    id: usize,
    user: bool,
    body: SharedString,
    display: Option<SharedString>,
    stopped: bool,
    expanded: bool,
    collapsed: Option<SharedString>,
}

impl Message {
    fn new(id: usize, user: bool, body: SharedString) -> Self {
        // User messages are literal text. Kit's HTML text view preserves line
        // breaks without treating user-authored Markdown as formatting.
        let html = |text: &str| -> SharedString {
            let escaped = text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\n', "<br>");
            format!("<p>{escaped}</p>").into()
        };
        let display = user.then(|| html(&body));
        // Cache a bounded excerpt: a folded message must not lay out its full
        // payload on every resize. Copy and rewind always use canonical body.
        let collapsed =
            (user && (body.lines().count() > 8 || body.chars().count() > 400)).then(|| {
                let excerpt: String = body
                    .lines()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .chars()
                    .take(400)
                    .collect();
                html(&format!("{excerpt}…"))
            });
        Self {
            id,
            user,
            body,
            display,
            stopped: false,
            expanded: false,
            collapsed,
        }
    }
}

pub struct Changed;
pub struct RewindRequested(pub usize);

/// Owns only the offline preview, never a production chat or process.
pub struct Transcript {
    messages: Vec<Message>,
    scroller: Entity<MessageScrollerState>,
    next_id: usize,
    stream_id: Option<usize>,
    task: Option<Task<()>>,
    markdown: Option<StreamingMarkdown>,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<Changed> for Transcript {}
impl EventEmitter<RewindRequested> for Transcript {}

impl Transcript {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let subscription = cx.observe(&scroller, |_, _, cx| cx.notify());
        Self {
            messages: Vec::new(),
            scroller,
            next_id: 0,
            stream_id: None,
            task: None,
            markdown: None,
            subscriptions: vec![subscription],
        }
    }

    pub fn watch_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions
            .push(cx.observe_window_bounds(window, |this, _, cx| this.remeasure(cx)));
    }

    pub fn count(&self) -> usize {
        self.messages.len()
    }
    pub fn is_streaming(&self) -> bool {
        self.stream_id.is_some()
    }

    pub fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.scroller.update(cx, |state, cx| state.remeasure(cx));
    }

    pub fn load(&mut self, count: usize, cx: &mut Context<Self>) {
        self.stop(cx);
        self.messages = (0..count).map(|ix| {
            let id = self.next_id;
            self.next_id += 1;
            let (user, body) = if count == 3 {
                match ix {
                    0 => (false, "## 从一次自然的对话开始\n\n这里是 Kivio 的原生预览。你可以编辑中文、选择文字，或切换长对话样本观察滚动。所有内容只保留在本次窗口中。".into()),
                    1 => (true, "长对话里输入和滚动还能保持流畅吗？".into()),
                    _ => (false, "可以先试试右上角的 **1,000 条**或 **10,000 条**样本。\n\n下面的输入框支持多行编辑。发送后会播放一段离线回复，过程中仍可输入下一条内容。".into()),
                }
            } else {
                let body = if ix % 2 == 0 {
                    format!("第 {} 条：中文输入、长消息与滚动位置如何保持一致？", ix + 1)
                } else {
                    format!("### 记录 {}\n\n{}\n\n```rust\nlet message = \"你好，世界 🌿\";\n```", ix + 1,
                        "这是一段固定的合成内容，包含中文、English 与 Emoji。拖动窗口改变宽度，可以观察文本换行和阅读位置。\n\n".repeat(ix % 4 + 1))
                };
                (ix % 2 == 0, body.into())
            };
            Message::new(id, user, body)
        }).collect();
        self.scroller.update(cx, |state, cx| state.reset(count, cx));
        cx.emit(Changed);
        cx.notify();
    }

    pub fn input_history(&self) -> Vec<SharedString> {
        self.messages
            .iter()
            .filter(|message| message.user)
            .map(|message| message.body.clone())
            .collect()
    }

    pub fn rewind(&mut self, id: usize, cx: &mut Context<Self>) -> Option<SharedString> {
        let ix = self
            .messages
            .iter()
            .position(|message| message.id == id && message.user)?;
        let body = self.messages[ix].body.clone();
        self.stop(cx);
        self.messages.truncate(ix);
        self.scroller.update(cx, |state, cx| state.reset(ix, cx));
        cx.emit(Changed);
        cx.notify();
        Some(body)
    }

    fn toggle_message(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(ix) = self.messages.iter().position(|message| message.id == id) {
            self.messages[ix].expanded = !self.messages[ix].expanded;
            self.scroller
                .update(cx, |state, cx| state.remeasure_items(ix..ix + 1, cx));
            cx.notify();
        }
    }

    pub fn send(&mut self, body: SharedString, cx: &mut Context<Self>) -> bool {
        if self.is_streaming() || body.trim().is_empty() {
            return false;
        }
        let id = self.next_id;
        self.next_id += 2;
        self.messages.push(Message::new(id, true, body));
        self.messages.push(Message::new(id + 1, false, "".into()));
        let reply_id = id + 1;
        self.stream_id = Some(reply_id);
        self.markdown = Some(StreamingMarkdown::new());
        self.scroller.update(cx, |state, cx| {
            state.append(2, cx);
            state.scroll_to_end(cx);
        });
        cx.emit(Changed);
        cx.notify();
        self.task = Some(cx.spawn(async move |view, cx| {
            let mut chars = RESPONSE.chars();
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let chunk: String = chars.by_ref().take(12).collect();
                let done = chunk.is_empty();
                let keep_going = view
                    .update(cx, |this, cx| {
                        if this.stream_id != Some(reply_id) {
                            return false;
                        }
                        let ix = this.messages.len() - 1;
                        if done {
                            this.stream_id = None;
                            this.markdown = None;
                            this.messages[ix].display = None;
                            this.scroller
                                .update(cx, |state, cx| state.remeasure_items(ix..ix + 1, cx));
                            cx.emit(Changed);
                        } else {
                            let markdown =
                                this.markdown.as_mut().expect("active stream owns parser");
                            markdown.append(&chunk);
                            this.messages[ix].body = markdown.source().to_owned().into();
                            this.messages[ix].display = Some(markdown.display().into());
                            this.scroller.update(cx, |state, cx| {
                                state.remeasure_items(ix..ix + 1, cx);
                            });
                        }
                        cx.notify();
                        !done
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
        true
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.markdown = None;
        if self.stream_id.take().is_some() {
            let ix = self.messages.len() - 1;
            self.messages[ix].display = None;
            self.messages[ix].stopped = true;
            self.scroller
                .update(cx, |state, cx| state.remeasure_items(ix..ix + 1, cx));
            cx.emit(Changed);
            cx.notify();
        }
    }
}

impl Render for Transcript {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        MessageScroller::new("transcript", self.scroller.clone(), move |ix, _, cx| {
            owner
                .update(cx, |this, cx| {
                    let message = &this.messages[ix];
                    let copy = message.body.clone();
                    let context_copy = copy.clone();
                    let id = message.id;
                    let foldable = message.collapsed.is_some();
                    let expanded = message.expanded;
                    let user = message.user;
                    let rewind = cx.entity().downgrade();
                    let context_rewind = rewind.clone();
                    let text = (if message.user {
                        TextView::html(
                            ("message", message.id),
                            if !expanded {
                                message.collapsed.as_ref()
                            } else {
                                None
                            }
                            .or(message.display.as_ref())
                            .unwrap_or(&message.body)
                            .clone(),
                        )
                    } else {
                        TextView::markdown(
                            ("message", message.id),
                            message.display.as_ref().unwrap_or(&message.body).clone(),
                        )
                    })
                    .on_link_click(|url, _, _, cx| {
                        if url.as_ref() != zeron_markdown::mend::PENDING_LINK_URL {
                            cx.open_url(url);
                        }
                    })
                    .style(
                        TextViewStyle::default()
                            .paragraph_gap(rems(0.85))
                            .code_block({
                                let mut style = StyleRefinement::default().max_h(rems(24.));
                                style.overflow.y = Some(Overflow::Scroll);
                                style
                            }),
                    )
                    .selectable(true)
                    .scrollable(false)
                    .stream_fade(this.stream_id == Some(message.id));
                    div()
                        .id(("message-row", id))
                        .context_menu(move |mut menu, _, _| {
                            let copy = context_copy.clone();
                            menu = menu.item(PopupMenuItem::new("复制整条消息").on_click(
                                move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy.to_string(),
                                    ))
                                },
                            ));
                            if user {
                                let owner = context_rewind.clone();
                                menu = menu.separator().item(
                                    PopupMenuItem::new("回到这里，重新编辑").on_click(
                                        move |_, _, cx| {
                                            let _ = owner
                                                .update(cx, |_, cx| cx.emit(RewindRequested(id)));
                                        },
                                    ),
                                );
                            }
                            menu
                        })
                        .w_full()
                        .px_6()
                        .py_3()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .w_full()
                                .max_w(rems(56.))
                                .v_flex()
                                .gap_2()
                                .when(message.user, |row| row.items_end())
                                .child(
                                    div()
                                        .v_flex()
                                        .min_w_0()
                                        .when(message.user, |body| {
                                            body.max_w(relative(0.85))
                                                .px_4()
                                                .py_2()
                                                .rounded(rems(0.85))
                                                .bg(cx.theme().muted)
                                        })
                                        .when(!message.user, |body| body.w_full())
                                        .child(
                                            div()
                                                .when(foldable && !expanded, |body| {
                                                    body.max_h(rems(12.)).overflow_hidden()
                                                })
                                                .child(text),
                                        )
                                        .when(foldable, |body| {
                                            body.child(
                                                Button::new(("expand-message", id))
                                                    .small()
                                                    .ghost()
                                                    .label(if expanded {
                                                        "收起长消息"
                                                    } else {
                                                        "展开完整消息"
                                                    })
                                                    .icon(if expanded {
                                                        IconName::ChevronUp
                                                    } else {
                                                        IconName::ChevronDown
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.toggle_message(id, cx)
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Clipboard::new(("copy-message", message.id))
                                                .small()
                                                .value(copy)
                                                .tooltip("复制消息")
                                                .accessibility_label("复制消息"),
                                        )
                                        .when(user, |row| {
                                            row.child(
                                                Button::new(("rewind-message", id))
                                                    .small()
                                                    .ghost()
                                                    .icon(IconName::SquarePen)
                                                    .tooltip("回到这里，重新编辑")
                                                    .accessibility_label("回到这里，重新编辑")
                                                    .on_click(move |_, _, cx| {
                                                        let _ = rewind.update(cx, |_, cx| {
                                                            cx.emit(RewindRequested(id))
                                                        });
                                                    }),
                                            )
                                        })
                                        .when(message.stopped, |row| {
                                            row.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("已停止"),
                                            )
                                        }),
                                ),
                        )
                        .into_any_element()
                })
                .unwrap_or_else(|_| div().into_any_element())
        })
        .with_jump_button_label("回到最新")
        .with_jump_button_renderer(|button| button.label("回到最新"))
        .with_jump_button_transition(cx.theme().motion.duration_fast)
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::Transcript;
    use gpui_kit::{AppContext, TestAppContext};
    use std::time::Duration;

    // Cancellation and replacement must keep visible partial content and never
    // deliver a previous stream into the newly selected conversation.
    #[gpui_kit::test]
    fn stopping_and_replacing_a_stream_rejects_late_output(cx: &mut TestAppContext) {
        let view = cx.update(|cx| cx.new(Transcript::new));
        view.update(cx, |view, cx| {
            assert!(view.send("第一条".into(), cx));
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        let partial = view.update(cx, |view, cx| {
            view.stop(cx);
            let partial = view.messages.last().unwrap().body.clone();
            assert!(!partial.is_empty());
            partial
        });
        cx.executor().advance_clock(Duration::from_secs(10));
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            assert_eq!(view.messages.last().unwrap().body, partial);
            assert!(view.send("第二条".into(), cx));
            view.load(3, cx);
        });
        cx.executor().advance_clock(Duration::from_secs(10));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.messages.len(), 3);
            assert!(!view.is_streaming());
            assert!(view.messages[2].body.contains("1,000 条"));
        });
    }
}
