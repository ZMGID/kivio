//! Kivio Sidebar.tsx / conversation library, using the shared Rust group catalog.
//! Group order is explicit (ADR 0004); activity never reorders groups.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarTab {
    Recent,
    Sets,
    Projects,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LibraryFilter {
    Active,
    Archived,
}

pub(super) struct Group {
    pub id: usize,
    pub key: String,
    pub name: SharedString,
    pub project: bool,
    pub directory: Option<std::path::PathBuf>,
}
pub(super) struct GroupEditor {
    pub id: Option<usize>,
    pub project: bool,
    pub name: Entity<InputState>,
    pub directory: Option<std::path::PathBuf>,
    pub error: Option<SharedString>,
    pub _subscription: Subscription,
}

#[derive(Clone)]
pub(super) struct DragGroup {
    pub id: usize,
    pub project: bool,
    pub name: SharedString,
}
impl Render for DragGroup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_lg()
            .shadow_md()
            .bg(cx.theme().popover)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(self.name.clone())
    }
}

#[derive(Clone)]
pub(super) struct DragConversation {
    pub id: usize,
    pub group: usize,
    pub title: SharedString,
}
impl Render for DragConversation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_lg()
            .shadow_md()
            .bg(cx.theme().popover)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(self.title.clone())
    }
}

impl Shell {
    pub(super) fn assign_conversation_group(
        &mut self,
        id: usize,
        group: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.catalog_busy || !self.catalog_ready || !self.persistence.ready {
            return;
        }
        let target = group.and_then(|id| self.groups.iter().find(|group| group.id == id));
        if group.is_some() && target.is_none() {
            return;
        }
        let Some(chat) = self.conversations.iter_mut().find(|chat| chat.id == id) else {
            return;
        };
        let previous = chat.project.or(chat.set);
        chat.project = target.filter(|group| group.project).map(|group| group.id);
        chat.set = target.filter(|group| !group.project).map(|group| group.id);
        if previous == chat.project.or(chat.set) {
            return;
        }
        chat.membership_dirty = true;
        let key = chat.record.id.clone();
        // The record is authoritative. Commit membership before its optional pin cleanup.
        self.mark_dirty(id, window, cx);
        if let Some(group) = previous.and_then(|id| self.groups.iter().find(|group| group.id == id))
        {
            let mut pins = self
                .persistence
                .pins
                .get(&group.key)
                .cloned()
                .unwrap_or_default();
            pins.retain(|pin| pin.id != key);
            self.queue_pins(group.key.clone(), pins, window, cx);
        }
        cx.notify();
    }

    pub(super) fn ordered_group_conversations(
        &self,
        group: usize,
        archived: bool,
    ) -> Vec<&Conversation> {
        let mut chats: Vec<_> = self
            .conversations
            .iter()
            .filter(|chat| chat.archived == archived && self.group_membership(chat, group))
            .collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
        let pins = self
            .groups
            .iter()
            .find(|g| g.id == group)
            .and_then(|g| self.persistence.pins.get(&g.key));
        crate::group_catalog::apply_conversation_pins(
            chats,
            pins.map(Vec::as_slice).unwrap_or_default(),
            |chat| &chat.record.id,
        )
    }

    pub(super) fn pin_conversation(
        &mut self,
        id: usize,
        group: usize,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.catalog_busy {
            return;
        }
        let Some(chat) = self
            .conversations
            .iter()
            .find(|chat| chat.id == id && self.group_membership(chat, group))
        else {
            return;
        };
        let Some(group) = self.groups.iter().find(|g| g.id == group) else {
            return;
        };
        let mut pins = self
            .persistence
            .pins
            .get(&group.key)
            .cloned()
            .unwrap_or_default();
        pins.retain(|pin| pin.id != chat.record.id);
        if let Some(row) = row {
            pins.push(crate::conversation_data::ConversationPin {
                id: chat.record.id.clone(),
                row: row as u32,
            });
        }
        self.queue_pins(group.key.clone(), pins, window, cx);
    }

    pub(super) fn move_group_conversation(
        &mut self,
        id: usize,
        group: usize,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chats = self.ordered_group_conversations(group, false);
        let Some(row) = chats.iter().position(|chat| chat.id == id) else {
            return;
        };
        let next = row
            .saturating_add_signed(delta)
            .min(chats.len().saturating_sub(1));
        self.pin_conversation(id, group, Some(next), window, cx);
    }

