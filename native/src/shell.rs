mod header;
mod library;
mod navigation;
mod search;
mod settings;
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
        OpenSettings,
        ToggleSidebar,
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
    conversations: Vec<Conversation>,
    tab: SidebarTab,
    library: bool,
    library_filter: LibraryFilter,
    groups: Vec<Group>,
    collapsed_groups: std::collections::HashSet<usize>,
    panels: Entity<ResizableState>,
    searching: bool,
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
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("secondary-n", NewChat, Some("Kivio")),
            KeyBinding::new("secondary-f", SearchChats, Some("Kivio")),
            KeyBinding::new("secondary-comma", OpenSettings, Some("Kivio")),
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
        let mut shell = Self {
            conversations: Vec::new(),
            tab: SidebarTab::Recent,
            library: false,
            library_filter: LibraryFilter::Active,
            groups: Vec::new(),
            collapsed_groups: Default::default(),
            panels,
            searching: false,
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
        let view = cx.new(|cx| Preview::new(window, cx));
        let events = cx.subscribe(&view, move |this, _, event, cx| {
            if matches!(
                event,
                ConversationEvent::FirstMessage(_) | ConversationEvent::Submitted
            ) {
                this.activity_clock += 1;
                if let Some(chat) = this.conversations.iter_mut().find(|chat| chat.id == id) {
                    chat.locked = true;
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
            cx.notify();
        });
        self.conversations.insert(
            0,
            Conversation {
                id,
                title: "新对话".into(),
                title_is_user_set: false,
                runtime: Runtime::Agent,
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
        self.settings = None;
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
        self.settings = None;
        self.rename = None;
        self.rename_subscription = None;
        window.close_dialog(cx);
        self.current()
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.group_editor = None;
        self.path_task = None;
        self.library = false;
        self.settings = Some(SettingsPage::Appearance);
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
        self.rename = None;
        self.rename_subscription = None;
        self.current()
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        cx.notify();
    }

    fn remove_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conversations.retain(|chat| chat.id != self.active);
        if let Some(chat) = self.conversations.first() {
            self.select(chat.id, window, cx);
        } else {
            self.new_chat(window, cx);
        }
    }

    fn sample(&mut self, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat(window, cx);
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
        let sidebar_max = (window.bounds().size.width - px(400.)).clamp(px(200.), px(400.));
        let content = div()
            .v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(if self.settings.is_some() {
                self.render_settings(window, cx)
            } else if self.library {
                self.render_library(window, cx)
            } else {
                self.current().view.clone().into_any_element()
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
            .on_action(cx.listener(|this, _: &NewChat, window, cx| {
                this.selected_group = None;
                this.new_chat(window, cx);
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
            .on_action(
                cx.listener(|this, _: &BackToChat, window, cx| {
                    this.select(this.active, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &RenameChat, window, cx| this.start_rename(window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenLibrary, _, cx| this.open_library(cx)))
            .on_action(cx.listener(|this, _: &ArchiveChat, _, cx| {
                if let Some(chat) = this
                    .conversations
                    .iter_mut()
                    .find(|chat| chat.id == this.active)
                {
                    chat.archived = !chat.archived;
                }
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
            .child(
                TitleBar::new()
                    .h(px(44.))
                    .bg(if self.sidebar_visible(window) {
                        cx.theme().sidebar
                    } else {
                        cx.theme().background
                    })
                    .border_b_0()
                    .when(self.sidebar_visible(window), |bar| {
                        bar.child(
                            div()
                                .w(self
                                    .panels
                                    .read(cx)
                                    .sizes()
                                    .first()
                                    .copied()
                                    .unwrap_or(px(240.))
                                    - px(80.))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_end()
                                .pr_2()
                                .gap_1()
                                .child(
                                    SidebarToggleButton::new()
                                        .collapsed(false)
                                        .accessibility_label("收起侧栏")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_sidebar(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("title-new-chat")
                                        .small()
                                        .ghost()
                                        .icon(IconName::SquarePen)
                                        .tooltip("新建对话 · ⌘N")
                                        .accessibility_label("新建对话")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.selected_group = None;
                                            this.new_chat(window, cx);
                                        })),
                                ),
                        )
                    })
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
    }
}

#[cfg(test)]
mod tests;
