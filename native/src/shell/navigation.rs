use super::library::DragGroup;
use super::*;

impl Shell {
    pub(super) fn nav_button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        icon: IconName,
    ) -> Button {
        let label = label.into();
        Button::new(id)
            .ghost()
            .small()
            .h(rems(2.28))
            .w_full()
            .accessibility_label(label.clone())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(Icon::new(icon).small().flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(label),
                    ),
            )
    }

    pub(super) fn render_sidebar(&self, _: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div().v_flex().gap_1();
        if let Some(page) = self.settings {
            body = body.children(
                [
                    (SettingsPage::Appearance, "通用", IconName::Sun),
                    (SettingsPage::Shortcuts, "快捷键", IconName::Keyboard),
                    (SettingsPage::About, "关于", IconName::Info),
                ]
                .into_iter()
                .map(|(value, label, icon)| {
                    Self::nav_button(label, label, icon)
                        .selected(page == value)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings = Some(value);
                            cx.notify();
                        }))
                }),
            );
        } else {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .py_2()
                    .child(
                        TabBar::new("sidebar-sections")
                            .segmented()
                            .small()
                            .selected_index(match self.tab {
                                SidebarTab::Recent => 0,
                                SidebarTab::Sets => 1,
                                SidebarTab::Projects => 2,
                            })
                            .children([
                                Tab::new().label("最近"),
                                Tab::new().label("集"),
                                Tab::new().label("项目"),
                            ])
                            .on_click(cx.listener(|this, index, _, cx| {
                                this.tab =
                                    [SidebarTab::Recent, SidebarTab::Sets, SidebarTab::Projects]
                                        [*index];
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(if self.tab == SidebarTab::Recent {
                        Button::new("all-chats")
                            .ghost()
                            .small()
                            .icon(IconName::Archive)
                            .tooltip("所有对话与归档")
                            .text_color(cx.theme().muted_foreground)
                            .accessibility_label("打开对话库")
                            .on_click(cx.listener(|this, _, _, cx| this.open_library(cx)))
                    } else {
                        let project = self.tab == SidebarTab::Projects;
                        Button::new("add-group")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(if project { "新建项目" } else { "新建集" })
                            .accessibility_label(if project { "新建项目" } else { "新建集" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_group(None, project, window, cx)
                            }))
                    }),
            );
            if self.tab == SidebarTab::Recent {
                let mut chats: Vec<_> = self
                    .conversations
                    .iter()
                    .filter(|chat| !chat.archived)
                    .collect();
                chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
                for chat in chats.into_iter().take(80) {
                    body = body.child(self.chat_row(chat, cx));
                }
            } else {
                let project = self.tab == SidebarTab::Projects;
                let groups: Vec<_> = self
                    .groups
                    .iter()
                    .filter(|group| group.project == project)
                    .collect();
                if groups.is_empty() {
                    body = body.child(
                        div()
                            .p_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if project {
                                "用项目整理工作目录和对话"
                            } else {
                                "用集整理相关对话"
                            }),
                    );
                }
                for group in groups {
                    let id = group.id;
                    let collapsed = self.collapsed_groups.contains(&id);
                    let count = self
                        .conversations
                        .iter()
                        .filter(|chat| !chat.archived && self.group_membership(chat, id))
                        .count();
                    let owner = cx.entity().downgrade();
                    let menu = move |menu: PopupMenu,
                                     _: &mut Window,
                                     _: &mut Context<PopupMenu>| {
                        let edit = owner.clone();
                        let open = owner.clone();
                        let new = owner.clone();
                        let up = owner.clone();
                        let down = owner.clone();
                        menu.item(
                            PopupMenuItem::new("新建对话").on_click(move |_, window, cx| {
                                let _ = new.update(cx, |this, cx| {
                                    this.selected_group = Some(id);
                                    this.new_chat(window, cx);
                                });
                            }),
                        )
                        .item(PopupMenuItem::new("查看全部").on_click(move |_, _, cx| {
                            let _ = open.update(cx, |this, cx| {
                                this.settings = None;
                                this.library = true;
                                this.selected_group = Some(id);
                                this.library_filter = LibraryFilter::Active;
                                cx.notify();
                            });
                        }))
                        .separator()
                        .item(PopupMenuItem::new("编辑名称与目录").on_click(
                            move |_, window, cx| {
                                let _ = edit.update(cx, |this, cx| {
                                    this.edit_group(Some(id), project, window, cx)
                                });
                            },
                        ))
                        .item(PopupMenuItem::new("上移").on_click(move |_, _, cx| {
                            let _ = up.update(cx, |this, cx| this.move_group(id, -1, cx));
                        }))
                        .item(PopupMenuItem::new("下移").on_click(move |_, _, cx| {
                            let _ = down.update(cx, |this, cx| this.move_group(id, 1, cx));
                        }))
                    };
                    body = body.child(
                        div()
                            .id(("group-row", id))
                            .border_t_2()
                            .border_color(cx.theme().transparent)
                            .drag_over::<DragGroup>(move |style, drag, _, cx| {
                                if drag.project == project && drag.id != id {
                                    style.border_color(cx.theme().primary)
                                } else {
                                    style
                                }
                            })
                            .on_drag(
                                DragGroup {
                                    id,
                                    project,
                                    name: group.name.clone(),
                                },
                                |drag, _, _, cx| cx.new(|_| drag.clone()),
                            )
                            .on_drop(cx.listener(move |this, drag: &DragGroup, _, cx| {
                                this.drop_group(drag, Some(id), cx)
                            }))
                            .flex()
                            .items_center()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Self::nav_button(
                                        ("group", id),
                                        group.name.clone(),
                                        if collapsed {
                                            IconName::ChevronRight
                                        } else {
                                            IconName::ChevronDown
                                        },
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if !this.collapsed_groups.remove(&id) {
                                                this.collapsed_groups.insert(id);
                                            }
                                            cx.notify();
                                        },
                                    )),
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(count.to_string()),
                            )
                            .child(
                                Button::new(("group-add", id))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Plus)
                                    .tooltip("在此分组新建对话")
                                    .accessibility_label(format!("在{}新建对话", group.name))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.selected_group = Some(id);
                                        this.new_chat(window, cx);
                                    })),
                            )
                            .child(
                                Button::new(("group-menu", id))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Ellipsis)
                                    .accessibility_label(format!("{}的操作", group.name))
                                    .tooltip("分组操作")
                                    .dropdown_menu(menu.clone()),
                            )
                            .context_menu(menu),
                    );
                    if !collapsed {
                        let mut chats: Vec<_> = self
                            .conversations
                            .iter()
                            .filter(|chat| !chat.archived && self.group_membership(chat, id))
                            .collect();
                        chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
                        if chats.is_empty() {
                            body = body.child(
                                div().pl_6().py_1().child(
                                    Button::new(("group-new", id))
                                        .ghost()
                                        .small()
                                        .label("新建对话")
                                        .icon(IconName::Plus)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.selected_group = Some(id);
                                            this.new_chat(window, cx);
                                        })),
                                ),
                            );
                        }
                        for chat in chats.into_iter().take(20) {
                            body = body.child(div().pl_3().child(self.chat_row(chat, cx)));
                        }
                        if count > 20 {
                            body = body.child(
                                Button::new(("group-all", id))
                                    .small()
                                    .ghost()
                                    .label(format!("查看全部 {count} 个对话"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.library = true;
                                        this.selected_group = Some(id);
                                        this.library_filter = LibraryFilter::Active;
                                        cx.notify();
                                    })),
                            );
                        }
                    }
                }
                body = body.child(
                    div()
                        .id("group-drop-end")
                        .h_4()
                        .border_b_2()
                        .border_color(cx.theme().transparent)
                        .drag_over::<DragGroup>(move |style, drag, _, cx| {
                            if drag.project == project {
                                style.border_color(cx.theme().primary)
                            } else {
                                style
                            }
                        })
                        .on_drop(cx.listener(|this, drag: &DragGroup, _, cx| {
                            this.drop_group(drag, None, cx)
                        })),
                );
            }
        }
        let header = if self.settings.is_some() {
            Self::nav_button("back", "返回对话", IconName::ArrowLeft)
                .on_click(cx.listener(|this, _, window, cx| this.select(this.active, window, cx)))
                .into_any_element()
        } else {
            div()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(div().flex_1().min_w_0().child(
                            Self::nav_button("new-chat", "新建对话", IconName::SquarePen).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.selected_group = None;
                                    this.new_chat(window, cx);
                                }),
                            ),
                        ))
                        .child(
                            Button::new("search-chats")
                                .small()
                                .ghost()
                                .icon(IconName::Search)
                                .tooltip("搜索对话 · ⌘F")
                                .accessibility_label("搜索对话")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.open_search(window, cx)),
                                ),
                        ),
                )
                .children(
                    [
                        ("插件", IconName::Puzzle),
                        ("任务", IconName::ListTodo),
                        ("作品", IconName::Briefcase),
                        ("扩展", IconName::Layers),
                    ]
                    .into_iter()
                    .map(|(label, icon)| {
                        Self::nav_button(label, label, icon)
                            .tooltip("原生预览暂未接入")
                            .accessibility_label(format!("{label}，原生预览暂未接入"))
                            .disabled(true)
                    }),
                )
                .into_any_element()
        };
        div()
            .id("kivio-sidebar")
            .v_flex()
            .size_full()
            .overflow_hidden()
            .text_sm()
            .bg(cx.theme().sidebar)
            .child(div().px_3().pt_2().pb_3().child(header))
            .child(div().mx_3().h(px(1.)).bg(cx.theme().border))
            .child(
                div()
                    .id("sidebar-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(img("kivio/momo.svg").w(px(28.)).h(px(28.)))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("本地预览"),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip("设置 · ⌘,")
                            .accessibility_label("设置")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn chat_row(&self, chat: &Conversation, cx: &mut Context<Self>) -> AnyElement {
        let id = chat.id;
        let archived = chat.archived;
        let owner = cx.entity().downgrade();
        let menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            let rename = owner.clone();
            let archive = owner.clone();
            let remove = owner.clone();
            menu.item(PopupMenuItem::new("重命名").on_click(move |_, window, cx| {
                let _ = rename.update(cx, |this, cx| {
                    this.select(id, window, cx);
                    this.start_rename(window, cx);
                });
            }))
            .item(
                PopupMenuItem::new(if archived {
                    "恢复对话"
                } else {
                    "归档对话"
                })
                .on_click(move |_, _, cx| {
                    let _ = archive.update(cx, |this, cx| {
                        if let Some(chat) = this.conversations.iter_mut().find(|chat| chat.id == id)
                        {
                            chat.archived = !chat.archived;
                        }
                        cx.notify();
                    });
                }),
            )
            .separator()
            .item(
                PopupMenuItem::new("移除此预览对话").on_click(move |_, window, cx| {
                    let _ = remove.update(cx, |this, cx| {
                        if this.active == id {
                            this.remove_current(window, cx);
                        } else {
                            this.conversations.retain(|chat| chat.id != id);
                            cx.notify();
                        }
                    });
                }),
            )
        };
        div()
            .id(("chat-row", id))
            .flex()
            .items_center()
            .min_w_0()
            .rounded_lg()
            .when(
                id == self.active && !self.library && self.settings.is_none(),
                |row| row.bg(cx.theme().sidebar_accent),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Self::nav_button(("chat", id), chat.title.clone(), IconName::MessageSquare)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.select(id, window, cx)),
                        ),
                ),
            )
            .when(chat.view.read(cx).is_streaming(cx), |row| {
                row.child(div().text_xs().child("·"))
            })
            .child(
                Button::new(("chat-more", id))
                    .small()
                    .ghost()
                    .icon(IconName::Ellipsis)
                    .accessibility_label(format!("{}的操作", chat.title))
                    .tooltip("对话操作")
                    .text_color(cx.theme().muted_foreground)
                    .dropdown_menu(menu.clone()),
            )
            .context_menu(menu)
            .into_any_element()
    }
}
