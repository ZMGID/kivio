use super::*;

impl Shell {
    // Zeron shell.rs::render_titlebar_cluster, using Kit's semantic controls.
    pub(super) fn render_window_controls(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_1()
            .pr_2()
            .child(
                SidebarToggleButton::new()
                    .collapsed(!self.sidebar_visible(window))
                    .accessibility_label("切换侧栏")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx))),
            )
            .child(
                Button::new("nav-back")
                    .small()
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .disabled(!self.nav.can_back())
                    .accessibility_label("后退")
                    .tooltip("后退 · ⌘[")
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(false, window, cx))),
            )
            .child(
                Button::new("nav-forward")
                    .small()
                    .ghost()
                    .icon(IconName::ArrowRight)
                    .disabled(!self.nav.can_forward())
                    .accessibility_label("前进")
                    .tooltip("前进 · ⌘]")
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(true, window, cx))),
            )
            .child(
                Button::new("title-new-chat")
                    .small()
                    .ghost()
                    .icon(IconName::Plus)
                    .accessibility_label("新建对话")
                    .tooltip("新建对话 · ⌘N")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected_group = None;
                        this.new_chat(window, cx);
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_header(&self, _window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let in_chat = self.settings.is_none() && !self.library;
        let mut leading = div().flex().items_center().gap_2().flex_1().min_w_0();
        if in_chat {
            let runtime = self.current().runtime;
            let locked = self.current().locked;
            let chat_id = self.active;
            let owner = cx.entity().downgrade();
            leading = leading.child(
                Button::new("runtime")
                    .ghost()
                    .small()
                    .label(runtime.label())
                    .dropdown_caret(true)
                    .when(locked, |button| button.icon(IconName::Lock))
                    .tooltip(if locked {
                        "已有消息，运行时已锁定"
                    } else {
                        "选择运行时"
                    })
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu.label(if locked {
                            "已有消息，运行时已锁定"
                        } else {
                            "选择此对话的运行时"
                        });
                        for value in [Runtime::Agent, Runtime::Chat] {
                            let owner = owner.clone();
                            menu = menu.item(
                                PopupMenuItem::new(value.label())
                                    .checked(runtime == value)
                                    .disabled(locked || value == Runtime::Agent)
                                    .on_click(move |_, window, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            if let Some(chat) = this
                                                .conversations
                                                .iter_mut()
                                                .find(|chat| chat.id == chat_id && !chat.locked)
                                            {
                                                chat.runtime = value;
                                            }
                                            this.mark_dirty(chat_id, window, cx);
                                            cx.notify();
                                        });
                                    }),
                            );
                        }
                        menu.separator()
                            .item(PopupMenuItem::new("外部 CLI 暂未接入").disabled(true))
                    }),
            );
            leading = leading.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.current().title.clone()),
            );
        } else {
            leading = leading.child(div().text_sm().child(if self.settings.is_some() {
                "设置"
            } else if self.group_editor.is_some() {
                "整理对话"
            } else {
                "对话库"
            }));
        }
        let mut trailing = div().flex().items_center().gap_1().child(
            Button::new("toggle-terminal")
                .ghost()
                .small()
                .label("终端")
                .tooltip("打开或关闭终端 · ⌘J")
                .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx))),
        );
        if self.persistence.error.is_some() {
            trailing = trailing.child(
                Button::new("retry-save")
                    .small()
                    .label("重试保存")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.retry_persistence(window, cx)),
                    ),
            );
        }
        if in_chat {
            let owner = cx.entity().downgrade();
            let id = self.active;
            let project = self.current().project;
            let set = self.current().set;
            let groups: Vec<_> = self
                .groups
                .iter()
                .map(|group| (group.id, group.name.clone(), group.project))
                .collect();
            trailing = trailing
                .child(
                    Button::new("assign-group")
                        .ghost()
                        .small()
                        .icon(IconName::Folder)
                        .accessibility_label("整理到集或项目")
                        .tooltip("整理到集或项目")
                        .dropdown_menu(move |mut menu, _, _| {
                            menu = menu.label("整理到集或项目");
                            for (group_id, name, is_project) in &groups {
                                let group_id = *group_id;
                                let is_project = *is_project;
                                let owner = owner.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(format!(
                                        "{} · {}",
                                        if is_project { "项目" } else { "集" },
                                        name
                                    ))
                                    .checked(if is_project {
                                        project == Some(group_id)
                                    } else {
                                        set == Some(group_id)
                                    })
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                let selected = if is_project {
                                                    project == Some(group_id)
                                                } else {
                                                    set == Some(group_id)
                                                };
                                                this.assign_conversation_group(
                                                    id,
                                                    if selected { None } else { Some(group_id) },
                                                    window,
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                );
                            }
                            if groups.is_empty() {
                                menu = menu.item(
                                    PopupMenuItem::new("先在侧栏新建集或项目").disabled(true),
                                );
                            }
                            menu
                        }),
                )
                .child(
                    Button::new("chat-menu")
                        .ghost()
                        .small()
                        .icon(IconName::Ellipsis)
                        .accessibility_label("对话操作")
                        .tooltip("对话操作")
                        .dropdown_menu({
                            let archived = self.current().archived;
                            move |menu, _, _| {
                                menu.menu("重命名对话", Box::new(RenameChat))
                                    .menu(
                                        if archived {
                                            "恢复对话"
                                        } else {
                                            "归档对话"
                                        },
                                        Box::new(ArchiveChat),
                                    )
                                    .menu("打开对话库", Box::new(OpenLibrary))
                                    .separator()
                                    .menu("删除对话", Box::new(RemoveChat))
                                    .separator()
                                    .menu("开发样本 · 短对话", Box::new(SampleSmall))
                                    .menu("开发样本 · 1,000 条", Box::new(SampleThousand))
                                    .menu("开发样本 · 10,000 条", Box::new(SampleTenThousand))
                            }
                        }),
                );
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(44.))
            .px_3()
            .gap_2()
            .flex_shrink_0()
            .child(leading)
            .child(trailing)
            .into_any_element()
    }
}
