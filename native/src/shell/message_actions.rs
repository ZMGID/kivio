//! Native message commands share the conversation's write queue and canonical rules.
use super::*;
use crate::conversation_data::Conversation as Record;
use crate::transcript::{MessageAction, MessageActionRequested};
use gpui_kit::component::{
    input::{Textarea, TextareaState},
    notification::Notification,
};
use kivio_model_runtime::{
    conversation_mutations as mutations, conversation_resources as resources,
};

pub(super) struct MessageEditor {
    chat_id: usize,
    message_id: String,
    input: Entity<TextareaState>,
    error: Option<String>,
}

enum Mutation {
    Edit(String),
    Delete,
    Rewind,
    Fork,
}

impl Shell {
    fn message_action_ready(&self, chat_id: usize, cx: &App) -> bool {
        self.persistence.ready
            && !self.persistence.busy
            && self.persistence.error.is_none()
            && self
                .conversations
                .iter()
                .find(|chat| chat.id == chat_id)
                .is_some_and(|chat| {
                    !chat.record.agent_runtime.is_external()
                        && !chat.view.read(cx).is_busy(cx)
                        && !chat.view.read(cx).is_streaming(cx)
                })
    }

    pub(super) fn handle_message_action(
        &mut self,
        chat_id: usize,
        request: &MessageActionRequested,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.message_action_ready(chat_id, cx) {
            window.push_notification(
                Notification::warning("请等待当前操作完成，或先处理保存错误。"),
                cx,
            );
            return;
        }
        let (record, _) = self.capture_record(chat_id, cx);
        let Ok(index) = mutations::find_message_index(&record, &request.message_id) else {
            return;
        };
        let message_id = request.message_id.clone();
        match request.action {
            MessageAction::Edit => {
                let input = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(4, 14)
                        .default_value(record.messages[index].content.clone())
                });
                self.message_editor = Some(MessageEditor {
                    chat_id,
                    message_id,
                    input: input.clone(),
                    error: None,
                });
                let owner = cx.weak_entity();
                window.open_dialog(cx, move |dialog, _, cx| {
                    let confirm = owner.clone();
                    let close = owner.clone();
                    let content = owner
                        .update(cx, |this, cx| {
                            let editor = this.message_editor.as_ref()?;
                            Some(
                                div()
                                    .v_flex()
                                    .gap_3()
                                    .child(Textarea::new(&editor.input).aria_label("编辑回答"))
                                    .when_some(editor.error.clone(), |view, error| {
                                        view.child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().danger)
                                                .child(error),
                                        )
                                    })
                                    .child(
                                        div().flex().justify_end().child(
                                            Button::new("save-message-edit")
                                                .primary()
                                                .label("保存")
                                                .disabled(this.persistence.busy)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.save_message_edit(window, cx)
                                                })),
                                        ),
                                    ),
                            )
                        })
                        .ok()
                        .flatten();
                    dialog
                        .title("编辑回答")
                        .width(px(640.))
                        .children(content)
                        .on_ok(move |_, window, cx| {
                            let _ =
                                confirm.update(cx, |this, cx| this.save_message_edit(window, cx));
                            false
                        })
                        .on_close(move |_, _, cx| {
                            let _ = close.update(cx, |this, cx| {
                                this.message_editor = None;
                                cx.notify();
                            });
                        })
                });
                input.update(cx, |input, cx| input.focus(window, cx));
            }
            MessageAction::Fork => {
                self.mutate_message(chat_id, message_id, Mutation::Fork, window, cx)
            }
            MessageAction::Delete | MessageAction::Rewind => {
                let rewind = matches!(request.action, MessageAction::Rewind);
                let owner = cx.weak_entity();
                window.open_alert_dialog(cx, move |dialog, _, _| {
                    let owner = owner.clone();
                    let message_id = message_id.clone();
                    dialog
                        .title(if rewind {
                            "回到这条提问？"
                        } else {
                            "删除这条回答？"
                        })
                        .description(if rewind {
                            "这条提问及之后的消息会被移除，提问原文将回到输入框。"
                        } else {
                            "仅删除这条助手回答，其他消息会保留。"
                        })
                        .confirm()
                        .show_cancel(true)
                        .button_props(
                            gpui_kit::component::dialog::DialogButtonProps::default()
                                .ok_text(if rewind {
                                    "回到这里"
                                } else {
                                    "删除回答"
                                })
                                .cancel_text("取消")
                                .ok_variant(gpui_kit::component::button::ButtonVariant::Danger),
                        )
                        .on_ok(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.mutate_message(
                                    chat_id,
                                    message_id.clone(),
                                    if rewind {
                                        Mutation::Rewind
                                    } else {
                                        Mutation::Delete
                                    },
                                    window,
                                    cx,
                                )
                            });
                            false
                        })
                });
            }
        }
    }

    fn save_message_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.message_editor else {
            return;
        };
        self.mutate_message(
            editor.chat_id,
            editor.message_id.clone(),
            Mutation::Edit(editor.input.read(cx).value().to_string()),
            window,
            cx,
        );
    }

    fn mutate_message(
        &mut self,
        chat_id: usize,
        message_id: String,
        mutation: Mutation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.message_action_ready(chat_id, cx) {
            return;
        }
        let (mut record, _) = self.capture_record(chat_id, cx);
        let source_key = record.id.clone();
        let fork = matches!(mutation, Mutation::Fork);
        let mut draft = None;
        let result = match mutation {
            Mutation::Edit(content) => {
                mutations::edit_assistant_answer(&mut record, &message_id, &content)
            }
            Mutation::Delete => mutations::remove_assistant_answer(&mut record, &message_id),
            Mutation::Rewind => mutations::rewind_to_user_message(&mut record, &message_id)
                .map(|content| draft = Some(content)),
            Mutation::Fork => mutations::build_fork_conversation(&record, &message_id, false)
                .map(|branch| record = branch),
        };
        if let Err(error) = result {
            self.message_action_failed(error, window, cx);
            return;
        }
        let chat = self
            .conversations
            .iter()
            .find(|chat| chat.id == chat_id)
            .unwrap();
        let persisted = chat.persisted;
        chat.view
            .update(cx, |view, cx| view.set_mutation_busy(true, cx));
        self.persistence.busy = true;
        let store = self.persistence.store.clone();
        let directory = self.catalog_dir.clone();
        let work = cx.background_spawn(async move {
            let mut warnings = Vec::new();
            let record = if fork {
                let destination = directory.join(format!("{}_attachments", record.id));
                warnings = resources::copy_forked_conversation_files_in_dirs(
                    &directory.join(format!("{source_key}_attachments")),
                    &destination,
                    &mut record.messages,
                );
                match store.create(record) {
                    Ok(record) => record,
                    Err(error) => {
                        let _ = std::fs::remove_dir_all(destination);
                        return Err(error.to_string());
                    }
                }
            } else {
                let revision = record.revision;
                let record = if persisted {
                    store.save(record, revision)
                } else {
                    store.create(record)
                }
                .map_err(|error| error.to_string())?;
                resources::sweep_conversation_attachments_in_dir(&directory, &record.id);
                record
            };
            Ok::<_, String>((record, warnings))
        });
        self.persistence.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.persistence.busy = false;
                if let Some(chat) = this.conversations.iter().find(|chat| chat.id == chat_id) {
                    chat.view
                        .update(cx, |view, cx| view.set_mutation_busy(false, cx));
                }
                match result {
                    Ok((record, warnings)) => {
                        if fork {
                            this.adopt_branch(record, window, cx);
                        } else if let Some(chat) = this
                            .conversations
                            .iter_mut()
                            .find(|chat| chat.id == chat_id)
                        {
                            let current_draft =
                                draft.unwrap_or_else(|| chat.view.read(cx).snapshot(cx).1);
                            let messages = record.messages.clone();
                            chat.record = record;
                            chat.persisted = true;
                            chat.view.update(cx, |view, cx| {
                                view.restore(messages, current_draft, window, cx)
                            });
                            // Draft changes (including rewind) join the same save barrier.
                            chat.dirty = true;
                        }
                        this.message_editor = None;
                        window.close_dialog(cx);
                        for warning in warnings {
                            window.push_notification(Notification::warning(warning), cx);
                        }
                        this.save_next(window, cx);
                    }
                    Err(error) => {
                        this.persistence.quitting = false;
                        this.message_action_failed(error, window, cx);
                        this.save_next(window, cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn message_action_failed(
        &mut self,
        error: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = &mut self.message_editor {
            editor.error = Some(error.clone());
        }
        window.push_notification(Notification::error(error), cx);
        cx.notify();
    }

    fn adopt_branch(&mut self, record: Record, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat(window, cx);
        let chat = &mut self.conversations[0];
        chat.title = record.title.clone().into();
        chat.title_is_user_set = true;
        chat.runtime = if record.agent_runtime.is_chat() {
            Runtime::Chat
        } else {
            Runtime::Agent
        };
        chat.locked = !record.messages.is_empty();
        chat.archived = false;
        chat.view.update(cx, |view, cx| {
            view.configure(
                self.providers.clone(),
                Some((record.provider_id.clone(), record.model.clone())),
                record.id.clone(),
                self.catalog_dir.clone(),
            );
            view.set_sample_mode(record.provider_id == "native-preview", cx);
            view.set_fork_origin(record.forked_from.as_ref());
            view.restore(record.messages.clone(), String::new(), window, cx);
        });
        chat.record = record;
        chat.persisted = true;
        self.sync_group_memberships();
        cx.notify();
    }
}
