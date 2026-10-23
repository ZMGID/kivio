mod catalog;
mod header;
mod history;
mod library;
mod message_actions;
mod navigation;
mod persistence;
mod search;
mod settings;
mod terminal;
use crate::{
    preview::{ConversationEvent, Preview},
    theme,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt, Theme, ThemeMode, TitleBar,
    WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    resizable::{ResizableState, h_resizable, resizable_panel},
    sidebar::SidebarToggleButton,
    switch::Switch,
    tab::{Tab, TabBar},
};
use gpui_kit::{assets::IconName, prelude::FluentBuilder, *};
use library::{Group, GroupEditor, LibraryFilter, SidebarTab};

actions!(
    kivio_shell,
    [
        NewChat,
        SearchChats,
        CommandPalette,
        ToggleTerminal,
        OpenSettings,
        ToggleSidebar,
        NavigateBack,
        NavigateForward,
        BackToChat,
        RenameChat,
        RemoveChat,
        ArchiveChat,
        OpenLibrary,
        SampleSmall,
        SampleThousand,
        SampleTenThousand
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    Providers,
    Appearance,
    Shortcuts,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Runtime {
    Agent,
    Chat,
}
impl Runtime {
    fn label(self) -> &'static str {
        match self {
            Self::Agent => "Kivio Agent",
            Self::Chat => "Kivio Chat",
        }
    }
}

struct Conversation {
    id: usize,
    record: crate::conversation_data::Conversation,
    persisted: bool,
    dirty: bool,
    record_dirty: bool,
    membership_dirty: bool,
    title: SharedString,
    title_is_user_set: bool,
    runtime: Runtime,
    locked: bool,
    archived: bool,
    activity: usize,
    project: Option<usize>,
    set: Option<usize>,
    view: Entity<Preview>,
    // Subscription lifetime follows the conversation, including removal.
    _events: Subscription,
}

pub struct Shell {
    nav: history::NavHistory,
    closed_sections: std::collections::HashSet<usize>,
    terminal: Option<Entity<crate::terminal::TerminalHost>>,
    terminal_picker: Option<Task<()>>,
    providers: Entity<crate::providers::ProviderSettings>,
    conversations: Vec<Conversation>,
    persistence: persistence::Persistence,
    preferences_busy: bool,
    preferences_ready: bool,
    preferences_task: Option<Task<()>>,
    tab: SidebarTab,
    library: bool,
    library_filter: LibraryFilter,
    groups: Vec<Group>,
    catalog_dir: std::path::PathBuf,
    catalog_busy: bool,
    catalog_ready: bool,
    catalog_task: Option<Task<()>>,
    collapsed_groups: std::collections::HashSet<usize>,
    panels: Entity<ResizableState>,
    searching: bool,
    command_palette: bool,
    search_selection: usize,
    search_scroll: ScrollHandle,
    next_group: usize,
    selected_group: Option<usize>,
    group_editor: Option<GroupEditor>,
    path_task: Option<Task<()>>,
    activity_clock: usize,
    active: usize,
    next_id: usize,
    search: Entity<InputState>,
    rename: Option<Entity<InputState>>,
    message_editor: Option<message_actions::MessageEditor>,
    rename_subscription: Option<Subscription>,
    settings: Option<SettingsPage>,
    sidebar_override: Option<bool>,
    follow_system: bool,
    reduce_motion_override: Option<bool>,
    font_size: f32,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(
        catalog_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("secondary-j", ToggleTerminal, Some("Kivio")),
            KeyBinding::new("secondary-n", NewChat, Some("Kivio")),
            KeyBinding::new("secondary-f", SearchChats, Some("Kivio")),
            KeyBinding::new("secondary-k", CommandPalette, Some("Kivio")),
            KeyBinding::new("secondary-comma", OpenSettings, Some("Kivio")),
            KeyBinding::new("secondary-[", NavigateBack, Some("Kivio")),
            KeyBinding::new("secondary-]", NavigateForward, Some("Kivio")),
            KeyBinding::new("secondary-b", ToggleSidebar, Some("Kivio")),
            KeyBinding::new("escape", BackToChat, Some("Kivio")),
        ]);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索对话"));
        let panels = cx.new(|_| ResizableState::default());
        let subscriptions = vec![
            cx.observe(&panels, |this, _, cx| {
                this.current()
                    .view
                    .update(cx, |view, cx| view.remeasure(cx));
                cx.notify();
            }),
            cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
                if this.searching && matches!(event, InputEvent::Change) {
                    this.search_selection = 0;
                }
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, _, cx| {
                if this.follow_system {
                    theme::apply_system(cx);
                    this.apply_font(cx);
                    this.remeasure(cx);
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    if let Some(reduce) = this.reduce_motion_override {
                        cx.set_reduce_motion(reduce);
                    } else {
                        gpui_kit::base::apply_system_reduce_motion(cx);
                    }
                }
            }),
            cx.observe_window_bounds(window, |this, _, cx| {
                this.current()
                    .view
                    .update(cx, |view, cx| view.remeasure(cx));
                cx.notify();
            }),
        ];
        let providers = cx.new(|cx| {
            crate::providers::ProviderSettings::new(catalog_dir.join("providers.json"), window, cx)
        });
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.subscribe(
            &providers,
            |_, _, _: &crate::providers::ProviderEvent, cx| cx.notify(),
        ));
        let mut shell = Self {
            nav: history::NavHistory::new(0),
            closed_sections: Default::default(),
            terminal: None,
            terminal_picker: None,
            providers,
            conversations: Vec::new(),
            persistence: persistence::Persistence::new(catalog_dir.clone()),
            preferences_busy: false,
            preferences_ready: false,
            preferences_task: None,
            tab: SidebarTab::Recent,
            library: false,
            library_filter: LibraryFilter::Active,
            groups: Vec::new(),
            catalog_dir,
            catalog_busy: false,
            catalog_ready: false,
            catalog_task: None,
            collapsed_groups: Default::default(),
            panels,
            searching: false,
            command_palette: false,
            search_selection: 0,
            search_scroll: ScrollHandle::new(),
            next_group: 0,
            selected_group: None,
            group_editor: None,
            path_task: None,
            activity_clock: 0,
            active: 0,
            next_id: 0,
            search,
            rename: None,
            message_editor: None,
            rename_subscription: None,
            settings: None,
            sidebar_override: None,
            follow_system: true,
            reduce_motion_override: None,
            font_size: 14.,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        shell.new_chat(window, cx);
        shell.catalog_operation(|_| Ok(None), false, window, cx);
        shell.load_preferences(window, cx);
        shell.load_conversations(window, cx);
        shell
    }

    fn current(&self) -> &Conversation {
        self.conversations
            .iter()
            .find(|chat| chat.id == self.active)
            .expect("shell always retains an active conversation")
    }

    fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let group = self
            .selected_group
            .and_then(|id| self.groups.iter().find(|group| group.id == id));
        let project = group.filter(|group| group.project).map(|group| group.id);
        let set = group.filter(|group| !group.project).map(|group| group.id);
        let id = self.next_id;
        self.next_id += 1;
        let record = crate::conversation_data::Conversation::new(String::new(), String::new());
        let view = cx.new(|cx| Preview::new(window, cx));
        view.update(cx, |view, _| {
            view.configure(
                self.providers.clone(),
                None,
                record.id.clone(),
                self.catalog_dir.clone(),
            )
        });
        let events = cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
            if matches!(event, ConversationEvent::ConfigureProviders) {
                this.open_settings(window, cx);
                return;
            }
            if let ConversationEvent::MessageAction(request) = event {
                this.handle_message_action(id, request, window, cx);
                return;
            }
            if matches!(
                event,
                ConversationEvent::FirstMessage(_)
                    | ConversationEvent::Submitted
                    | ConversationEvent::BindingChanged
            ) {
                this.activity_clock += 1;
                if let Some(chat) = this.conversations.iter_mut().find(|chat| chat.id == id) {
                    if let Some((provider, model)) = chat.view.read(cx).binding() {
                        chat.record.provider_id = provider.clone();
                        chat.record.model = model.clone();
                    }
                    if !matches!(event, ConversationEvent::BindingChanged) {
                        chat.locked = true;
                    }
                    chat.activity = this.activity_clock;
                }
            }
            if let ConversationEvent::FirstMessage(body) = event
                && let Some(ix) = this.conversations.iter().position(|chat| chat.id == id)
            {
                let title: String = body
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(28)
                    .collect();
                if !this.conversations[ix].title_is_user_set {
                    this.conversations[ix].title = title.into();
                }
            }
            if matches!(event, ConversationEvent::DraftChanged) {
                this.mark_draft_dirty(id, window, cx);
            } else {
                this.mark_dirty(id, window, cx);
            }
            cx.notify();
        });
        self.conversations.insert(
            0,
            Conversation {
                id,
                record,
                persisted: false,
                dirty: false,
                record_dirty: false,
                membership_dirty: project.is_some() || set.is_some(),
                title: "新对话".into(),
                title_is_user_set: false,
                runtime: Runtime::Chat,
                locked: false,
                archived: false,
                activity: {
                    self.activity_clock += 1;
                    self.activity_clock
                },
                project,
                set,
                view,
                _events: events,
            },
        );
        self.library = false;
        self.group_editor = None;
        self.path_task = None;
        self.active = id;
        self.nav.push(id);
        self.set_settings_page(None, cx);
        self.rename = None;
        self.rename_subscription = None;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn select(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.library = false;
        self.group_editor = None;
        self.path_task = None;
        self.active = id;
        self.nav.push(id);
        self.set_settings_page(None, cx);
        self.rename = None;
        self.rename_subscription = None;
        window.close_dialog(cx);
        self.current()
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        cx.notify();
    }

    fn navigate(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Deleted conversations remain historical entries, but never become a
        // route target. Walking history does not recreate a deleted record.
        while let Some(id) = if forward {
            self.nav.forward()
        } else {
            self.nav.back()
        } {
            if self.conversations.iter().any(|chat| chat.id == id) {
                self.select(id, window, cx);
                return;
            }
        }
        cx.notify();
    }

    fn set_settings_page(&mut self, page: Option<SettingsPage>, cx: &mut Context<Self>) {
        if self.settings == Some(SettingsPage::Providers) && page != self.settings {
            self.providers
                .update(cx, |providers, cx| providers.cancel_pending_login(cx));
        }
        self.settings = page;
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.group_editor = None;
        self.path_task = None;
        self.library = false;
        self.set_settings_page(Some(SettingsPage::Providers), cx);
        self.rename = None;
        self.rename_subscription = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.current().title.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        self.rename_subscription =
            Some(
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.finish_rename(window, cx);
                    }
                }),
            );
        self.rename = Some(input.clone());
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let close = owner.clone();
            let confirm = owner.clone();
            let content = owner
                .update(cx, |this, cx| {
                    div()
                        .v_flex()
                        .gap_4()
                        .when_some(this.rename.as_ref(), |view, input| {
                            view.child(Input::new(input).aria_label("对话名称"))
                        })
                        .child(div().flex().justify_end().child(
                            Button::new("save-title").primary().label("保存").on_click(
                                cx.listener(|this, _, window, cx| this.finish_rename(window, cx)),
                            ),
                        ))
                })
                .ok();
            dialog
                .title("重命名对话")
                .width(px(420.))
                .children(content)
                .on_ok(move |_, window, cx| {
                    let _ = confirm.update(cx, |this, cx| this.finish_rename(window, cx));
                    false
                })
                .on_close(move |_, _, cx| {
                    let _ = close.update(cx, |this, cx| {
                        this.rename = None;
                        this.rename_subscription = None;
                        cx.notify();
                    });
                })
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn finish_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.rename {
            let value = input.read(cx).value();
            let value = value.trim();
            if value.is_empty() {
                return;
            }
            if let Some(chat) = self
                .conversations
                .iter_mut()
                .find(|chat| chat.id == self.active)
            {
                chat.title = value.to_owned().into();
                chat.title_is_user_set = true;
            }
        }
        self.mark_dirty(self.active, window, cx);
        window.close_dialog(cx);
        self.rename = None;
        self.rename_subscription = None;
        self.current()
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        cx.notify();
    }

    fn remove_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_current(window, cx);
    }

    fn sample(&mut self, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat(window, cx);
        self.conversations[0].record.provider_id = "native-preview".into();
        self.conversations[0].record.model = "offline".into();
        self.conversations[0].locked = count > 0;
        self.conversations[0].title = format!("示例对话 · {count} 条").into();
        self.current()
            .view
            .update(cx, |view, cx| view.load_sample(count, cx));
        cx.notify();
    }

    fn sidebar_visible(&self, window: &Window) -> bool {
        self.sidebar_override
            .unwrap_or(window.bounds().size.width >= px(840.))
    }

    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_override = Some(!self.sidebar_visible(window));
        self.current()
            .view
            .update(cx, |view, cx| view.remeasure(cx));
        cx.notify();
    }

    fn remeasure(&self, cx: &mut Context<Self>) {
        for chat in &self.conversations {
            chat.view.update(cx, |view, cx| view.remeasure(cx));
        }
        cx.notify();
    }

    fn apply_font(&self, cx: &mut Context<Self>) {
        Theme::update(cx, |theme| theme.font_size = px(self.font_size));
    }

    fn appearance(&mut self, mode: Option<ThemeMode>, cx: &mut Context<Self>) {
        self.follow_system = mode.is_none();
        match mode {
            Some(mode) => theme::set_mode(mode, cx),
            None => theme::apply_system(cx),
        }
        self.apply_font(cx);
        self.remeasure(cx);
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.persistence.ready {
            return div()
                .size_full()
                .v_flex()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .child(if self.persistence.error.is_some() {
                    "无法读取对话"
                } else {
                    "正在读取对话…"
                })
                .when_some(self.persistence.error.clone(), |view, error| {
                    view.child(error)
                        .child(Button::new("retry-load").label("重试读取").on_click(
                            cx.listener(|this, _, window, cx| this.retry_persistence(window, cx)),
                        ))
                })
                .into_any_element();
        }
        let sidebar_max = (window.bounds().size.width - px(400.)).clamp(px(200.), px(400.));
        let content = div()
            .v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(div().flex_1().min_h_0().child(if self.settings.is_some() {
                self.render_settings(window, cx)
            } else if self.library {
                self.render_library(window, cx)
            } else {
                self.current().view.clone().into_any_element()
            }))
            .when_some(self.terminal.clone(), |content, terminal| {
                content.child(
                    div()
                        .v_flex()
                        .h(px(260.))
                        .min_h(px(120.))
                        .flex_shrink_0()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_3()
                                .child(div().text_sm().child("终端"))
                                .child(
                                    Button::new("close-terminal")
                                        .ghost()
                                        .small()
                                        .icon(IconName::X)
                                        .tooltip("关闭终端进程")
                                        .accessibility_label("关闭终端进程")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_terminal(window, cx)
                                        })),
                                ),
                        )
                        .child(div().flex_1().min_h_0().child(terminal)),
                )
            })
            .into_any_element();
        div()
            .id("kivio-shell")
            .key_context("Kivio")
            .track_focus(&self.focus)
            .v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(
                cx.listener(|this, _: &crate::Quit, window, cx| this.request_quit(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NavigateBack, window, cx| this.navigate(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NavigateForward, window, cx| {
                    this.navigate(true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &NewChat, window, cx| {
                this.selected_group = None;
                this.new_chat(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &ToggleTerminal, window, cx| {
                    this.toggle_terminal(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                this.open_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SearchChats, window, cx| {
                this.open_search(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)),
            )
            .on_action(cx.listener(|this, _: &BackToChat, window, cx| {
                if this.settings.is_none()
                    && !this.library
                    && this.current().view.read(cx).is_streaming(cx)
                {
                    this.current().view.update(cx, |view, cx| view.stop(cx));
                } else {
                    this.select(this.active, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &RenameChat, window, cx| this.start_rename(window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenLibrary, _, cx| this.open_library(cx)))
            .on_action(cx.listener(|this, _: &ArchiveChat, window, cx| {
                if let Some(chat) = this
                    .conversations
                    .iter_mut()
                    .find(|chat| chat.id == this.active)
                {
                    chat.archived = !chat.archived;
                }
                this.mark_dirty(this.active, window, cx);
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &RemoveChat, window, cx| this.remove_current(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SampleSmall, window, cx| this.sample(3, window, cx)))
            .on_action(
                cx.listener(|this, _: &SampleThousand, window, cx| this.sample(1_000, window, cx)),
            )
            .on_action(cx.listener(|this, _: &SampleTenThousand, window, cx| {
                this.sample(10_000, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::Quit, window, cx| this.request_quit(window, cx)),
            )
            .child(
                TitleBar::new()
                    .h(px(44.))
                    .bg(if self.sidebar_visible(window) {
                        cx.theme().sidebar
                    } else {
                        cx.theme().background
                    })
                    .border_b_0()
                    .child(self.render_window_controls(window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .bg(cx.theme().background)
                            .child(self.render_header(window, cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(if self.sidebar_visible(window) {
                        let owner = cx.entity().downgrade();
                        h_resizable("workspace-panels")
                            .with_state(&self.panels)
                            .on_resize(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.remeasure(cx));
                            })
                            .child(
                                resizable_panel()
                                    .size(px(240.))
                                    .size_range(px(200.)..sidebar_max)
                                    .child(self.render_sidebar(window, cx)),
                            )
                            .child(
                                resizable_panel()
                                    .size(px(0.))
                                    .size_range(px(400.)..Pixels::MAX)
                                    .child(content),
                            )
                            .into_any_element()
                    } else {
                        content
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
