//! Kivio hook list and editor, using the existing HookDef and settings writer.
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    native_preferences::{self, Change},
    tool_settings::HookDef,
};

const EVENTS: [(&str, &str); 8] = [
    ("agent_start", "对话开始"),
    ("turn_start", "轮次开始"),
    ("message_start", "消息开始"),
    ("message_end", "消息结束"),
    ("tool_execution_start", "工具执行前"),
    ("tool_execution_end", "工具执行后"),
    ("turn_end", "轮次结束"),
    ("agent_end", "对话结束"),
];
fn event_label(event: &str) -> String {
    EVENTS
        .iter()
        .find(|(key, _)| *key == event)
        .map(|(_, label)| *label)
        .unwrap_or(event)
        .to_owned()
}
pub struct HooksPage {
    directory: std::path::PathBuf,
    hooks: Vec<HookDef>,
    failures: Vec<serde_json::Value>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    editor: Option<WeakEntity<HookEditor>>,
    editor_events: Option<Subscription>,
}
impl HooksPage {
    pub(crate) fn is_busy(&self, cx: &App) -> bool {
        self.busy
            || self
                .editor
                .as_ref()
                .and_then(|editor| editor.upgrade())
                .is_some_and(|editor| editor.read(cx).busy)
    }

    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .and_then(|editor| editor.upgrade())
            .is_some_and(|editor| editor.read(cx).has_unsaved_changes(cx))
    }

    pub fn new(directory: std::path::PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            directory,
            hooks: vec![],
            failures: vec![],
            busy: false,
            error: None,
            task: None,
            editor: None,
            editor_events: None,
        };
        this.reload(None, cx);
        this
    }
    fn reload(&mut self, change: Option<Change>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let directory = self.directory.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let failures = kivio_model_runtime::native_hooks::failures(&directory)?;
                    let settings = match change {
                        Some(change) => native_preferences::save(&directory, change),
                        None => native_preferences::load(&directory),
                    }?;
                    Ok::<_, String>((settings, failures))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((settings, failures)) => {
                        this.failures = failures;
                        this.hooks = settings.chat_tools.hooks;
                        this.hooks.sort_by_key(|hook| {
                            EVENTS
                                .iter()
                                .position(|(event, _)| *event == hook.event)
                                .unwrap_or(99)
                        });
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn edit(
        &mut self,
        hook: Option<HookDef>,
        event: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.new(|cx| HookEditor::new(self.directory.clone(), hook, event, window, cx));
        self.editor_events = Some(cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
            this.editor = None;
            this.editor_events = None;
            if matches!(event, EditorEvent::Saved) {
                this.reload(None, cx);
            }
            cx.notify();
        }));
        self.editor = Some(editor.downgrade());
        let owner = cx.weak_entity();
        let editor_id = editor.entity_id();
        window.open_dialog(cx, move |dialog, _, cx| {
            let owner = owner.clone();
            dialog
                .title(if editor.read(cx).before.is_some() {
                    "编辑 Hook"
                } else {
                    "新建 Hook"
                })
                .close_button(!editor.read(cx).busy)
                .keyboard(!editor.read(cx).busy)
                .width(px(440.))
                .overlay_closable(false)
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if this
                            .editor
                            .as_ref()
                            .is_some_and(|editor| editor.entity_id() == editor_id)
                        {
                            this.editor = None;
                            this.editor_events = None;
                            cx.notify();
                        }
                    });
                })
                .child(editor.clone())
        });
    }
}
impl Render for HooksPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let add = Button::new("hook-add")
            .small()
            .label(crate::i18n::text("新建 Hook…"))
            .disabled(self.busy)
            .dropdown_menu(move |mut menu, _, _| {
                for (event, label) in EVENTS {
                    let owner = owner.clone();
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let _ =
                            owner.update(cx, |this, cx| this.edit(None, event.into(), window, cx));
                    }));
                }
                menu
            });
        let mut card = div().v_flex().gap_3().p_4().bg(cx.theme().group_box).rounded(px(12.))
            .child(div().flex().items_center().justify_between().gap_3()
                .child(div().v_flex().gap_2().child(div().font_weight(FontWeight::MEDIUM).child(crate::i18n::text("已配置的 Hook")))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::text("按触发顺序排列。失败只报警告，不会打断对话，也不能拒绝或改写工具调用。"))))
                .child(div().flex().gap_2().child(Button::new("hooks-refresh").small().label(crate::i18n::text("刷新")).disabled(self.busy).on_click(cx.listener(|this, _, _, cx| this.reload(None, cx)))).child(add)));
        if let Some(error) = &self.error {
            card = card.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        for failure in self.failures.iter().rev().take(8) {
            card = card.child(div().text_sm().text_color(cx.theme().danger).child(format!(
                "{} · {}：{}",
                failure["hookName"].as_str().unwrap_or("Hook"),
                failure["event"].as_str().unwrap_or(""),
                failure["message"].as_str().unwrap_or("执行失败")
            )));
        }
        if self.hooks.is_empty() {
            card = card.child(
                div()
                    .v_flex()
                    .items_center()
                    .gap_3()
                    .py_10()
                    .child(if self.busy { crate::i18n::text("正在读取…") } else { crate::i18n::text("还没有 Hook") })
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::i18n::text("在对话开始、工具执行前后、对话结束等时机执行脚本或 HTTP 请求。"),
                            ),
                    ),
            );
        }
        for hook in &self.hooks {
            let toggle_hook = hook.clone();
            let edit_hook = hook.clone();
            let delete_hook = hook.clone();
            let summary = if !hook.description.trim().is_empty() {
                hook.description.clone()
            } else if hook.kind == "http" {
                format!("{} {}", hook.method, hook.url)
            } else {
                hook.script
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("")
                    .to_owned()
            };
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap_1()
                            .child(div().child(format!(
                                "{} · {}",
                                hook.name,
                                event_label(&hook.event)
                            )))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(summary),
                            ),
                    )
                    .child(
                        Switch::new(SharedString::from(format!("hook-enabled-{}", hook.id)))
                            .checked(hook.enabled)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let mut after = toggle_hook.clone();
                                after.enabled = *checked;
                                this.reload(
                                    Some(Change::Hook {
                                        before: Some(toggle_hook.clone()),
                                        after: Some(after),
                                    }),
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("hook-edit-{}", hook.id)))
                            .small()
                            .label(crate::i18n::text("编辑"))
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit(
                                    Some(edit_hook.clone()),
                                    edit_hook.event.clone(),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("hook-delete-{}", hook.id)))
                            .small()
                            .label(crate::i18n::text("删除"))
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(move |_, _, window, cx| {
                                let owner = cx.weak_entity();
                                let hook = delete_hook.clone();
                                window.open_dialog(cx, move |dialog, _, _| {
                                    let owner = owner.clone();
                                    let hook = hook.clone();
                                    dialog
                                        .title(crate::i18n::text("删除 Hook"))
                                        .child(crate::ui_format!("删除“{}”？此操作会移除这条 Hook 配置。", "Delete “{}”? This removes the hook configuration.",
                                            hook.name
                                        ))
                                        .footer(
                                            div()
                                                .flex()
                                                .justify_end()
                                                .gap_2()
                                                .child(
                                                    Button::new("cancel-delete-hook")
                                                        .small()
                                                        .label(crate::i18n::text("取消"))
                                                        .on_click(|_, window, cx| {
                                                            window.close_dialog(cx)
                                                        }),
                                                )
                                                .child(
                                                    Button::new("confirm-delete-hook")
                                                        .small()
                                                        .label(crate::i18n::text("删除"))
                                                        .on_click(move |_, window, cx| {
                                                            let _ = owner.update(cx, |this, cx| {
                                                                this.reload(
                                                                    Some(Change::Hook {
                                                                        before: Some(hook.clone()),
                                                                        after: None,
                                                                    }),
                                                                    cx,
                                                                )
                                                            });
                                                            window.close_dialog(cx);
                                                        }),
                                                ),
                                        )
                                });
                            })),
                    ),
            );
        }
        if !self.hooks.is_empty() {
            card = card.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::ui_format!("共 {} 条，已启用 {} 条", "{} total; {} enabled",
                        self.hooks.len(),
                        self.hooks.iter().filter(|hook| hook.enabled).count()
                    )),
            );
        }
        crate::kivio_settings::page_frame(
            "Hooks",
            "内置对话已接入 Agent、轮次与消息的六个起止事件。工具前后事件会随原生工具执行入口接通。",
            card.into_any_element(),
            cx,
        )
    }
}
enum EditorEvent {
    Saved,
    Closed,
}
struct HookEditor {
    directory: std::path::PathBuf,
    before: Option<HookDef>,
    draft: HookDef,
    baseline: HookDef,
    name: Entity<InputState>,
    description: Entity<InputState>,
    url: Entity<InputState>,
    timeout: Entity<InputState>,
    script: Entity<TextareaState>,
    headers: Entity<TextareaState>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
}
impl EventEmitter<EditorEvent> for HookEditor {}
impl HookEditor {
    fn has_unsaved_changes(&self, cx: &App) -> bool {
        let baseline = &self.baseline;
        self.name.read(cx).value().as_ref() != baseline.name
            || self.description.read(cx).value().as_ref() != baseline.description
            || self.url.read(cx).value().as_ref() != baseline.url
            || self.timeout.read(cx).value().as_ref() != baseline.timeout_ms.to_string()
            || self.script.read(cx).value().as_ref() != baseline.script
            || self.headers.read(cx).value().as_ref()
                != baseline
                    .headers
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            || self.draft.event != baseline.event
            || self.draft.kind != baseline.kind
            || self.draft.method != baseline.method
            || self.draft.enabled != baseline.enabled
    }