    pub(super) fn drop_conversation(
        &mut self,
        drag: &DragConversation,
        before: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if drag.id == before {
            return;
        }
        let chats = self.ordered_group_conversations(drag.group, false);
        let Some(row) = chats.iter().position(|chat| chat.id == before) else {
            return;
        };
        self.pin_conversation(drag.id, drag.group, Some(row), window, cx);
    }

    pub(super) fn delete_group(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.catalog_busy {
            return;
        }
        let Some(group) = self.groups.iter().find(|group| group.id == id) else {
            return;
        };
        let title = format!("删除“{}”？", group.name);
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            dialog
                .title(title.clone())
                .confirm()
                .description("对话会保留在最近和对话库中，项目目录及其中的文件不会删除。")
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("删除分组")
                        .cancel_text("取消")
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| this.queue_group_deletion(id, window, cx));
                    true
                })
        });
    }

    pub(super) fn open_library(&mut self, cx: &mut Context<Self>) {
        self.set_settings_page(None, cx);
        self.path_task = None;
        self.library = true;
        self.group_editor = None;
        self.selected_group = None;
        cx.notify();
    }

    pub(super) fn edit_group(
        &mut self,
        id: Option<usize>,
        project: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.catalog_busy {
            return;
        }
        if !self.catalog_ready {
            self.catalog_operation(|_| Ok(None), false, window, cx);
            return;
        }
        self.path_task = None;
        self.rename = None;
        let group = id.and_then(|id| self.groups.iter().find(|group| group.id == id));
        let name = group.map(|group| group.name.clone()).unwrap_or_default();
        let directory = group.and_then(|group| group.directory.clone());
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if project { "项目名称" } else { "集名称" })
                .default_value(name)
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_group(window, cx);
            }
        });
        self.group_editor = Some(GroupEditor {
            id,
            project,
            name: input,
            directory,
            error: None,
            _subscription: subscription,
        });
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let close = owner.clone();
            let confirm = owner.clone();
            let content = owner
                .update(cx, |this, cx| {
                    this.group_editor
                        .as_ref()
                        .map(|_| this.group_editor_view(cx))
                        .unwrap_or_else(|| div().into_any_element())
                })
                .unwrap_or_else(|_| div().into_any_element());
            dialog
                .width(px(440.))
                .child(content)
                .on_ok(move |_, window, cx| {
                    let _ = confirm.update(cx, |this, cx| this.save_group(window, cx));
                    false
                })
                .on_close(move |_, _, cx| {
                    let _ = close.update(cx, |this, cx| {
                        this.group_editor = None;
                        this.path_task = None;
                        cx.notify();
                    });
                })
        });
        if let Some(editor) = &self.group_editor {
            editor.name.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    pub(super) fn save_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.catalog_busy {
            return;
        }
        let Some(editor) = self.group_editor.as_ref() else {
            return;
        };
        let name = editor.name.read(cx).value().to_string();
        let project = editor.project;
        let root = editor
            .directory
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        let existing = editor
            .id
            .and_then(|id| self.groups.iter().find(|g| g.id == id))
            .map(|g| g.key.clone());
        self.tab = if project {
            SidebarTab::Projects
        } else {
            SidebarTab::Sets
        };
        self.catalog_operation(
            move |dir| {
                use crate::group_catalog::{self as catalog, ChatProject, ChatSet};
                let now = chrono::Local::now().timestamp();
                let key = existing.clone().unwrap_or_else(|| {
                    format!(
                        "{}_{}",
                        if project { "proj" } else { "set" },
                        uuid::Uuid::new_v4()
                    )
                });
                if project {
                    if existing.is_some() {
                        catalog::update_project(
                            dir,
                            &key,
                            Some(name),
                            None,
                            false,
                            None,
                            false,
                            root,
                            true,
                        )?;
                    } else {
                        catalog::create_project_with_options(
                            dir,
                            ChatProject {
                                id: key.clone(),
                                name,
                                description: None,
                                color: None,
                                root_path: root,
                                created_at: now,
                                updated_at: now,
                            },
                            false,
                        )?;
                    }
                } else if existing.is_some() {
                    catalog::update_set(
                        dir,
                        &key,
                        Some(name),
                        None,
                        false,
                        None,
                        false,
                        None,
                        false,
                    )?;
                } else {
                    catalog::create_set(
                        dir,
                        ChatSet {
                            id: key.clone(),
                            name,
                            system_prompt: String::new(),
                            default_assistant_id: None,
                            color: None,
                            created_at: now,
                            updated_at: now,
                        },
                    )?;
                }
                Ok(Some(key))
            },
            true,
            window,
            cx,
        );
    }

    pub(super) fn choose_directory(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择项目工作目录".into()),
        });
        // A picker result belongs to this editor only; replacing the editor cancels its task.
        self.path_task = Some(cx.spawn(async move |this, cx| {
            let result = rx.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(editor) = this.group_editor.as_mut() {
                    match result {
                        Ok(Ok(Some(paths))) => {
                            editor.directory = paths.into_iter().next();
                            editor.error = None;
                        }
                        Ok(Ok(None)) => {}
                        _ => editor.error = Some("无法打开目录选择器，请重试。".into()),
                    }
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn group_membership(&self, chat: &Conversation, id: usize) -> bool {
        chat.project == Some(id) || chat.set == Some(id)
    }

    pub(super) fn group_editor_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = self.group_editor.as_ref().expect("editor route");
        let project = editor.project;
        let title = format!(
            "{}{}",
            if editor.id.is_some() {
                "编辑"
            } else {
                "新建"
            },
            if project { "项目" } else { "集" }
        );
        let mut content = div()
            .v_flex()
            .gap_4()
            .w_full()
            .max_w(rems(34.))
            .child(div().text_lg().font_weight(FontWeight::MEDIUM).child(title))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if project {
                        "把对话放在同一个工作目录下。"
                    } else {
                        "把相关对话收在一起，不改变它们的工作目录。"
                    }),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(div().text_sm().child(if project {
                        "项目名称"
                    } else {
                        "集名称"
                    }))
                    .child(Input::new(&editor.name).aria_label(if project {
                        "项目名称"
                    } else {
                        "集名称"
                    })),
            );
        if project {
            content = content.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(div().text_sm().child("工作目录"))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                editor
                                    .directory
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or("未指定 · 使用默认工作目录".into()),
                            ),
                    )
                    .child(
                        Button::new("choose-directory")
                            .disabled(self.catalog_busy)
                            .label("选择文件夹")
                            .icon(IconName::Folder)
                            .on_click(cx.listener(|this, _, _, cx| this.choose_directory(cx))),
                    ),
            );
        }
        if let Some(error) = &editor.error {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        content = content.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .pt_3()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    Button::new("cancel-group")
                        .ghost()
                        .label("取消")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.group_editor = None;
                            this.path_task = None;
                            window.close_dialog(cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("save-group")
                        .disabled(self.catalog_busy)
                        .loading(self.catalog_busy)
                        .primary()
                        .label(if editor.id.is_some() {
                            "保存"
                        } else {
                            "创建"
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.save_group(window, cx))),
                ),
        );
        content.into_any_element()
    }

    pub(super) fn render_library(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
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
        let group = self
            .selected_group
            .and_then(|id| self.groups.iter().find(|group| group.id == id));
        let query = self.search.read(cx).value().trim().to_lowercase();
        let archived = self.library_filter == LibraryFilter::Archived;
        let mut chats: Vec<_> = self
            .conversations
            .iter()
            .filter(|chat| {
                chat.archived == archived
                    && group.is_none_or(|group| self.group_membership(chat, group.id))
                    && Self::match_conversation(chat, &query, cx).is_some()
            })
            .collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
        if let Some(group) = group {
            // Pins refer to the complete group, so searching only filters the result.
            chats = self
                .ordered_group_conversations(group.id, archived)
                .into_iter()
                .filter(|chat| Self::match_conversation(chat, &query, cx).is_some())
                .collect();
        }
        let mut content = div().v_flex().w_full().max_w(rems(52.)).gap_4().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(rems(1.43))
                        .font_weight(FontWeight::MEDIUM)
                        .child(
                            group
                                .map(|group| group.name.clone())
                                .unwrap_or("对话库".into()),
                        ),
                )
                .child(
                    Button::new("library-new")
                        .small()
                        .label("新建对话")
                        .icon(IconName::Plus)
                        .disabled(self.catalog_busy)
                        .on_click(cx.listener(|this, _, window, cx| this.new_chat(window, cx))),
                ),
        );
        if let Some(group) = group {
            let id = group.id;
            let project = group.project;
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("edit-group")
                            .small()
                            .ghost()
                            .label("编辑")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_group(Some(id), project, window, cx)
                            })),
                    )
                    .child(
                        Button::new("group-up")
                            .small()
                            .ghost()
                            .label("上移")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.move_group(id, -1, window, cx)
                            })),
                    )
                    .child(
                        Button::new("group-down")
                            .small()
                            .ghost()
                            .label("下移")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.move_group(id, 1, window, cx)
                            })),
                    )
                    .child(
                        Button::new("delete-group")
                            .small()
                            .ghost()
                            .label("删除分组")
                            .disabled(self.catalog_busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_group(id, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                group
                                    .directory
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default(),
                            ),
                    ),
            );
        }
        content = content
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(compact, |row| row.flex_col().items_start())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(compact, |field| field.w_full())
                            .child(
                                Input::new(&self.search)
                                    .small()
                                    .prefix(Icon::new(IconName::Search).small())
                                    .cleanable(true)
                                    .aria_label("搜索对话库"),
                            ),
                    )
                    .child(
                        TabBar::new("library-filter")
                            .segmented()
                            .small()
                            .selected_index(usize::from(archived))
                            .children([Tab::new().label("对话"), Tab::new().label("已归档")])
                            .on_click(cx.listener(|this, index, _, cx| {
                                this.library_filter =
                                    [LibraryFilter::Active, LibraryFilter::Archived][*index];
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} 个对话", chats.len())),
            );
        if chats.is_empty() {
            content = content.child(
                div().v_flex().gap_2().py_8().child("这里还没有对话").child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if !query.is_empty() {
                            "试试其他关键词，或清空搜索。"
                        } else if archived {
                            "归档的对话会保留在这里，可以随时恢复。"
                        } else {
                            "新建一个对话，从这里开始。"
                        }),
                ),
            );
        }
        for chat in chats {
            let id = chat.id;
            content = content.child(
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
                            .v_flex()
                            .child(self.chat_row(chat, group.map(|g| g.id), cx))
                            .when(!query.is_empty(), |row| {
                                row.child(
                                    div()
                                        .px_3()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            Self::match_conversation(chat, &query, cx)
                                                .map(|found| found.snippet)
                                                .unwrap_or_default(),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(chat.runtime.label()),
                    )
                    .child(
                        Button::new(("archive", id))
                            .small()
                            .ghost()
                            .label(if archived { "恢复" } else { "归档" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(chat) =
                                    this.conversations.iter_mut().find(|chat| chat.id == id)
                                {
                                    chat.archived = !chat.archived;
                                }
                                this.mark_dirty(id, window, cx);
                                cx.notify();
                            })),
                    ),
            );
        }
        div()
            .id("conversation-library")
            .size_full()
            .overflow_y_scroll()
            .when(compact, |pane| pane.p_5())
            .when(!compact, |pane| pane.p_8())
            .flex()
            .justify_center()
            .child(content)
            .into_any_element()
    }

    pub(super) fn drop_group(
        &mut self,
        drag: &DragGroup,
        before: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.catalog_busy || before == Some(drag.id) {
            return;
        }
        let mut rows: Vec<_> = self
            .groups
            .iter()
            .filter(|g| g.project == drag.project)
            .map(|g| (g.id, g.key.clone()))
            .collect();
        if before.is_some_and(|id| !rows.iter().any(|g| g.0 == id)) {
            return;
        }
        let Some(from) = rows.iter().position(|g| g.0 == drag.id) else {
            return;
        };
        let row = rows.remove(from);
        let to = before
            .and_then(|id| rows.iter().position(|g| g.0 == id))
            .unwrap_or(rows.len());
        rows.insert(to, row);
        self.persist_group_order(
            drag.project,
            rows.into_iter().map(|g| g.1).collect(),
            window,
            cx,
        );
    }

    pub(super) fn move_group(
        &mut self,
        id: usize,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.catalog_busy {
            return;
        }
        let Some(group) = self.groups.iter().find(|g| g.id == id) else {
            return;
        };
        let project = group.project;
        let mut rows: Vec<_> = self
            .groups
            .iter()
            .filter(|g| g.project == project)
            .map(|g| (g.id, g.key.clone()))
            .collect();
        let Some(from) = rows.iter().position(|g| g.0 == id) else {
            return;
        };
        let to = from as isize + direction;
        if to < 0 || to >= rows.len() as isize {
            return;
        }
        rows.swap(from, to as usize);
        self.persist_group_order(project, rows.into_iter().map(|g| g.1).collect(), window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use gpui_kit::{AppContext, TestAppContext, WindowOptions, test::TestWindowExt};

    // A group is a catalog membership, never ownership of a user's project directory.
    // Persisted pins survive reopening and disappear when their conversation leaves.
    #[gpui_kit::test]
    fn group_pins_membership_and_deletion_survive_reopening(cx: &mut TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        let project_root = tempfile::tempdir().unwrap();
        std::fs::write(project_root.path().join("keep.txt"), "project work").unwrap();
        crate::group_catalog::create_project_with_options(
            data.path(),
            crate::group_catalog::ChatProject {
                id: "proj_test".into(),
                name: "真实项目".into(),
                description: None,
                color: None,
                root_path: Some(project_root.path().display().to_string()),
                created_at: 1,
                updated_at: 1,
            },
            false,
        )
        .unwrap();
        let store = crate::conversation_files::ConversationStore::new(data.path().to_owned());
        let mut records = Vec::new();
        for index in 0..3 {
            let mut record = crate::conversation_data::Conversation::new(
                "native-preview".into(),
                "sample".into(),
            );
            record.title = format!("对话 {index}");
            record.updated_at = index;
            record.project_id = Some("proj_test".into());
            record.folder = Some("真实项目".into());
            record
                .messages
                .push(crate::conversation_data::ChatMessage::text(
                    "user",
                    "保留正文",
                ));
            records.push(store.create(record).unwrap());
        }
        let (window, shell) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(data.path().to_owned(), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let (group, oldest) = shell.read_with(cx, |shell, _| {
            (
                shell.groups[0].id,
                shell
                    .conversations
                    .iter()
                    .find(|chat| chat.record.id == records[0].id)
                    .unwrap()
                    .id,
            )
        });
        let middle = shell.read_with(cx, |shell, _| {
            shell
                .conversations
                .iter()
                .find(|chat| chat.record.id == records[1].id)
                .unwrap()
                .id
        });
        // Recent remains a timeline: native dragging there cannot create a pin.
        window
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.drag_to(("chat", oldest), ("chat", middle), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(
            crate::group_catalog::load_conversation_pins(data.path())
                .unwrap()
                .is_empty()
        );
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.tab = super::SidebarTab::Projects;
                    cx.notify();
                });
                window.render_frame(cx);
                window.drag_to(("chat", oldest), ("chat", middle), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let (_second_window, reopened) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(data.path().to_owned(), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        reopened.read_with(cx, |shell, _| {
            assert_eq!(
                shell.ordered_group_conversations(shell.groups[0].id, false)[1]
                    .record
                    .id,
                records[0].id
            )
        });
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.assign_conversation_group(oldest, None, window, cx)
                })
            })
            .unwrap();
        cx.run_until_parked();
        shell.read_with(cx, |shell, _| {
            assert!(
                shell.persistence.error.is_none(),
                "{:?}",
                shell.persistence.error
            );
            assert!(
                shell
                    .conversations
                    .iter()
                    .find(|chat| chat.id == oldest)
                    .unwrap()
                    .project
                    .is_none()
            );
        });
        assert!(
            !crate::group_catalog::load_conversation_pins(data.path())
                .unwrap()
                .contains_key("proj_test")
        );
        assert!(store.load(&records[0].id).unwrap().project_id.is_none());
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.delete_group(group, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "escape");
        cx.run_until_parked();
        assert_eq!(
            crate::group_catalog::load_project_index(data.path())
                .unwrap()
                .projects
                .len(),
            1,
            "cancel retains membership"
        );
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.delete_group(group, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        assert!(
            crate::group_catalog::load_project_index(data.path())
                .unwrap()
                .projects
                .is_empty()
        );
        let saved = store.load_all().unwrap();
        assert_eq!(saved.len(), 3);
        for record in saved {
            assert!(record.project_id.is_none());
            assert!(record.folder.is_none());
            assert_eq!(record.messages[0].content, "保留正文");
        }
        assert_eq!(
            std::fs::read_to_string(project_root.path().join("keep.txt")).unwrap(),
            "project work"
        );
    }
}
