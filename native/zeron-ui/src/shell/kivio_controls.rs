//! Kivio runtime, model, thinking and approval controls in the conversation titlebar.

use super::*;
use gpui_kit::base::Disableable;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use kivio_model_runtime::model_metadata::{ThinkingOffMode, thinking_capabilities_for_model};

use crate::kivio::{ChatChoice, chat_choice, set_chat_choice};

#[derive(Clone, Copy, PartialEq)]
enum Agent {
    KivioAgent,
    KivioChat,
    Cli(HarnessId),
    NotPorted,
}

struct AgentTile {
    label: &'static str,
    icon: &'static str,
    agent: Agent,
}

const AGENT_TILES: [AgentTile; 11] = [
    AgentTile { label: "Kivio Agent", icon: icons::KIVIO_MARK, agent: Agent::KivioAgent },
    AgentTile { label: "Kivio Chat", icon: icons::KIVIO_MARK, agent: Agent::KivioChat },
    AgentTile { label: "Claude Code", icon: icons::CLAUDE_MARK, agent: Agent::Cli(HarnessId::ClaudeCode) },
    AgentTile { label: "Codex CLI", icon: icons::OPENAI_MARK, agent: Agent::Cli(HarnessId::Codex) },
    AgentTile { label: "OpenCode", icon: icons::OPENCODE_MARK, agent: Agent::Cli(HarnessId::Opencode) },
    AgentTile { label: "Kimi CLI", icon: icons::KIMI_MARK, agent: Agent::NotPorted },
    AgentTile { label: "Pi", icon: icons::PI_MARK, agent: Agent::Cli(HarnessId::Pi) },
    AgentTile { label: "Hermes", icon: icons::HERMES_MARK, agent: Agent::Cli(HarnessId::Hermes) },
    AgentTile { label: "Grok CLI", icon: icons::GROK_MARK, agent: Agent::Cli(HarnessId::Grok) },
    AgentTile { label: "DeepSeek Harness", icon: icons::DSH_MARK, agent: Agent::NotPorted },
    AgentTile { label: "Antigravity CLI", icon: icons::ANTIGRAVITY_MARK, agent: Agent::Cli(HarnessId::Antigravity) },
];

const APPROVAL_POLICIES: [(&str, &str); 3] = [
    ("always_confirm", "每次确认"),
    ("readonly_auto_sensitive_confirm", "敏感确认"),
    ("auto", "完全访问"),
];

#[derive(Default)]
pub(super) struct ChoiceCache {
    chat_id: String,
    modified: Option<std::time::SystemTime>,
    choice: Option<ChatChoice>,
}

fn level_label(level: &str) -> SharedString {
    match level {
        "off" => "Off".into(),
        "xhigh" => "XHigh".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
                .into()
        }
    }
}

