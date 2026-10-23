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
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    editor: Option<Entity<HookEditor>>,
    editor_events: Option<Subscription>,
}
impl HooksPage {
    pub fn new(directory: std::path::PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            directory,
            hooks: vec![],
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
                    match change {
                        Some(change) => native_preferences::save(&directory, change),
                        None => native_preferences::load(&directory),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(settings) => {
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
        self.editor_events =
            Some(cx.subscribe(&editor, |this, _, _: &Saved, cx| this.reload(None, cx)));
        self.editor = Some(editor.clone());
        window.open_dialog(cx, move |dialog, _, cx| {
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
                .child(editor.clone())
        });
    }
}
impl Render for HooksPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let add = Button::new("hook-add")
            .label("＋ 新建 Hook")
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
        let mut card = div().v_flex().gap_3().p_4().border_1().border_color(cx.theme().border).rounded_lg()
            .child(div().flex().items_center().justify_between().gap_3()
                .child(div().v_flex().gap_2().child(div().font_weight(FontWeight::MEDIUM).child("已配置的 Hook"))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("按触发顺序排列。失败只报警告，不会打断对话，也不能拒绝或改写工具调用。")))
                .child(add));
        if let Some(error) = &self.error {
            card = card.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        if self.hooks.is_empty() {
            card = card.child(
                div()
                    .v_flex()
                    .items_center()
                    .gap_3()
                    .py_10()
                    .child(if self.busy {
                        "正在读取…"
                    } else {
                        "还没有 Hook"
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "在对话开始、工具执行前后、对话结束等时机执行脚本或 HTTP 请求。",
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
                            .label("编辑")
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
                            .label("删除")
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(move |_, _, window, cx| {
                                let owner = cx.weak_entity();
                                let hook = delete_hook.clone();
                                window.open_dialog(cx, move |dialog, _, _| {
                                    let owner = owner.clone();
                                    let hook = hook.clone();
                                    dialog
                                        .title("删除 Hook")
                                        .child(format!(
                                            "删除“{}”？此操作会移除这条 Hook 配置。",
                                            hook.name
                                        ))
                                        .footer(
                                            div()
                                                .flex()
                                                .justify_end()
                                                .gap_2()
                                                .child(
                                                    Button::new("cancel-delete-hook")
                                                        .label("取消")
                                                        .on_click(|_, window, cx| {
                                                            window.close_dialog(cx)
                                                        }),
                                                )
                                                .child(
                                                    Button::new("confirm-delete-hook")
                                                        .label("删除")
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
                    .child(format!(
                        "共 {} 条，已启用 {} 条",
                        self.hooks.len(),
                        self.hooks.iter().filter(|hook| hook.enabled).count()
                    )),
            );
        }
        crate::kivio_settings::page_frame(
            "Hooks",
            "配置对话生命周期的 8 个事件。原生对话的事件派发尚未接入；当前保存配置不会执行脚本。",
            card.into_any_element(),
            cx,
        )
    }
}
struct Saved;
struct HookEditor {
    directory: std::path::PathBuf,
    before: Option<HookDef>,
    draft: HookDef,
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
impl EventEmitter<Saved> for HookEditor {}
impl HookEditor {
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
                            cx.emit(Saved);
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
            .label(label)
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
                .child(div().w(px(88.)).child(label))
                .child(div().flex_1().min_w_0().child(input))
        };
        let mut body = div()
            .text_size(px(14.))
            .v_flex()
            .gap_3()
            .child(field(
                "名称",
                Input::new(&self.name)
                    .disabled(self.busy)
                    .into_any_element(),
            ))
            .child(field(
                "说明",
                Input::new(&self.description)
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
                    Input::new(&self.url).disabled(self.busy).into_any_element(),
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
                .child(div().child("请求头（每行 Name: Value）"))
                .child(Textarea::new(&self.headers).disabled(self.busy));
        } else {
            body = body
                .child(div().child("脚本"))
                .child(Textarea::new(&self.script).disabled(self.busy));
        }
        body = body.child(field(
            "超时 (ms)",
            Input::new(&self.timeout)
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
                        .label("取消")
                        .disabled(self.busy)
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("hook-save")
                        .primary()
                        .label(if self.busy { "保存中…" } else { "保存" })
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }
}
