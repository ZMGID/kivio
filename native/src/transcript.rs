mod details;
pub(crate) use details::fork_marker;
use std::time::Duration;

use crate::streaming_markdown::StreamingMarkdown;

use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    menu::{ContextMenuExt, PopupMenuItem},
    message_scroller::{MessageScroller, MessageScrollerState},
    text::{TextView, TextViewStyle},
};
use gpui_kit::{assets::IconName, prelude::FluentBuilder, *};

const RESPONSE: &str = "这是离线生成的示例回复，用于观察原生界面的输入、滚动和流式呈现。\n\n你可以在回复过程中向上滚动，阅读位置会保留；点击“回到最新”后继续跟随输出。停止后，已经显示的文字仍然保留。\n\n### 原生交互\n\n- 中文、Emoji 和多行文本由原生输入控件处理。\n- 历史消息使用动态高度虚拟列表。\n- 更新只作用于当前回复，输入区可以继续编辑。\n\n```rust\nfn main() {\n    println!(\"你好，Kivio\");\n}\n```\n\n这段内容不代表模型回答，也不会连接供应商或访问你的资料。";

struct Message {
    record: crate::conversation_data::ChatMessage,
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
            record: crate::conversation_data::ChatMessage::text(
                if user { "user" } else { "assistant" },
                body.to_string(),
            ),
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
pub struct RetryRequested;
#[derive(Clone, Copy)]
pub enum MessageAction {
    Edit,
    Delete,
    Fork,
    Rewind,
}
#[derive(Clone)]
pub struct MessageActionRequested {
    pub message_id: String,
    pub action: MessageAction,
}

/// Owns the visible transcript and cancels its request when stopped or dropped.
pub struct Transcript {
    messages: Vec<Message>,
    tool_folds: std::collections::HashMap<String, bool>,
    scroller: Entity<MessageScrollerState>,
    next_id: usize,
    attachment_directory: Option<std::path::PathBuf>,
    sample_mode: bool,
    stream_id: Option<usize>,
    task: Option<Task<()>>,
    markdown: Option<StreamingMarkdown>,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<MessageActionRequested> for Transcript {}
impl EventEmitter<Changed> for Transcript {}
impl EventEmitter<RetryRequested> for Transcript {}
impl EventEmitter<RewindRequested> for Transcript {}

impl Transcript {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let subscription = cx.observe(&scroller, |_, _, cx| cx.notify());
        Self {
            messages: Vec::new(),
            tool_folds: Default::default(),
            scroller,
            next_id: 0,
            attachment_directory: None,
            sample_mode: false,
            stream_id: None,
            task: None,
            markdown: None,
            subscriptions: vec![subscription],
        }
    }

    pub fn set_attachment_directory(&mut self, directory: std::path::PathBuf) {
        self.attachment_directory = Some(directory);
    }

    pub fn watch_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions
            .push(cx.observe_window_bounds(window, |this, _, cx| this.remeasure(cx)));
    }

    pub fn canonical_id(&self, id: usize) -> Option<String> {
        self.messages
            .iter()
            .find(|message| message.id == id)
            .map(|message| message.record.id.clone())
    }

    pub fn count(&self) -> usize {
        self.messages.len()
    }
    pub fn is_streaming(&self) -> bool {
        self.stream_id.is_some()
    }

