use super::*;

// Action + conversation sections follow Zeron shell/command_palette.rs;
// Kivio owns navigation and history search, Kit owns modal focus/dismissal.
#[derive(Clone, Copy)]
enum PaletteCommand {
    New,
    Library,
    Providers,
    Appearance,
    Shortcuts,
    Sidebar,
}
impl PaletteCommand {
    fn label(self) -> &'static str {
        match self {
            Self::New => "新建对话",
            Self::Library => "打开对话库",
            Self::Providers => "配置供应商与模型",
            Self::Appearance => "外观设置",
            Self::Shortcuts => "查看快捷键",
            Self::Sidebar => "切换侧栏",
        }
    }
}

pub(super) struct HistoryMatch {
    pub snippet: String,
    pub message_id: Option<String>,
}

impl Shell {
    pub(super) fn match_conversation(
        chat: &Conversation,
        query: &str,
        cx: &App,
    ) -> Option<HistoryMatch> {
        use crate::conversation_search::{first_messages_match, make_search_snippet};
        if query.is_empty() {
            return Some(HistoryMatch {
                snippet: String::new(),
                message_id: None,
            });
        }
        for text in [
            Some(chat.title.as_ref()),
            chat.record.folder.as_deref(),
            chat.record
                .assistant_snapshot
                .as_ref()
                .map(|assistant| assistant.name.as_str()),
            Some(chat.record.model.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            if text.to_lowercase().contains(query) {
                return Some(HistoryMatch {
                    snippet: make_search_snippet(text, query),
                    message_id: None,
                });
            }
        }
        // The transcript is authoritative while a save or streaming reply is pending.
        let live;
        let messages = if chat.dirty || chat.view.read(cx).is_streaming(cx) {
            live = chat.view.read(cx).snapshot(cx).0;
            &live
        } else {
            &chat.record.messages
        };
        first_messages_match(messages, query).map(|(_, id, snippet)| HistoryMatch {
            snippet,
            message_id: Some(id),
        })
    }

    fn matching_conversations(&self, cx: &App) -> Vec<(&Conversation, HistoryMatch)> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut chats: Vec<_> = self
            .conversations
            .iter()
            .filter_map(|chat| {
                Self::match_conversation(chat, &query, cx).map(|found| (chat, found))
            })
            .collect();
        chats.sort_by_key(|(chat, _)| std::cmp::Reverse(chat.activity));
        chats.truncate(50);
        chats
    }