impl Shell {
    pub(super) fn ensure_provider_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.provider_page.is_some() {
            return;
        }
        let path = self.data_dir.join("conversations/providers.json");
        let page = cx.new(|cx| crate::providers::ProviderSettings::new(path, window, cx));
        self.kivio_provider_sub = Some(cx.observe(&page, |_, _, cx| cx.notify()));
        self.provider_page = Some(page);
    }

    fn kivio_chat_choice(&mut self, chat_id: &str) -> Option<ChatChoice> {
        let file = self.data_dir.join("conversations").join(format!("{chat_id}.json"));
        let modified = std::fs::metadata(&file).and_then(|meta| meta.modified()).ok();
        let cache = &mut self.kivio_choice_cache;
        if cache.chat_id != chat_id || cache.modified != modified {
            *cache = ChoiceCache {
                chat_id: chat_id.to_owned(),
                modified,
                choice: chat_choice(&self.data_dir, chat_id),
            };
        }
        cache.choice.clone()
    }

    fn kivio_approval_policy(&mut self) -> String {
        let file = self.data_dir.join("settings.json");
        let modified = std::fs::metadata(&file).and_then(|meta| meta.modified()).ok();
        if self.kivio_approval.0 != modified || self.kivio_approval.1.is_empty() {
            let policy = kivio_model_runtime::native_preferences::load(&self.data_dir)
                .map(|settings| settings.chat_tools.approval_policy)
                .unwrap_or_default();
            self.kivio_approval = (modified, policy);
        }
        self.kivio_approval.1.clone()
    }

    pub(super) fn close_kivio_agent_menu(&mut self, cx: &mut Context<Self>) {
        if self.kivio_agent_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.kivio_agent_menu);
            cx.notify();
        }
    }

    fn select_kivio_agent(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let composer = self.composer.clone();
        match agent {
            Agent::KivioAgent | Agent::KivioChat => composer.update(cx, |this, cx| {
                this.set_native_cli(false, cx);
                this.set_kivio_chat_mode(agent == Agent::KivioChat, cx);
            }),
            Agent::Cli(harness) => {
                composer.update(cx, |this, cx| this.set_native_cli(true, cx));
                let pickers = composer.read(cx).pickers().clone();
                pickers.update(cx, |pickers, cx| pickers.select_harness(harness, cx));
            }
            Agent::NotPorted => {
                self.show_notice(crate::toast::ToastKind::Info, "此代理尚未迁移到 GPUI 版本", cx);
                return;
            }
        }
        self.close_kivio_agent_menu(cx);
    }

    fn render_kivio_agent_trigger(
        &mut self,
        current: Agent,
        locked: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.kivio_agent_menu.is_open();
        let (glyph, tint) = match current {
            Agent::Cli(harness) => crate::pickers::harness_brand_icon(harness),
            Agent::KivioChat => (icons::KIVIO_MARK, Some(theme.accent)),
            _ => (icons::KIVIO_MARK, None),
        };
        let trigger = div()
            .id("kivio-agent-trigger")
            .debug_selector(|| "kivio-agent-trigger".into())
            .role(gpui_kit::Role::Button)
            .aria_label("代理")
            .aria_expanded(open)
            .size(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.0))
            .cursor_pointer()
            .bg(if open {
                theme.glass_hover()
            } else {
                motion::hover_blend(
                    "kivio-agent-trigger",
                    theme.glass_hover().opacity(0.0),
                    theme.glass_hover(),
                )
            })
            .on_hover(motion::hover_listener("kivio-agent-trigger"))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    window.prevent_default();
                    this.kivio_agent_menu.note_trigger_press();
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if this.kivio_agent_menu.take_press_was_open() {
                    this.close_kivio_agent_menu(cx);
                } else {
                    let pickers = this.composer.read(cx).pickers().clone();
                    pickers.update(cx, |pickers, cx| pickers.load_harness_catalog(false, cx));
                    this.kivio_agent_menu.open(());
                }
                cx.notify();
            }))
            .child(
                icon(glyph)
                    .size(px(16.0))
                    .text_color(tint.unwrap_or(theme.text)),
            );
        if self.kivio_agent_menu.get().is_none() {
            return trigger.into_any_element();
        }
        let closing = self.kivio_agent_menu.closing_since();
        let menu = self.render_kivio_agent_menu(current, locked, theme, cx);
        trigger
            .relative()
            .child(popover::anchored_menu_below("kivio-agent-menu", menu, closing))
            .into_any_element()
    }

    fn render_kivio_agent_menu(
        &mut self,
        current: Agent,
        locked: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &theme.for_popup();
        let offered = self.composer.read(cx).pickers().read(cx).offered_harness_ids();
        let mut tiles = Vec::new();
        for (index, tile) in AGENT_TILES.iter().enumerate() {
            let selected = tile.agent == current;
            let available = match tile.agent {
                Agent::Cli(harness) => offered.contains(&harness),
                Agent::NotPorted => false,
                _ => true,
            };
            let enabled = available && !locked;
            let (glyph, tint) = match tile.agent {
                Agent::Cli(harness) => crate::pickers::harness_brand_icon(harness),
                Agent::KivioChat => (tile.icon, Some(theme.accent)),
                _ => (tile.icon, None),
            };
            let agent = tile.agent;
            let id = SharedString::from(format!("kivio-agent-{index}"));
            tiles.push(
                div()
                    .id(id.clone())
                    .debug_selector({
                        let id = id.clone();
                        move || id.to_string()
                    })
                    .w(px(136.0))
                    .h(px(34.0))
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(if selected { theme.accent } else { theme.border })
                    .bg(if selected {
                        theme.accent.opacity(0.12)
                    } else {
                        theme.glass_hover().opacity(0.0)
                    })
                    .opacity(if available { 1.0 } else { 0.45 })
                    .when(enabled, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(theme.glass_hover()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_kivio_agent(agent, cx)
                            }))
                    })
                    .when(agent == Agent::NotPorted, |el| {
                        el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                            this.select_kivio_agent(agent, cx)
                        }))
                    })
                    .child(
                        icon(glyph)
                            .size(px(18.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(theme.text)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(crate::typography::ui_rems(13.0))
                            .child(SharedString::from(tile.label)),
                    )
                    .into_any_element(),
            );
        }
        let refresh = div()
            .id("kivio-agent-refresh")
            .role(gpui_kit::Role::Button)
            .aria_label("重新检测")
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|el| el.bg(theme.glass_hover()))
            .on_click(cx.listener(|this, _, _, cx| {
                let pickers = this.composer.read(cx).pickers().clone();
                pickers.update(cx, |pickers, cx| pickers.load_harness_catalog(true, cx));
            }))
            .child(icon(icons::REFRESH).size(px(14.0)).text_color(theme.text_muted));
        popover::popover_card(theme)
            .w(px(2.0 * 136.0 + 8.0 + 2.0 * 6.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_kivio_agent_menu(cx)))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(4.0))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(12.0))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(theme.text_muted)
                            .child("代理"),
                    )
                    .child(refresh),
            )
            .child(div().flex().flex_row().flex_wrap().gap(px(8.0)).children(tiles))
            .when(locked, |el| {
                el.child(
                    div()
                        .px(px(4.0))
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child("对话已开始，代理不可更换"),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_kivio_controls(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let page = self.provider_page.clone()?;
        let (chat_id, chat_harness) = {
            let state = self.state.read(cx);
            match state.selected_chat_row() {
                Some(chat) => (Some(chat.id.clone()), chat.config.as_ref().map(|c| c.harness)),
                None => (None, None),
            }
        };
        let composer = self.composer.clone();
        let existing = chat_id.is_some();
        let cli = if existing { chat_harness.is_some() } else { composer.read(cx).native_cli() };
        let busy = composer.read(cx).is_sending()
            || chat_id.as_deref().is_some_and(|id| {
                let state = self.state.read(cx);
                state.chats.iter().any(|chat| {
                    chat.id == id
                        && state.display_status_for(chat, Utc::now())
                            == zeron_proto::ChatIndicator::Working
                })
            });
        let shell = cx.weak_entity();
        let saved = chat_id.as_deref().and_then(|id| self.kivio_chat_choice(id));
        let chat_mode = match (&saved, existing) {
            (Some(choice), _) => choice.chat_mode,
            (None, true) => false,
            (None, false) => composer.read(cx).kivio_chat_mode(),
        };
        let current_agent = if cli {
            let pickers = composer.read(cx).pickers().clone();
            match chat_harness.or_else(|| pickers.read(cx).current_harness(cx)) {
                Some(harness) => Agent::Cli(harness),
                None => Agent::KivioAgent,
            }
        } else if chat_mode {
            Agent::KivioChat
        } else {
            Agent::KivioAgent
        };

        let mut row = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .child(self.render_kivio_agent_trigger(current_agent, existing || busy, theme, cx));
        if cli {
            return Some(row.into_any_element());
        }

        let default = page.read(cx).selection();
        let current = match saved {
            Some(choice) => Some((choice.provider_id, choice.model, choice.thinking)),
            None => default.as_ref().ok().map(|(provider, model)| {
                (
                    provider.id.clone(),
                    model.clone(),
                    composer.read(cx).kivio_thinking().map(str::to_owned),
                )
            }),
        };
        let models = page.read(cx).models();
        let directory = self.data_dir.clone();
        let model_label: SharedString = current
            .as_ref()
            .map(|(_, model, _)| SharedString::from(model.clone()))
            .unwrap_or_else(|| "选择模型".into());
        let model_button = {
            let current = current.clone();
            let page = page.clone();
            let directory = directory.clone();
            let chat_id = chat_id.clone();
            let shell = shell.clone();
            let composer = composer.downgrade();
            Button::new("kivio-model")
                .ghost()
                .compact()
                .label(model_label)
                .disabled(busy)
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    if models.is_empty() {
                        let shell = shell.clone();
                        return menu.item(PopupMenuItem::new("配置供应商…").on_click(
                            move |_, _, cx| { let _ = shell.update(cx, |this, cx| this.open_settings(SettingsSection::Harnesses, cx)); },
                        ));
                    }
                    menu = menu.max_h(px(360.0)).scrollable(true);
                    for choice in &models {
                        let checked = current
                            .as_ref()
                            .is_some_and(|(provider, model, _)| {
                                *provider == choice.provider_id && *model == choice.model
                            });
                        let (provider_id, model) = (choice.provider_id.clone(), choice.model.clone());
                        let (page, directory, chat_id, shell, composer) = (
                            page.clone(),
                            directory.clone(),
                            chat_id.clone(),
                            shell.clone(),
                            composer.clone(),
                        );
                        menu = menu.item(PopupMenuItem::new(choice.label.clone()).checked(checked).on_click(
                            move |_, window, cx| {
                                let _ = composer.update(cx, |this, cx| this.set_kivio_thinking(None, cx));
                                match &chat_id {
                                    Some(id) => {
                                        let result = set_chat_choice(
                                            &directory,
                                            id,
                                            ChatChoice {
                                                provider_id: provider_id.clone(),
                                                model: model.clone(),
                                                thinking: None,
                                                chat_mode,
                                            },
                                        );
                                        let _ = shell.update(cx, |this, cx| {
                                            if let Err(error) = result {
                                                this.show_notice(crate::toast::ToastKind::Error, error, cx);
                                            }
                                            this.kivio_choice_cache = ChoiceCache::default();
                                            cx.notify();
                                        });
                                    }
                                    None => page.update(cx, |page, cx| {
                                        page.select_model(provider_id.clone(), model.clone(), window, cx)
                                    }),
                                }
                            },
                        ));
                    }
                    menu
                })
        };
        row = row.child(model_button);

        let capabilities = current.as_ref().and_then(|(provider_id, model, _)| {
            page.read(cx)
                .selected(provider_id, model)
                .ok()
                .map(|(provider, model)| thinking_capabilities_for_model(Some(&provider), &model))
        });
        if let (Some((_, _, thinking)), Some(capabilities)) = (current.clone(), capabilities) {
            let can_turn_off =
                !matches!(capabilities.off_mode, ThinkingOffMode::Unsupported | ThinkingOffMode::NotApplicable);
            let mut options: Vec<String> = Vec::new();
            if can_turn_off && !capabilities.levels.is_empty() {
                options.push("off".into());
            }
            options.extend(capabilities.levels.iter().cloned());
            if !options.is_empty() {
                let effective = thinking
                    .clone()
                    .filter(|level| options.contains(level))
                    .unwrap_or_else(|| {
                        if options.iter().any(|level| level == "high") {
                            "high".into()
                        } else {
                            options.last().cloned().unwrap_or_default()
                        }
                    });
                let directory = directory.clone();
                let chat_id = chat_id.clone();
                let shell = shell.clone();
                let composer = composer.downgrade();
                let choice_base = current.clone();
                let thinking_button = Button::new("kivio-thinking")
                    .ghost()
                    .compact()
                    .icon(Icon::empty().path(icons::TUNING))
                    .label(level_label(&effective))
                    .disabled(busy)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for level in &options {
                            let level = level.clone();
                            let (directory, chat_id, shell, composer, choice_base) = (
                                directory.clone(),
                                chat_id.clone(),
                                shell.clone(),
                                composer.clone(),
                                choice_base.clone(),
                            );
                            menu = menu.item(
                                PopupMenuItem::new(level_label(&level))
                                    .checked(level == effective)
                                    .on_click(move |_, _, cx| match (&chat_id, &choice_base) {
                                        (Some(id), Some((provider_id, model, _))) => {
                                            let result = set_chat_choice(
                                                &directory,
                                                id,
                                                ChatChoice {
                                                    provider_id: provider_id.clone(),
                                                    model: model.clone(),
                                                    thinking: Some(level.clone()),
                                                    chat_mode,
                                                },
                                            );
                                            let _ = shell.update(cx, |this, cx| {
                                                if let Err(error) = result {
                                                    this.show_notice(crate::toast::ToastKind::Error, error, cx);
                                                }
                                                this.kivio_choice_cache = ChoiceCache::default();
                                                cx.notify();
                                            });
                                        }
                                        _ => {
                                            let _ = composer.update(cx, |this, cx| {
                                                this.set_kivio_thinking(Some(level.clone()), cx)
                                            });
                                        }
                                    }),
                            );
                        }
                        menu
                    });
                row = row.child(thinking_button);
            }
        }

        let policy = self.kivio_approval_policy();
        let shield = {
            let directory = directory.clone();
            let shell = shell.clone();
            let current_policy = policy.clone();
            Button::new("kivio-approval")
                .ghost()
                .compact()
                .icon(Icon::empty().path(icons::SHIELD_CHECK))
                .tooltip("工具审批")
                .dropdown_menu(move |mut menu, _, _| {
                    for (value, label) in APPROVAL_POLICIES {
                        let (directory, shell, before) =
                            (directory.clone(), shell.clone(), current_policy.clone());
                        menu = menu.item(PopupMenuItem::new(label).checked(before == value).on_click(
                            move |_, _, cx| {
                                let result = kivio_model_runtime::native_preferences::save(
                                    &directory,
                                    kivio_model_runtime::native_preferences::Change::Fields(vec![
                                        kivio_model_runtime::native_preferences::FieldEdit {
                                            path: "/chatTools/approvalPolicy".into(),
                                            before: before.clone().into(),
                                            after: value.into(),
                                        },
                                    ]),
                                );
                                let _ = shell.update(cx, |this, cx| {
                                    if let Err(error) = result {
                                        this.show_notice(crate::toast::ToastKind::Error, error, cx);
                                    }
                                    this.kivio_approval = (None, String::new());
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                })
        };
        row = row.child(shield);
        let _ = theme;
        Some(row.into_any_element())
    }
}

impl Shell {
    pub(crate) fn lens_engine(&self, cx: &App) -> Option<crate::state::EngineHandle> {
        self.state.read(cx).engine().cloned()
    }
    pub(crate) fn open_lens_chat(&mut self, chat: zeron_proto::Chat, cx: &mut Context<Self>) {
        let id = chat.id.clone();
        self.state.update(cx, |state, cx| {
            let mut chats = state.chats.clone();
            chats.retain(|existing| existing.id != id); chats.push(chat);
            state.apply_chats(chats); state.select_chat(Some(id.clone()), cx);
        });
        self.nav.push(NavEntry::Chat(id));
        self.set_route(Route::Chat, cx);
        self.focus_composer(cx);
        cx.notify();
    }
}
