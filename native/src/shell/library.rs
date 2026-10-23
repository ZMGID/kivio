//! Kivio Sidebar.tsx / conversation library, with window-local M1 data.
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

impl Shell {
    pub(super) fn open_library(&mut self, cx: &mut Context<Self>) {
        self.settings = None;
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
        let Some(editor) = self.group_editor.as_mut() else {
            return;
        };
        let name = editor.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            editor.error = Some("请填写名称。".into());
            cx.notify();
            return;
        }
        let id = editor.id.unwrap_or(self.next_group);
        let group = Group {
            id,
            name: name.into(),
            project: editor.project,
            directory: editor.directory.clone(),
        };
        self.tab = if group.project {
            SidebarTab::Projects
        } else {
            SidebarTab::Sets
        };
        if let Some(old) = self.groups.iter_mut().find(|group| group.id == id) {
            *old = group;
        } else {
            self.next_group += 1;
            self.groups.insert(0, group);
        }
        self.group_editor = None;
        self.selected_group = Some(id);
        self.collapsed_groups.remove(&id);
        window.close_dialog(cx);
        window.focus(&self.focus, cx);
        cx.notify();
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
        let query = self.search.read(cx).value().to_lowercase();
        let archived = self.library_filter == LibraryFilter::Archived;
        let mut chats: Vec<_> = self
            .conversations
            .iter()
            .filter(|chat| {
                chat.archived == archived
                    && group.is_none_or(|group| self.group_membership(chat, group.id))
                    && (query.is_empty() || chat.title.to_lowercase().contains(&query))
            })
            .collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
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
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.move_group(id, -1, cx)),
                            ),
                    )
                    .child(
                        Button::new("group-down")
                            .small()
                            .ghost()
                            .label("下移")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.move_group(id, 1, cx)),
                            ),
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
                        Self::nav_button(
                            ("library-chat", id),
                            chat.title.clone(),
                            IconName::MessageSquare,
                        )
                        .flex_1()
                        .min_w_0()
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.select(id, window, cx)),
                        ),
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
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(chat) =
                                    this.conversations.iter_mut().find(|chat| chat.id == id)
                                {
                                    chat.archived = !chat.archived;
                                }
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
        cx: &mut Context<Self>,
    ) {
        if before == Some(drag.id) {
            return;
        }
        if before.is_some_and(|id| {
            !self
                .groups
                .iter()
                .any(|g| g.id == id && g.project == drag.project)
        }) {
            return;
        }
        let Some(from) = self
            .groups
            .iter()
            .position(|g| g.id == drag.id && g.project == drag.project)
        else {
            return;
        };
        let group = self.groups.remove(from);
        let to = before
            .and_then(|id| self.groups.iter().position(|g| g.id == id))
            .unwrap_or_else(|| {
                self.groups
                    .iter()
                    .rposition(|g| g.project == drag.project)
                    .map_or(self.groups.len(), |ix| ix + 1)
            });
        self.groups.insert(to, group);
        cx.notify();
    }

    pub(super) fn move_group(&mut self, id: usize, direction: isize, cx: &mut Context<Self>) {
        let Some(from) = self.groups.iter().position(|group| group.id == id) else {
            return;
        };
        let project = self.groups[from].project;
        let target = if direction < 0 {
            (0..from)
                .rev()
                .find(|ix| self.groups[*ix].project == project)
        } else {
            (from + 1..self.groups.len()).find(|ix| self.groups[*ix].project == project)
        };
        if let Some(to) = target {
            self.groups.swap(from, to);
            cx.notify();
        }
    }
}
