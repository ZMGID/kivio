use super::*;

impl Shell {
    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = false;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.close_dialog(cx);
    }

    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.searching {
            return;
        }
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
                .title("搜索对话")
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
        let query = self.search.read(cx).value().to_lowercase();
        let mut chats: Vec<_> = self
            .conversations
            .iter()
            .filter(|chat| chat.title.to_lowercase().contains(&query))
            .collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
        if let Some(id) = chats.get(self.search_selection).map(|chat| chat.id) {
            self.close_search(window, cx);
            self.select(id, window, cx);
        }
    }

    fn search_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search.read(cx).value().to_lowercase();
        let mut chats: Vec<_> = self
            .conversations
            .iter()
            .filter(|chat| chat.title.to_lowercase().contains(&query))
            .collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
        let count = chats.len().min(50);
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
                    .when(chats.is_empty(), |list| {
                        list.child(
                            div()
                                .p_6()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("没有找到相关对话，试试其他关键词。"),
                        )
                    })
                    .children(chats.into_iter().take(50).enumerate().map(|(index, chat)| {
                        let id = chat.id;
                        Self::nav_button(
                            ("search-result", id),
                            if chat.archived {
                                format!("{} · 已归档", chat.title)
                            } else {
                                chat.title.to_string()
                            },
                            IconName::MessageSquare,
                        )
                        .selected(index == self.search_selection)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.close_search(window, cx);
                                this.select(id, window, cx);
                            },
                        ))
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