    pub fn reveal_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .messages
            .iter()
            .position(|message| message.record.id == id)
        {
            self.messages[index].expanded = true;
            self.scroller.update(cx, |state, cx| {
                state.remeasure_items(index..index + 1, cx);
                state.scroll_to_item(index, cx);
            });
            cx.notify();
        }
    }

    pub fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.scroller.update(cx, |state, cx| state.remeasure(cx));
    }

    pub fn set_sample_mode(&mut self, sample: bool) {
        self.sample_mode = sample;
    }

    pub fn load(&mut self, count: usize, cx: &mut Context<Self>) {
        self.sample_mode = true;
        self.tool_folds.clear();
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

    pub fn snapshot(&self) -> Vec<crate::conversation_data::ChatMessage> {
        self.messages
            .iter()
            .map(|message| {
                let mut record = message.record.clone();
                record.content = message.body.to_string();
                if message.stopped {
                    record.stream_outcome = Some("cancelled".into());
                }
                record
            })
            .collect()
    }

    pub fn restore(
        &mut self,
        messages: Vec<crate::conversation_data::ChatMessage>,
        cx: &mut Context<Self>,
    ) {
        self.stop(cx);
        self.tool_folds
            .retain(|id, _| messages.iter().any(|message| &message.id == id));
        self.messages = messages
            .into_iter()
            .map(|record| {
                let mut message = Message::new(
                    self.next_id,
                    record.role == "user",
                    record.content.clone().into(),
                );
                self.next_id += 1;
                message.stopped = record.stream_outcome.as_deref() == Some("cancelled");
                message.record = record;
                message
            })
            .collect();
        self.scroller
            .update(cx, |state, cx| state.reset(self.messages.len(), cx));
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

    pub fn send_sample(&mut self, body: SharedString, cx: &mut Context<Self>) -> bool {
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

    pub fn send(
        &mut self,
        body: SharedString,
        attachments: Vec<crate::conversation_data::Attachment>,
        provider: crate::provider_settings::ModelProvider,
        model: String,
        conversation_id: String,
        catalog_dir: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        if self.is_streaming() || (body.trim().is_empty() && attachments.is_empty()) {
            return Ok(false);
        }
        let mut user = Message::new(self.next_id, true, body);
        user.record.attachments = attachments;
        let mut history = self.snapshot();
        history.push(user.record.clone());
        let run = crate::chat_runtime::start(
            provider,
            model,
            history,
            conversation_id.clone(),
            catalog_dir.join("runtime"),
        )?;
        self.attachment_directory =
            Some(catalog_dir.join(format!("{conversation_id}_attachments")));
        self.sample_mode = false;
        let reply_id = self.next_id + 1;
        self.next_id += 2;
        self.messages.push(user);
        self.messages.push(Message::new(reply_id, false, "".into()));
        self.stream_id = Some(reply_id);
        self.markdown = Some(StreamingMarkdown::new());
        self.scroller.update(cx, |state, cx| {
            state.append(2, cx);
            state.scroll_to_end(cx);
        });
        cx.emit(Changed);
        cx.notify();
        self.receive(run, reply_id, cx);
        Ok(true)
    }

    pub fn retry(
        &mut self,
        provider: crate::provider_settings::ModelProvider,
        model: String,
        conversation_id: String,
        catalog_dir: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        if self.is_streaming() || self.messages.last().is_none_or(|m| m.user) {
            return Ok(false);
        }
        let mut history = self.snapshot();
        history.pop();
        let run = crate::chat_runtime::start(
            provider,
            model,
            history,
            conversation_id.clone(),
            catalog_dir.join("runtime"),
        )?;
        self.attachment_directory =
            Some(catalog_dir.join(format!("{conversation_id}_attachments")));
        self.sample_mode = false;
        let ix = self.messages.len() - 1;
        let id = self.next_id;
        self.next_id += 1;
        self.messages[ix] = Message::new(id, false, "".into());
        self.messages[ix].record.run_entry = Some("regenerate".into());
        self.stream_id = Some(id);
        self.markdown = Some(StreamingMarkdown::new());
        self.scroller
            .update(cx, |state, cx| state.remeasure_items(ix..ix + 1, cx));
        cx.emit(Changed);
        cx.notify();
        self.receive(run, id, cx);
        Ok(true)
    }

    fn receive(
        &mut self,
        run: crate::chat_runtime::ChatRun,
        reply_id: usize,
        cx: &mut Context<Self>,
    ) {
        self.task = Some(cx.spawn(async move |owner, cx| {
            while let Ok(event) = run.events.recv().await {
                let terminal = matches!(
                    &event,
                    crate::chat_runtime::ChatEvent::Finished(_)
                        | crate::chat_runtime::ChatEvent::Failed(_)
                );
                let keep = owner
                    .update(cx, |this, cx| {
                        if this.stream_id != Some(reply_id) {
                            return false;
                        }
                        let ix = this.messages.len() - 1;
                        let message = &mut this.messages[ix];
                        match event {
                            crate::chat_runtime::ChatEvent::Delta(chunk) => {
                                let markdown =
                                    this.markdown.as_mut().expect("active request owns parser");
                                markdown.append(&chunk);
                                message.body = markdown.source().to_owned().into();
                                message.display = Some(markdown.display().into());
                            }
                            crate::chat_runtime::ChatEvent::Reasoning(chunk) => {
                                message
                                    .record
                                    .reasoning
                                    .get_or_insert_default()
                                    .push_str(&chunk);
                            }
                            crate::chat_runtime::ChatEvent::Finished(result) => {
                                message.body = result.text.into();
                                message.record.reasoning = result.reasoning;
                                if result.finish_reason.as_deref() == Some("length") {
                                    message.record.degraded =
                                        Some(crate::conversation_data::DegradedAnswer {
                                            kind: "max_tokens".into(),
                                            reason: "回复达到模型输出上限，可继续提问。".into(),
                                            detail: None,
                                            tool_summaries: Vec::new(),
                                            text: String::new(),
                                        });
                                }
                                message.record.model_messages = result.provider_messages;
                                message.record.usage = result.usage.clone();
                                message.record.anchor_usage = result.usage;
                                message.record.stream_outcome = Some("completed".into());
                                this.stream_id = None;
                                this.markdown = None;
                                message.display = None;
                            }
                            crate::chat_runtime::ChatEvent::Failed(error) => {
                                message.record.stream_outcome = Some("error".into());
                                message.record.degraded =
                                    Some(crate::conversation_data::DegradedAnswer {
                                        kind: "unknown".into(),
                                        reason: error,
                                        detail: None,
                                        tool_summaries: Vec::new(),
                                        text: String::new(),
                                    });
                                this.stream_id = None;
                                this.markdown = None;
                                message.display = None;
                            }
                        }
                        this.scroller
                            .update(cx, |state, cx| state.remeasure_items(ix..ix + 1, cx));
                        if terminal {
                            cx.emit(Changed);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep || terminal {
                    return;
                }
            }
            let _ = owner.update(cx, |this, cx| {
                if this.stream_id == Some(reply_id) {
                    this.stop(cx);
                }
            });
        }));
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
                    let tools = (!this.messages[ix].record.tool_calls.is_empty()).then(|| this.tool_details(ix, cx));
                    let message = &this.messages[ix];
                    let copy = message.body.clone();
                    let context_copy = copy.clone();
                    let id = message.id;
                    let foldable = message.collapsed.is_some();
                    let expanded = message.expanded;
                    let user = message.user;
                    let rewind = cx.entity().downgrade();
                    let context_rewind = rewind.clone();
                    let message_actions = rewind.clone();
                    let canonical_id = message.record.id.clone();
                    let can_mutate = !this.is_streaming();
                    let retry = rewind.clone();
                    let can_retry = !this.sample_mode && !user && ix + 1 == this.messages.len() && !this.is_streaming();
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
                            for (label, action) in [("从这里建立分支", MessageAction::Fork)] {
                                let owner = message_actions.clone();
                                let message_id = canonical_id.clone();
                                menu = menu.separator().item(PopupMenuItem::new(label).disabled(!can_mutate).on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |_, cx| cx.emit(MessageActionRequested {message_id: message_id.clone(), action}));
                                }));
                            }
                            if !user {
                                for (label, action) in [("编辑回答", MessageAction::Edit), ("删除回答", MessageAction::Delete)] {
                                    let owner = message_actions.clone();
                                    let message_id = canonical_id.clone();
                                    menu = menu.item(PopupMenuItem::new(label).disabled(!can_mutate).on_click(move |_, _, cx| {
                                        let _ = owner.update(cx, |_, cx| cx.emit(MessageActionRequested {message_id: message_id.clone(), action}));
                                    }));
                                }
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
                                        .when_some(
                                            message
                                                .record
                                                .reasoning
                                                .as_ref()
                                                .filter(|text| !text.is_empty()),
                                            |body, reasoning| {
                                                body.child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(
                                                            TextView::markdown(
                                                                ("reasoning", id),
                                                                reasoning.clone(),
                                                            )
                                                            .selectable(true),
                                                        ),
                                                )
                                            },
                                        )
                                        .when_some(tools, |body, tools| body.child(tools))
                                        .when(!message.record.attachments.is_empty(), |body| {
                                            body.child(div().flex().flex_wrap().gap_2().children(message.record.attachments.iter().map(|attachment| {
                                                let attachment = attachment.clone();
                                                let directory = this.attachment_directory.clone();
                                                Button::new(SharedString::from(format!("message-{id}-attachment-{}", attachment.id)))
                                                    .small().ghost().icon(IconName::File).label(attachment.name.clone())
                                                    .tooltip("打开附件")
                                                    .on_click(move |_, window, cx| {
                                                        if let Some(content) = &attachment.content {
                                                            let content = content.clone();
                                                            let name = attachment.name.clone();
                                                            window.open_dialog(cx, move |dialog, _, _| dialog.title(name.clone()).child(TextView::markdown("attachment-text", content.clone()).selectable(true)));
                                                        } else {
                                                            match kivio_model_runtime::attachment_files::resolve_attachment_file_path_in_dir(directory.as_deref(), &attachment.path) {
                                                                Ok(path) => cx.open_with_system(&path),
                                                                Err(error) => window.push_notification(gpui_kit::component::notification::Notification::error(error), cx),
                                                            }
                                                        }
                                                    })
                                            })))
                                        })
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
                                        .when_some(
                                            message.record.degraded.as_ref(),
                                            |row, failure| {
                                                row.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().danger)
                                                        .child(failure.reason.clone()),
                                                )
                                            },
                                        )
                                        .when(can_retry, |row| {
                                            row.child(
                                                Button::new(("retry-reply", id))
                                                    .small()
                                                    .ghost()
                                                    .label("重新生成")
                                                    .on_click(move |_, _, cx| {
                                                        let _ = retry.update(cx, |_, cx| {
                                                            cx.emit(RetryRequested)
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
            assert!(view.send_sample("第一条".into(), cx));
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
            assert!(view.send_sample("第二条".into(), cx));
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
