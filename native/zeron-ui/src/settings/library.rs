//! Kivio's conversation library, using its shared query and durable store.
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    conversation_data::{ConversationListItem, ConversationSearchHit},
    conversation_files::ConversationStore,
    conversation_library::{self, ConversationLibraryQuery},
    conversation_search,
};

pub struct OpenConversation(pub String);
pub struct LibraryPage {
    directory: std::path::PathBuf,
    state: Entity<crate::state::AppState>,
    operation: Option<Task<()>>,
    search: Entity<InputState>,
    shelf: &'static str,
    sort: &'static str,
    full_text: bool,
    page: usize,
    total: usize,
    rows: Vec<ConversationSearchHit>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    _search: Subscription,
}
impl EventEmitter<OpenConversation> for LibraryPage {}
impl LibraryPage {
    pub fn new(
        directory: std::path::PathBuf,
        state: Entity<crate::state::AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索标题、预览和正文"));
        let subscription = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.page = 0;
                this.refresh(cx);
            }
        });
        let mut this = Self {
            directory,
            state,
            operation: None,
            search,
            shelf: "all",
            sort: "updated",
            full_text: true,
            page: 0,
            total: 0,
            rows: vec![],
            busy: false,
            error: None,
            task: None,
            _search: subscription,
        };
        this.refresh(cx);
        this
    }
    fn archive(&mut self, id: String, archived: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("对话服务未连接。".into());
            cx.notify();
            return;
        };
        self.busy = true;
        self.operation = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(
                    "Mutate",
                    serde_json::json!({"op":"setChatArchived", "chatId":id, "archived":archived}),
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(_) => this.refresh(cx),
                    Err(error) => {
                        this.error = Some(error.to_string());
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let directory = self.directory.clone();
        let query = ConversationLibraryQuery {
            offset: self.page * 40,
            limit: 40,
            shelf: self.shelf.into(),
            sort: self.sort.into(),
            order: "desc".into(),
            q: Some(self.search.read(cx).value().to_string()),
            full_text: self.full_text,
            ..Default::default()
        };
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let conversations = ConversationStore::new(directory)
                        .load_all()
                        .map_err(|error| error.to_string())?;
                    let items = conversations
                        .iter()
                        .map(ConversationListItem::from)
                        .collect();
                    let by_id: std::collections::HashMap<_, _> =
                        conversations.iter().map(|c| (c.id.as_str(), c)).collect();
                    conversation_library::query_items(
                        items,
                        query,
                        |id, needle| {
                            by_id.get(id).is_some_and(|c| {
                                conversation_search::first_message_match(c, needle).is_some()
                            })
                        },
                        |item, needle| {
                            let (field, id, snippet) = conversation_search::first_message_match(
                                by_id.get(item.id.as_str())?,
                                needle,
                            )?;
                            Some(ConversationSearchHit {
                                item,
                                match_field: field,
                                match_message_id: Some(id),
                                match_snippet: Some(snippet),
                            })
                        },
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(page) => {
                        this.rows = page.items;
                        this.total = page.total;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for LibraryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut shelves = div().v_flex().gap_2().child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .mb_2()
                .child("书架"),
        );
        for (id, label) in [
            ("all", "全部"),
            ("starred", "收藏"),
            ("uncategorized", "未归类"),
            ("recent7d", "近 7 天"),
            ("archived", "归档"),
        ] {
            shelves = shelves.child(
                Button::new(id)
                    .ghost()
                    .w_full()
                    .justify_start()
                    .label(label)
                    .selected(self.shelf == id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.shelf = id;
                        this.page = 0;
                        this.refresh(cx);
                    })),
            );
        }
        let mut rows = div().v_flex();
        for hit in &self.rows {
            let item = &hit.item;
            let id = item.id.clone();
            let archive_id = id.clone();
            let archived = item.archived;
            let date = chrono::DateTime::from_timestamp(item.updated_at, 0)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                Button::new(SharedString::from(format!("library-open-{id}")))
                                    .ghost()
                                    .small()
                                    .w_full()
                                    .justify_start()
                                    .label(item.title.clone())
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(OpenConversation(id.clone()))
                                    })),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(
                                        hit.match_snippet
                                            .as_deref()
                                            .unwrap_or(&item.preview)
                                            .split_whitespace()
                                            .collect::<Vec<_>>()
                                            .join(" "),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w(px(128.))
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(date),
                    )
                    .child(
                        Button::new(SharedString::from(format!("library-archive-{archive_id}")))
                            .label(if archived { "恢复对话" } else { "归档" })
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.archive(archive_id.clone(), !archived, cx)
                            })),
                    ),
            );
        }
        if self.rows.is_empty() && !self.busy {
            rows = rows.child(
                div()
                    .py_12()
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child("没有匹配的对话。换个书架或清空搜索条件试试。"),
            );
        }
        let owner = cx.weak_entity();
        let sort = self.sort;
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .px_6()
            .pt(px(48.))
            .pb_6()
            .gap_4()
            .text_size(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("对话库"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(format!("{} 个对话", self.total))
                            .child(
                                Button::new("library-refresh")
                                    .label(if self.busy { "读取中…" } else { "刷新" })
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                            ),
                    ),
            )
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_color(cx.theme().danger).child(error))
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_lg()
                    .child(
                        div()
                            .w(px(152.))
                            .flex_none()
                            .p_3()
                            .border_r_1()
                            .border_color(cx.theme().border)
                            .child(shelves),
                    )
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .p_3()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(Input::new(&self.search).aria_label("搜索对话")),
                                    )
                                    .child(
                                        Button::new("library-sort")
                                            .label(match self.sort {
                                                "created" => "创建时间",
                                                "title" => "标题",
                                                "messages" => "消息数",
                                                _ => "最近更新",
                                            })
                                            .dropdown_caret(true)
                                            .dropdown_menu(move |mut menu, _, _| {
                                                for (id, label) in [
                                                    ("updated", "最近更新"),
                                                    ("created", "创建时间"),
                                                    ("title", "标题"),
                                                    ("messages", "消息数"),
                                                ] {
                                                    let owner = owner.clone();
                                                    menu = menu.item(
                                                        PopupMenuItem::new(label)
                                                            .checked(sort == id)
                                                            .on_click(move |_, _, cx| {
                                                                let _ =
                                                                    owner.update(cx, |this, cx| {
                                                                        this.sort = id;
                                                                        this.page = 0;
                                                                        this.refresh(cx);
                                                                    });
                                                            }),
                                                    );
                                                }
                                                menu
                                            }),
                                    )
                                    .child(
                                        Switch::new("library-full-text")
                                            .label("搜正文")
                                            .checked(self.full_text)
                                            .on_click(cx.listener(|this, checked, _, cx| {
                                                this.full_text = *checked;
                                                this.page = 0;
                                                this.refresh(cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("标题 / 内容预览")
                                    .child("更新时间"),
                            )
                            .child(
                                div()
                                    .id("library-rows")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(rows),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        Button::new("library-previous")
                                            .label("上一页")
                                            .disabled(self.busy || self.page == 0)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.page = this.page.saturating_sub(1);
                                                this.refresh(cx);
                                            })),
                                    )
                                    .child(format!(
                                        "{} / {}",
                                        self.page + 1,
                                        self.total.div_ceil(40).max(1)
                                    ))
                                    .child(
                                        Button::new("library-next")
                                            .label("下一页")
                                            .disabled(
                                                self.busy || (self.page + 1) * 40 >= self.total,
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.page += 1;
                                                this.refresh(cx);
                                            })),
                                    ),
                            ),
                    ),
            )
    }
}
