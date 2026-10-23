use super::library::{DragConversation, DragGroup};
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
                    (SettingsPage::Providers, "供应商", IconName::Server),
                    (SettingsPage::Appearance, "通用", IconName::Sun),
                    (SettingsPage::Shortcuts, "快捷键", IconName::Keyboard),
                    (SettingsPage::About, "关于", IconName::Info),
                ]
                .into_iter()
                .map(|(value, label, icon)| {
                    Self::nav_button(label, label, icon)
                        .selected(page == value)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_settings_page(Some(value), cx);
                            cx.notify();
                        }))
                }),
            );
        } else {
            // Zeron's stacked disclosure sections, backed by Kivio's existing
            // recent/set/project collections and ADR 0004 ordering.
            for (section_id, section, label) in [
                (0usize, SidebarTab::Recent, "最近"),
                (1, SidebarTab::Sets, "集"),
                (2, SidebarTab::Projects, "项目"),
            ] {
                let collapsed = self.closed_sections.contains(&section_id);
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .pt_3()
                        .pb_1()
                        .child(
                            Button::new(("section", section_id))
                                .ghost()
                                .small()
                                .icon(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .label(label)
                                .accessibility_label(format!(
                                    "{}{}",
                                    if collapsed { "展开" } else { "收起" },
                                    label
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.tab = section;
                                    if !this.closed_sections.remove(&section_id) {
                                        this.closed_sections.insert(section_id);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1())
                        .child(if section == SidebarTab::Recent {
                            Button::new("all-chats")
                                .ghost()
                                .small()
                                .icon(IconName::Archive)
                                .accessibility_label("打开对话库")
                                .tooltip("所有对话与归档")
                                .on_click(cx.listener(|this, _, _, cx| this.open_library(cx)))
                        } else {
                            let project = section == SidebarTab::Projects;
                            Button::new(("add-group", section_id))
                                .ghost()
                                .small()
                                .icon(IconName::Plus)
                                .accessibility_label(if project {
                                    "新建项目"
                                } else {
                                    "新建集"
                                })
                                .tooltip(if project {
                                    "新建项目…"
                                } else {
                                    "新建集…"
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_group(None, project, window, cx)
                                }))
                        }),
                );
                if collapsed {
                    continue;
                }
                if section == SidebarTab::Recent {
                    let mut chats: Vec<_> = self
                        .conversations
                        .iter()
                        .filter(|chat| !chat.archived)
                        .collect();
                    chats.sort_by_key(|chat| std::cmp::Reverse(chat.activity));
                    for chat in chats.into_iter().take(80) {
                        body = body.child(self.chat_row(chat, None, cx));
                    }
                } else {
                    let project = section == SidebarTab::Projects;
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
                        let menu =
                            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                                let edit = owner.clone();
                                let open = owner.clone();
                                let new = owner.clone();
                                let up = owner.clone();
                                let down = owner.clone();
                                let delete = owner.clone();
                                menu.item(PopupMenuItem::new("新建对话").on_click(
                                    move |_, window, cx| {
                                        let _ = new.update(cx, |this, cx| {
                                            if !this.catalog_busy {
                                                this.selected_group = Some(id);
                                                this.new_chat(window, cx);
                                            }
                                        });
                                    },
                                ))
                                .item(PopupMenuItem::new("查看全部").on_click(move |_, _, cx| {
                                    let _ = open.update(cx, |this, cx| {
                                        this.set_settings_page(None, cx);
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
                                .item(PopupMenuItem::new("上移").on_click(move |_, window, cx| {
                                    let _ = up
                                        .update(cx, |this, cx| this.move_group(id, -1, window, cx));
                                }))
                                .item(PopupMenuItem::new("下移").on_click(move |_, window, cx| {
                                    let _ = down
                                        .update(cx, |this, cx| this.move_group(id, 1, window, cx));
                                }))
                                .separator()
                                .item(
                                    PopupMenuItem::new("删除分组").on_click(
                                        move |_, window, cx| {
                                            let _ = delete.update(cx, |this, cx| {
                                                this.delete_group(id, window, cx)
                                            });
                                        },
                                    ),
                                )
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
                                .on_drop(cx.listener(move |this, drag: &DragGroup, window, cx| {
                                    this.drop_group(drag, Some(id), window, cx)
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
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if !this.collapsed_groups.remove(&id) {
                                                    this.collapsed_groups.insert(id);
                                                }
                                                cx.notify();
                                            }),
                                        ),
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
                                        .disabled(self.catalog_busy)
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
                            let chats = self.ordered_group_conversations(id, false);
                            if chats.is_empty() {
                                body = body.child(
                                    div().pl_6().py_1().child(
                                        Button::new(("group-new", id))
                                            .disabled(self.catalog_busy)
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
                                body = body.child(div().pl_3().child(self.chat_row(
                                    chat,
                                    Some(id),
                                    cx,
                                )));
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
                            .id(("group-drop-end", section_id))
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
                            .on_drop(cx.listener(|this, drag: &DragGroup, window, cx| {
                                this.drop_group(drag, None, window, cx)
                            })),
                    );
                }
            }
        }
        let header = if self.settings.is_some() {
            Self::nav_button("back", "返回对话", IconName::ArrowLeft)
                .on_click(cx.listener(|this, _, window, cx| this.select(this.active, window, cx)))
                .into_any_element()
        } else {
            Self::nav_button("search-chats", "搜索对话", IconName::Search)
                .tooltip("搜索对话 · ⌘F")
                .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx)))
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
                            .child("Kivio"),
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

    pub(super) fn chat_row(
        &self,
        chat: &Conversation,
        group: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = chat.id;
        let archived = chat.archived;
        let row = group.and_then(|g| {
            self.ordered_group_conversations(g, archived)
                .iter()
                .position(|chat| chat.id == id)
        });
        let pinned = group
            .and_then(|g| self.groups.iter().find(|group| group.id == g))
            .and_then(|g| self.persistence.pins.get(&g.key))
            .is_some_and(|pins| pins.iter().any(|pin| pin.id == chat.record.id));
        let match_id = self
            .library
            .then(|| {
                Self::match_conversation(
                    chat,
                    &self.search.read(cx).value().trim().to_lowercase(),
                    cx,
                )
            })
            .flatten()
            .and_then(|found| found.message_id);
        let owner = cx.entity().downgrade();
        let menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            let rename = owner.clone();
            let archive = owner.clone();
            let remove = owner.clone();
            let mut menu = menu;
            if let Some(group) = group {
                let pin = owner.clone();
                let up = owner.clone();
                let down = owner.clone();
                menu = menu
                    .item(
                        PopupMenuItem::new(if pinned {
                            "取消固定位置"
                        } else {
                            "固定在此位置"
                        })
                        .checked(pinned)
                        .on_click(move |_, window, cx| {
                            let _ = pin.update(cx, |this, cx| {
                                this.pin_conversation(
                                    id,
                                    group,
                                    if pinned { None } else { row },
                                    window,
                                    cx,
                                )
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new("上移并固定").on_click(move |_, window, cx| {
                            let _ = up.update(cx, |this, cx| {
                                this.move_group_conversation(id, group, -1, window, cx)
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new("下移并固定").on_click(move |_, window, cx| {
                            let _ = down.update(cx, |this, cx| {
                                this.move_group_conversation(id, group, 1, window, cx)
                            });
                        }),
                    )
                    .separator();
            }
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
                .on_click(move |_, window, cx| {
                    let _ = archive.update(cx, |this, cx| {
                        if let Some(chat) = this.conversations.iter_mut().find(|chat| chat.id == id)
                        {
                            chat.archived = !chat.archived;
                        }
                        this.mark_dirty(id, window, cx);
                        cx.notify();
                    });
                }),
            )
            .separator()
            .item(
                PopupMenuItem::new("删除对话").on_click(move |_, window, cx| {
                    let _ = remove.update(cx, |this, cx| {
                        this.delete_conversation(id, window, cx);
                    });
                }),
            )
        };
        div()
            .id(("chat-row", id))
            .when_some(group, |row, group| {
                row.border_t_2()
                    .border_color(cx.theme().transparent)
                    .drag_over::<DragConversation>(move |style, drag, _, cx| {
                        if drag.group == group && drag.id != id {
                            style.border_color(cx.theme().primary)
                        } else {
                            style
                        }
                    })
                    .on_drag(
                        DragConversation {
                            id,
                            group,
                            title: chat.title.clone(),
                        },
                        |drag, _, _, cx| cx.new(|_| drag.clone()),
                    )
                    .on_drop(
                        cx.listener(move |this, drag: &DragConversation, window, cx| {
                            if drag.group == group {
                                this.drop_conversation(drag, id, window, cx);
                            }
                        }),
                    )
            })
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
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select(id, window, cx);
                            if let Some(message) = &match_id {
                                this.current()
                                    .view
                                    .update(cx, |view, cx| view.reveal_message(message, cx));
                            }
                        })),
                ),
            )
            .when(pinned, |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("固定"),
                )
            })
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