    fn open_search_match(
        &mut self,
        id: usize,
        message: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_search(window, cx);
        self.select(id, window, cx);
        if let Some(message) = message {
            self.current()
                .view
                .update(cx, |view, cx| view.reveal_message(&message, cx));
        }
    }

    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = false;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.close_dialog(cx);
    }

    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search_dialog(true, window, cx);
    }

    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search_dialog(false, window, cx);
    }

    fn palette_commands(&self, cx: &App) -> Vec<PaletteCommand> {
        if !self.command_palette {
            return Vec::new();
        }
        let query = self.search.read(cx).value().trim().to_lowercase();
        [
            PaletteCommand::New,
            PaletteCommand::Library,
            PaletteCommand::Providers,
            PaletteCommand::Appearance,
            PaletteCommand::Shortcuts,
            PaletteCommand::Sidebar,
        ]
        .into_iter()
        .filter(|command| command.label().to_lowercase().contains(&query))
        .collect()
    }

    fn run_palette_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_search(window, cx);
        match command {
            PaletteCommand::New => self.new_chat(window, cx),
            PaletteCommand::Library => self.open_library(cx),
            PaletteCommand::Sidebar => self.toggle_sidebar(window, cx),
            _ => {
                self.open_settings(window, cx);
                self.set_settings_page(
                    Some(match command {
                        PaletteCommand::Providers => SettingsPage::Providers,
                        PaletteCommand::Appearance => SettingsPage::Appearance,
                        _ => SettingsPage::Shortcuts,
                    }),
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn open_search_dialog(&mut self, commands: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.searching {
            return;
        }
        self.command_palette = commands;
        self.searching = true;
        self.search_selection = 0;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let close = owner.clone();
            let confirm = owner.clone();
            let content = owner
                .update(cx, |this, cx| this.search_results(cx))
                .unwrap_or_else(|_| div().into_any_element());
            dialog
                .title(if commands {
                    "命令与对话"
                } else {
                    "搜索对话"
                })
                .width(px(560.))
                .child(content)
                .on_ok(move |_, window, cx| {
                    let _ = confirm.update(cx, |this, cx| this.confirm_search(window, cx));
                    false
                })
                .on_close(move |_, window, cx| {
                    let _ = close.update(cx, |this, cx| {
                        this.searching = false;
                        this.search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    });
                })
        });
        self.search.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn confirm_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self.palette_commands(cx);
        if let Some(command) = commands.get(self.search_selection) {
            self.run_palette_command(*command, window, cx);
            return;
        }
        if let Some((chat, found)) = self
            .matching_conversations(cx)
            .into_iter()
            .nth(self.search_selection.saturating_sub(commands.len()))
        {
            self.open_search_match(chat.id, found.message_id, window, cx);
        }
    }

    fn search_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let chats = self.matching_conversations(cx);
        let commands = self.palette_commands(cx);
        let offset = commands.len();
        let count = chats.len() + offset;
        div()
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "down" if count > 0 => {
                        this.search_selection = (this.search_selection + 1).min(count - 1)
                    }
                    "up" => this.search_selection = this.search_selection.saturating_sub(1),
                    _ => return,
                }
                this.search_scroll.scroll_to_item(this.search_selection);
                cx.stop_propagation();
                cx.notify();
            }))
            .v_flex()
            .gap_3()
            .child(
                Input::new(&self.search)
                    .prefix(Icon::new(IconName::Search).small())
                    .cleanable(true)
                    .aria_label("搜索对话"),
            )
            .child(
                div()
                    .id("search-results")
                    .track_scroll(&self.search_scroll)
                    .v_flex()
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .gap_1()
                    .when(count == 0, |list| {
                        list.child(
                            div()
                                .p_6()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("没有找到相关对话，试试其他关键词。"),
                        )
                    })
                    .children(commands.into_iter().enumerate().map(|(index, command)| {
                        Button::new(("palette-command", index))
                            .ghost()
                            .w_full()
                            .small()
                            .label(command.label())
                            .selected(index == self.search_selection)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_palette_command(command, window, cx)
                            }))
                    }))
                    .children(chats.into_iter().enumerate().map(|(index, (chat, found))| {
                        let id = chat.id;
                        let message = found.message_id;
                        Button::new(("search-result", id))
                            .ghost()
                            .w_full()
                            .h_auto()
                            .py_2()
                            .selected(index + offset == self.search_selection)
                            .accessibility_label(chat.title.clone())
                            .child(
                                div()
                                    .v_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_1()
                                    .child(div().text_sm().text_ellipsis().overflow_hidden().child(
                                        if chat.archived {
                                            format!("{} · 已归档", chat.title)
                                        } else {
                                            chat.title.to_string()
                                        },
                                    ))
                                    .when(!found.snippet.is_empty(), |row| {
                                        row.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .text_ellipsis()
                                                .overflow_hidden()
                                                .child(found.snippet),
                                        )
                                    }),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_search_match(id, message.clone(), window, cx)
                            }))
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("↑ ↓ 选择 · Enter 打开 · Esc 关闭"),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use gpui_kit::{AppContext, TestAppContext, WindowOptions};

    // Search must read the persisted history beyond the index preview and route
    // native Enter to the matching conversation, including reasoning-only hits.
    #[gpui_kit::test]
    fn full_history_search_opens_body_and_reasoning_matches(cx: &mut TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        let store = crate::conversation_files::ConversationStore::new(data.path().to_owned());
        let mut record =
            crate::conversation_data::Conversation::new("native-preview".into(), "sample".into());
        record.title = "历史记录".into();
        record
            .messages
            .push(crate::conversation_data::ChatMessage::text(
                "user",
                "与索引预览无关".repeat(80),
            ));
        let mut answer =
            crate::conversation_data::ChatMessage::text("assistant", "真正的正文命中蓝鲸坐标");
        answer.reasoning = Some("深层推理术语轨道交点".into());
        record.messages.push(answer);
        let record = store.create(record).unwrap();
        let (window, shell) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(data.path().to_owned(), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        for query in ["蓝鲸坐标", "轨道交点"] {
            window
                .update(cx, |_, window, cx| {
                    shell.update(cx, |shell, cx| shell.new_chat(window, cx))
                })
                .unwrap();
            cx.simulate_keystrokes(window, "secondary-f");
            cx.simulate_input(window, query);
            cx.simulate_keystrokes(window, "enter");
            cx.run_until_parked();
            shell.read_with(cx, |shell, _| {
                assert!(!shell.searching, "a full-history match must open");
                assert_eq!(shell.current().record.id, record.id);
            });
        }
    }
}