    fn new(
        directory: std::path::PathBuf,
        before: Option<HookDef>,
        event: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = before.clone().unwrap_or_else(|| HookDef {
            id: uuid::Uuid::new_v4().to_string(),
            event,
            enabled: true,
            ..Default::default()
        });
        let name = cx.new(|cx| InputState::new(window, cx).default_value(draft.name.clone()));
        let description =
            cx.new(|cx| InputState::new(window, cx).default_value(draft.description.clone()));
        let url = cx.new(|cx| InputState::new(window, cx).default_value(draft.url.clone()));
        let timeout =
            cx.new(|cx| InputState::new(window, cx).default_value(draft.timeout_ms.to_string()));
        let script = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(5)
                .default_value(draft.script.clone())
        });
        let headers = cx.new(|cx| {
            TextareaState::new(window, cx).rows(3).default_value(
                draft
                    .headers
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        });
        Self {
            directory,
            before,
            baseline: draft.clone(),
            draft,
            name,
            description,
            url,
            timeout,
            script,
            headers,
            busy: false,
            error: None,
            task: None,
        }
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let mut after = self.draft.clone();
        after.name = self.name.read(cx).value().trim().into();
        after.description = self.description.read(cx).value().trim().into();
        after.script = self.script.read(cx).value().to_string();
        after.url = self.url.read(cx).value().trim().into();
        after.timeout_ms = match self.timeout.read(cx).value().parse() {
            Ok(value) => value,
            Err(_) => {
                self.error = Some("超时必须为 1000–600000 ms 的整数。".into());
                cx.notify();
                return;
            }
        };
        after.headers = self
            .headers
            .read(cx)
            .value()
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                let (key, value) = line.split_once(':').unwrap_or((line, ""));
                (!key.trim().is_empty()).then(|| (key.trim().to_owned(), value.trim().to_owned()))
            })
            .collect();
        if after.kind == "command" {
            after.url.clear();
            after.headers.clear();
            after.method = "POST".into();
        } else {
            after.script.clear();
        }
        let change = Change::Hook {
            before: self.before.clone(),
            after: Some(after),
        };
        let directory = self.directory.clone();
        let handle = window.window_handle();
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { native_preferences::save(&directory, change) })
                .await;
            let _ = cx.update_window(handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.busy = false;
                    match result {
                        Ok(_) => {
                            cx.emit(EditorEvent::Saved);
                            window.close_dialog(cx);
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }
    fn picker(
        &self,
        id: &'static str,
        current: &str,
        options: Vec<(&'static str, &'static str)>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let owner = cx.weak_entity();
        let label = options
            .iter()
            .find(|(key, _)| *key == current)
            .map(|(_, label)| *label)
            .unwrap_or(current)
            .to_owned();
        Button::new(id)
            .small()
            .label(crate::i18n::display(label))
            .dropdown_caret(true)
            .disabled(self.busy)
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in &options {
                    let owner = owner.clone();
                    let value = value.to_string();
                    menu = menu.item(PopupMenuItem::new(*label).on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            match id {
                                "hook-event" => this.draft.event = value.clone(),
                                "hook-kind" => this.draft.kind = value.clone(),
                                _ => this.draft.method = value.clone(),
                            }
                            cx.notify();
                        });
                    }));
                }
                menu
            })
    }
}
impl Render for HookEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |label: &'static str, input: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .w(px(88.))
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::i18n::display(label)),
                )
                .child(div().flex_1().min_w_0().child(input))
        };
        let mut body = div()
            .text_size(crate::typography::ui_rems(13.))
            .v_flex()
            .gap_3()
            .child(field(
                "名称",
                Input::new(&self.name)
                    .aria_label("Hook 名称")
                    .small()
                    .disabled(self.busy)
                    .into_any_element(),
            ))
            .child(field(
                "说明",
                Input::new(&self.description)
                    .aria_label("Hook 说明")
                    .small()
                    .disabled(self.busy)
                    .into_any_element(),
            ))
            .child(field(
                "触发事件",
                self.picker("hook-event", &self.draft.event, EVENTS.to_vec(), cx)
                    .into_any_element(),
            ))
            .child(field(
                "执行方式",
                self.picker(
                    "hook-kind",
                    &self.draft.kind,
                    vec![("command", "命令脚本"), ("http", "HTTP 请求")],
                    cx,
                )
                .into_any_element(),
            ));
        if self.draft.kind == "http" {
            body = body
                .child(field(
                    "URL",
                    Input::new(&self.url)
                        .aria_label("Hook URL")
                        .small()
                        .disabled(self.busy)
                        .into_any_element(),
                ))
                .child(field(
                    "方法",
                    self.picker(
                        "hook-method",
                        &self.draft.method,
                        vec![
                            ("POST", "POST"),
                            ("GET", "GET"),
                            ("PUT", "PUT"),
                            ("PATCH", "PATCH"),
                            ("DELETE", "DELETE"),
                        ],
                        cx,
                    )
                    .into_any_element(),
                ))
                .child(div().child(crate::i18n::text("请求头（每行 Name: Value）")))
                .child(
                    Textarea::new(&self.headers)
                        .aria_label("Hook 请求头")
                        .disabled(self.busy),
                );
        } else {
            body = body.child(div().child(crate::i18n::text("脚本"))).child(
                Textarea::new(&self.script)
                    .aria_label("Hook 脚本")
                    .disabled(self.busy),
            );
        }
        body = body.child(field(
            "超时 (ms)",
            Input::new(&self.timeout)
                .aria_label("Hook 超时毫秒")
                .small()
                .disabled(self.busy)
                .into_any_element(),
        ));
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        body.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("hook-cancel")
                        .small()
                        .label(crate::i18n::text("取消"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.busy {
                                cx.emit(EditorEvent::Closed);
                                window.close_dialog(cx);
                            }
                        })),
                )
                .child(
                    Button::new("hook-save")
                        .small()
                        .primary()
                        .label(if self.busy { crate::i18n::text("保存中…") } else { crate::i18n::text("保存") })
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }
}
