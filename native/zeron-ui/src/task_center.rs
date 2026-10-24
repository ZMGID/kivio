//! Retained task-center presentation; ScheduledTasks remains the business-state owner.
use crate::{icons, settings::widgets, theme::Theme};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, WindowExt,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    automation_catalog::{self, types::Automation},
    scheduled_tasks::{self, ScheduledTasks, types::*},
};
use std::{path::PathBuf, sync::Arc};
mod form;

pub struct OpenTaskConversation(pub String);
pub struct TaskCenter {
    directory: PathBuf,
    state: Entity<crate::state::AppState>,
    service: Option<Arc<crate::kivio::KivioService>>,
    owner: Option<Arc<ScheduledTasks>>,
    snapshot: ScheduledTasksSnapshot,
    conversations: Vec<(String, String)>,
    automations: Vec<Automation>,
    automation_warnings: Vec<String>,
    automations_tab: bool,
    search: Entity<InputState>,
    page: usize,
    loading: bool,
    busy: bool,
    error: Option<String>,
    editor: Option<Entity<TaskEditor>>,
    editor_observation: Option<Subscription>,
    editing: bool,
    history: Option<String>,
    runs: Vec<TaskRun>,
    operation: Option<Task<()>>,
    read: Option<Task<()>>,
    subscription: Option<Task<()>>,
    _search: Subscription,
    tab_focus: [FocusHandle; 2],
}
impl EventEmitter<OpenTaskConversation> for TaskCenter {}
impl TaskCenter {
    pub fn new(
        directory: PathBuf,
        state: Entity<crate::state::AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索任务"));
        let subscription = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.page = 0;
                cx.notify();
            }
        });
        let mut this = Self {
            directory,
            state,
            service: None,
            owner: None,
            snapshot: ScheduledTasksSnapshot {
                tasks: vec![],
                live_runs: vec![],
            },
            conversations: vec![],
            automations: vec![],
            automation_warnings: vec![],
            automations_tab: crate::settings::current(cx).tasks_tab == "automations",
            search,
            page: 0,
            loading: false,
            busy: false,
            error: None,
            editor: None,
            editor_observation: None,
            editing: false,
            history: None,
            runs: vec![],
            operation: None,
            read: None,
            subscription: None,
            _search: subscription,
            tab_focus: [
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
            ],
        };
        this.refresh(cx);
        this
    }
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }
    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| editor.read(cx).is_dirty(cx))
    }
    pub fn on_visible(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.owner.is_none() {
            self.service = self.state.read(cx).engine().and_then(|e| e.kivio_service());
            let result = self
                .service
                .as_ref()
                .ok_or_else(|| "对话服务尚未连接，请稍后重试。".to_owned())
                .and_then(|s| s.scheduled_tasks());
            match result {
                Ok(owner) => {
                    let mut changes = owner.subscribe();
                    self.subscription = Some(cx.spawn(async move |this, cx| {
                        while changes.changed().await.is_ok() {
                            if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                                break;
                            }
                        }
                    }));
                    self.owner = Some(owner);
                }
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
        }
        let owner = self.owner.as_ref().unwrap().clone();
        let directory = self.directory.clone();
        let history = self.history.clone();
        self.loading = true;
        self.read = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let snapshot = owner.snapshot();
                    let conversations =
                        kivio_model_runtime::conversation_files::ConversationStore::new(
                            directory.join("conversations"),
                        )
                        .load_all()
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|c| (c.id, c.title))
                        .collect();
                    let (automations, warnings) =
                        automation_catalog::list_in(&directory.join("automations"))?;
                    let runs = history
                        .map(|id| owner.runs(&id))
                        .transpose()?
                        .unwrap_or_default();
                    Ok::<_, String>((snapshot, conversations, automations, warnings, runs))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((snapshot, conversations, automations, warnings, runs)) => {
                        this.snapshot = snapshot;
                        this.conversations = conversations;
                        this.automations = automations;
                        this.automation_warnings = warnings;
                        this.runs = runs;
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(error.clone());
                        crate::toast::show(cx, crate::toast::ToastKind::Error, error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn mutate(
        &mut self,
        action: impl FnOnce(&crate::kivio::KivioService) -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(service) = self.service.clone() else {
            self.refresh(cx);
            return;
        };
        self.busy = true;
        let operation = gpui_tokio::Tokio::spawn(cx, async move { action(&service) });
        self.operation = Some(cx.spawn(async move |this, cx| {
            let result = operation.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(error) = result {
                    crate::toast::show(cx, crate::toast::ToastKind::Error, error);
                }
                this.refresh(cx);
            });
        }));
        cx.notify();
    }
    fn edit(&mut self, task: Option<ScheduledTask>, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.editor.is_none() {
            let editor = cx.new(|cx| TaskEditor::new(task, window, cx));
            self.editor_observation = Some(cx.observe(&editor, |_, _, cx| cx.notify()));
            self.editor = Some(editor);
        }
        self.editing = true;
        cx.notify();
    }
    fn delete(&mut self, task: ScheduledTask, window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let id = task.id.clone();
            dialog
                .title(format!("删除“{}”？", task.name))
                .child("删除定时任务及其运行记录。绑定对话和已经开始的回复会保留。")
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-delete-task")
                                .label("取消")
                                .on_click(|_, w, cx| w.close_dialog(cx)),
                        )
                        .child(Button::new("delete-task").danger().label("删除").on_click(
                            move |_, w, cx| {
                                let id = id.clone();
                                let _ = owner.update(cx, |this, cx| {
                                    this.mutate(move |s| s.scheduled_tasks()?.delete(&id), cx)
                                });
                                w.close_dialog(cx);
                            },
                        )),
                )
        });
    }
    fn render_row(&self, task: &ScheduledTask, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id.clone();
        let run_id = id.clone();
        let history_id = id.clone();
        let open_id = task.conversation_id.clone();
        let edit = task.clone();
        let delete = task.clone();
        let enabled = task.enabled;
        let live = self
            .snapshot
            .live_runs
            .iter()
            .filter(|r| r.task_id == task.id)
            .map(|r| form::run_status(r.status))
            .collect::<Vec<_>>()
            .join("、");
        let target = self
            .conversations
            .iter()
            .find(|(id, _)| id == &task.conversation_id)
            .map(|(_, t)| t.as_str())
            .unwrap_or("绑定对话已不可读取");
        div()
            .id(SharedString::from(format!("task-{}", task.id)))
            .flex()
            .flex_col()
            .gap_2()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(task.name.clone()))
                    .child(if !live.is_empty() {
                        live
                    } else if task.status == TaskStatus::Completed {
                        "已完成".into()
                    } else if enabled {
                        "已启用".into()
                    } else {
                        "已暂停".into()
                    })
                    .child(
                        Switch::new(SharedString::from(format!("enable-{}", id)))
                            .label("启用")
                            .checked(enabled)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, value, _, cx| {
                                let id = id.clone();
                                let enabled = *value;
                                this.mutate(
                                    move |s| {
                                        s.scheduled_tasks()?
                                            .set_enabled(&id, enabled, scheduled_tasks::now_secs())
                                            .map(|_| ())
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} · {} · 下次：{}",
                        form::summary(&task.schedule),
                        target,
                        form::time_label(task.next_run_at)
                    )),
            )
            .when_some(task.last_error.clone(), |v, error| {
                v.child(div().text_sm().child(error))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("run-{}", run_id)))
                            .small()
                            .label("立即运行")
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let id = run_id.clone();
                                this.mutate(move |s| s.run_scheduled(&id).map(|_| ()), cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("edit-{}", task.id)))
                            .small()
                            .label("编辑…")
                            .disabled(self.busy || self.editor.is_some())
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.edit(Some(edit.clone()), w, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("history-{}", task.id)))
                            .small()
                            .ghost()
                            .label("运行记录")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.history = Some(history_id.clone());
                                this.refresh(cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("chat-{}", task.id)))
                            .small()
                            .ghost()
                            .label("打开对话")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(OpenTaskConversation(open_id.clone()))
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{}", task.id)))
                            .small()
                            .ghost()
                            .label("删除…")
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.delete(delete.clone(), w, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}
impl TaskCenter {
    fn select_tab(&mut self, automation: bool, cx: &mut Context<Self>) {
        self.automations_tab = automation;
        self.page = 0;
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, move |s| {
            s.tasks_tab = if automation {
                "automations"
            } else {
                "schedules"
            }
            .to_owned();
        });
        cx.notify();
    }

    fn render_tabs(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut tabs = div().flex().items_center().gap_1();
        for (ix, (id, label, glyph, count)) in [
            (
                "task-tab-schedules",
                "定时任务",
                icons::CLOCK_CIRCLE,
                self.snapshot.tasks.len(),
            ),
            (
                "task-tab-automations",
                "自动化",
                icons::WORKTREE,
                self.automations.len(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = self.automations_tab == (ix == 1);
            let t = widgets::tab_selection_t(
                window,
                format!("{id}-selection"),
                selected,
                crate::motion::reduced_motion(cx),
            );
            tabs = tabs.child(
                widgets::section_tab(theme, selected, t, id, format!("{id}-hover"))
                    .role(Role::Tab)
                    .aria_label(label)
                    .aria_selected(selected)
                    .track_focus(&self.tab_focus[ix].clone().tab_stop(selected))
                    .focus_visible(|el| el.border_2().border_color(theme.accent))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_tab(ix == 1, cx)))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let next = match event.keystroke.key.as_str() {
                            "left" | "right" => 1 - ix,
                            "home" => 0,
                            "end" => 1,
                            "enter" | "space" => ix,
                            _ => return,
                        };
                        this.select_tab(next == 1, cx);
                        window.focus(&this.tab_focus[next], cx);
                        cx.stop_propagation();
                    }))
                    .child(icons::icon(glyph).size_4().text_color(theme.text_muted))
                    .child(label)
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(count.to_string()),
                    ),
            );
        }
        tabs
    }
}

impl Render for TaskCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let mut content = div().flex().flex_col().gap_3();
        if self.editing {
            if let Some(editor) = self.editor.clone() {
                return self.render_editor(editor, window, cx);
            }
        }
        if self.automations_tab {
            content = content.child(div().text_sm().text_color(cx.theme().muted_foreground).child("自动化工作流与定时任务独立。当前可查看既有工作流；原生画布、编辑和运行尚未接入，不会执行已有触发器。"));
            for item in &self.automations {
                content = content.child(
                    div()
                        .py_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(item.name.clone())
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} 个节点 · {} 条连线 · {}",
                                    item.nodes.len(),
                                    item.edges.len(),
                                    if item.enabled {
                                        "原配置已启用（本客户端未执行）"
                                    } else {
                                        "草稿"
                                    }
                                )),
                        ),
                );
            }
            if self.automations.is_empty() {
                content = content.child(
                    widgets::section_card(&theme).mt_0().child(
                        div()
                            .py_12()
                            .px_6()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                icons::icon(icons::WORKTREE)
                                    .size_6()
                                    .text_color(theme.text_muted),
                            )
                            .child(widgets::row_title(&theme, "没有自动化工作流"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.text_muted)
                                    .child("原生编辑与执行尚未接入"),
                            ),
                    ),
                );
            }
            for warning in &self.automation_warnings {
                content = content.child(div().text_sm().child(warning.clone()));
            }
        } else if let Some(history) = self.history.clone() {
            let title = self
                .snapshot
                .tasks
                .iter()
                .find(|t| t.id == history)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "任务已删除".into());
            content = content
                .child(
                    Button::new("close-task-history")
                        .label("返回任务列表")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.history = None;
                            cx.notify();
                        })),
                )
                .child(title);
            if self.runs.is_empty() {
                content = content.child("暂无运行记录");
            }
            for run in &self.runs {
                let task = run.task_id.clone();
                let id = run.id.clone();
                content = content.child(
                    div()
                        .py_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(format!(
                            "{} · {} · {}",
                            form::run_status(run.status),
                            if run.trigger == RunTrigger::Manual {
                                "手动"
                            } else {
                                "计划"
                            },
                            form::time_label(Some(run.created_at))
                        ))
                        .when_some(run.error.clone(), |v, e| v.child(div().text_sm().child(e)))
                        .child(
                            Button::new(SharedString::from(format!("delete-run-{}", run.id)))
                                .small()
                                .ghost()
                                .label("删除记录")
                                .disabled(self.busy || !run.status.is_terminal())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let task = task.clone();
                                    let id = id.clone();
                                    this.mutate(
                                        move |s| s.scheduled_tasks()?.delete_run(&task, &id),
                                        cx,
                                    );
                                })),
                        ),
                );
            }
        } else {
            let needle = self.search.read(cx).value().to_lowercase();
            let tasks: Vec<_> = self
                .snapshot
                .tasks
                .iter()
                .filter(|t| {
                    format!("{} {}", t.name, t.prompt)
                        .to_lowercase()
                        .contains(&needle)
                })
                .cloned()
                .collect();
            let pages = tasks.len().div_ceil(40).max(1);
            let page = self.page.min(pages - 1);
            if tasks.is_empty() && !self.loading {
                let searching = !needle.trim().is_empty();
                content = content.child(
                    widgets::section_card(&theme).mt_0().child(
                        div()
                            .py_12()
                            .px_6()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                icons::icon(icons::CLOCK_CIRCLE)
                                    .size_6()
                                    .text_color(theme.text_muted),
                            )
                            .child(widgets::row_title(
                                &theme,
                                if searching {
                                    "没有匹配的任务"
                                } else {
                                    "还没有定时任务"
                                },
                            ))
                            .child(div().text_sm().text_color(theme.text_muted).child(
                                if searching {
                                    "换个关键词试试。"
                                } else {
                                    "新建任务，让提示词按计划发送到指定对话。"
                                },
                            )),
                    ),
                );
            }
            for task in tasks.iter().skip(page * 40).take(40) {
                content = content.child(self.render_row(task, cx));
            }
            if pages > 1 {
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("tasks-prev")
                                .small()
                                .label("上一页")
                                .disabled(page == 0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.page = page.saturating_sub(1);
                                    cx.notify();
                                })),
                        )
                        .child(format!("{} / {}", page + 1, pages))
                        .child(
                            Button::new("tasks-next")
                                .small()
                                .label("下一页")
                                .disabled(page + 1 >= pages)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.page = page + 1;
                                    cx.notify();
                                })),
                        ),
                );
            }
        }
        if let Some(error) = &self.error {
            content = content.child(div().text_sm().child(error.clone()));
        }
        if self.loading {
            content = content.child("正在读取任务…");
        }
        // The task outlet already reserves the platform titlebar above us.
        let tabs = self.render_tabs(&theme, window, cx);
        let page = widgets::page_column()
            .pt_4()
            .child(widgets::page_header(&theme, "任务", None))
            .child(widgets::page_subtitle(
                &theme,
                "安排定时对话，查看自动化工作流。",
            ))
            .child(
                div()
                    .mt_6()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(tabs)
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-task-center")
                            .small()
                            .ghost()
                            .label("刷新")
                            .disabled(self.loading || self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .when(!self.automations_tab, |v| {
                        v.child(
                            Button::new("create-scheduled-task")
                                .custom(
                                    ButtonCustomVariant::new(cx)
                                        .color(widgets::select_fill(&theme, false))
                                        .foreground(theme.text)
                                        .hover(widgets::select_fill(&theme, true)),
                                )
                                .label(if self.editor.is_some() {
                                    "继续编辑…"
                                } else {
                                    "新建任务…"
                                })
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, w, cx| this.edit(None, w, cx))),
                        )
                    }),
            )
            .when(!self.automations_tab && self.history.is_none(), |v| {
                v.child(
                    div()
                        .mt_4()
                        .w_full()
                        .rounded_lg()
                        .bg(widgets::select_fill(&theme, false))
                        .child(
                            Input::new(&self.search).appearance(false).prefix(
                                icons::icon(icons::MAGNIFER)
                                    .size_4()
                                    .text_color(theme.text_muted),
                            ),
                        ),
                )
            })
            .child(div().mt_3().child(content));
        widgets::scroll_faded(
            "task-center-scroll",
            div()
                .id("task-center-scroll-area")
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .text_color(theme.text)
                .child(page),
        )
        .into_any_element()
    }
}

struct TaskEditor {
    original: Option<ScheduledTask>,
    name: Entity<InputState>,
    prompt: Entity<TextareaState>,
    fields: Vec<Entity<InputState>>,
    kind: usize,
    target: Option<String>,
    enabled: bool,
    error: Option<String>,
    preview: Vec<i64>,
}
impl TaskEditor {
    fn new(task: Option<ScheduledTask>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (kind, values) = form::fields(task.as_ref().map(|t| &t.schedule));
        Self {
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(task.as_ref().map(|t| t.name.clone()).unwrap_or_default())
            }),
            prompt: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(5)
                    .default_value(task.as_ref().map(|t| t.prompt.clone()).unwrap_or_default())
            }),
            fields: values
                .into_iter()
                .map(|v| cx.new(|cx| InputState::new(window, cx).default_value(v)))
                .collect(),
            kind,
            target: task.as_ref().map(|t| t.conversation_id.clone()),
            enabled: task.as_ref().is_none_or(|t| t.enabled),
            original: task,
            error: None,
            preview: vec![],
        }
    }
    fn request(&self, cx: &App) -> Result<ScheduledTaskInput, String> {
        let values = self
            .fields
            .iter()
            .map(|f| f.read(cx).value().to_string())
            .collect::<Vec<_>>();
        Ok(ScheduledTaskInput {
            id: self.original.as_ref().map(|t| t.id.clone()),
            name: self.name.read(cx).value().to_string(),
            prompt: self.prompt.read(cx).value().to_string(),
            schedule: form::parse(
                self.kind,
                &values,
                self.original.as_ref().map(|t| &t.schedule),
            )?,
            target: self
                .target
                .clone()
                .map(|conversation_id| TaskTarget::Conversation { conversation_id })
                .unwrap_or(TaskTarget::NewConversation {
                    provider_id: None,
                    model: None,
                    project_id: None,
                    thinking_level: None,
                }),
            enabled: Some(self.enabled),
        })
    }
    fn is_dirty(&self, cx: &App) -> bool {
        let Some(original) = &self.original else {
            return !self.name.read(cx).value().is_empty()
                || !self.prompt.read(cx).value().is_empty();
        };
        self.request(cx)
            .map(|r| {
                r.name != original.name
                    || r.prompt != original.prompt
                    || r.schedule != original.schedule
                    || r.enabled != Some(original.enabled)
                    || r.target
                        != TaskTarget::Conversation {
                            conversation_id: original.conversation_id.clone(),
                        }
            })
            .unwrap_or(true)
    }
}

impl TaskCenter {
    fn render_editor(
        &mut self,
        editor: Entity<TaskEditor>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let e = editor.read(cx);
        let weak = editor.downgrade();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xl().child(if e.original.is_some() {
                "编辑定时任务"
            } else {
                "新建定时任务"
            }))
            .child("任务名称")
            .child(Input::new(&e.name).disabled(self.busy))
            .child("提示词")
            .child(Textarea::new(&e.prompt).disabled(self.busy));
        let kind = e.kind;
        let target = e.target.clone();
        let choices = self.conversations.clone();
        body = body.child("发送到").child(
            Button::new("task-target")
                .dropdown_caret(true)
                .label(
                    target
                        .as_ref()
                        .map(|id| {
                            self.conversations
                                .iter()
                                .find(|(c, _)| c == id)
                                .map(|(_, t)| t.clone())
                                .unwrap_or_else(|| format!("对话 {id}"))
                        })
                        .unwrap_or_else(|| "新建对话（默认运行时与模型）".into()),
                )
                .disabled(self.busy)
                .dropdown_menu(move |mut menu, _, _| {
                    let editor = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new("新建对话（默认运行时与模型）")
                            .checked(target.is_none())
                            .on_click(move |_, _, cx| {
                                let _ = editor.update(cx, |e, cx| {
                                    e.target = None;
                                    cx.notify();
                                });
                            }),
                    );
                    for (id, title) in &choices {
                        let id = id.clone();
                        let checked = target.as_ref() == Some(&id);
                        let editor = weak.clone();
                        menu =
                            menu.item(PopupMenuItem::new(title.clone()).checked(checked).on_click(
                                move |_, _, cx| {
                                    let _ = editor.update(cx, |e, cx| {
                                        e.target = Some(id.clone());
                                        cx.notify();
                                    });
                                },
                            ));
                    }
                    menu
                }),
        );
        let weak = editor.downgrade();
        body = body.child("计划").child(
            Button::new("task-schedule-kind")
                .dropdown_caret(true)
                .label(form::KINDS[kind])
                .disabled(self.busy)
                .dropdown_menu(move |mut menu, _, _| {
                    for (ix, label) in form::KINDS.iter().enumerate() {
                        let editor = weak.clone();
                        menu = menu.item(PopupMenuItem::new(*label).checked(ix == kind).on_click(
                            move |_, _, cx| {
                                let _ = editor.update(cx, |e, cx| {
                                    e.kind = ix;
                                    e.preview.clear();
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                }),
        );
        for (ix, label) in form::labels(kind) {
            body = body
                .child(label)
                .child(Input::new(&e.fields[ix]).disabled(self.busy));
        }
        body = body.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("使用本机时区；每次运行发送到同一条绑定对话。编辑保留间隔锚点。"),
        );
        let weak = editor.downgrade();
        body = body.child(
            Switch::new("editor-task-enabled")
                .label("启用")
                .checked(e.enabled)
                .disabled(self.busy)
                .on_click(move |enabled, _, cx| {
                    let _ = weak.update(cx, |e, cx| {
                        e.enabled = *enabled;
                        cx.notify();
                    });
                }),
        );
        if let Some(error) = &e.error {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        for at in &e.preview {
            body = body.child(form::time_label(Some(*at)));
        }
        let weak = editor.downgrade();
        let preview = Button::new("preview-task-schedule")
            .label("预览下次时间")
            .disabled(self.busy)
            .on_click(move |_, _, cx| {
                let _ = weak.update(cx, |e, cx| {
                    match e
                        .request(cx)
                        .and_then(|r| scheduled_tasks::preview(&r.schedule))
                    {
                        Ok(times) => {
                            e.preview = times;
                            e.error = None;
                        }
                        Err(err) => e.error = Some(err),
                    }
                    cx.notify();
                });
            });
        let save_editor = editor.clone();
        let save = Button::new("save-scheduled-task")
            .primary()
            .label(if self.busy {
                "正在保存…"
            } else {
                "保存"
            })
            .disabled(self.busy)
            .on_click(cx.listener(move |this, _, _, cx| {
                let request = save_editor.read(cx).request(cx).and_then(|r| {
                    scheduled_tasks::validated_name(&r)?;
                    Ok(r)
                });
                let input = match request {
                    Ok(r) => r,
                    Err(err) => {
                        save_editor.update(cx, |e, cx| {
                            e.error = Some(err);
                            cx.notify();
                        });
                        return;
                    }
                };
                let Some(service) = this.service.clone() else {
                    return;
                };
                this.busy = true;
                let editor = save_editor.clone();
                let operation =
                    gpui_tokio::Tokio::spawn(cx, async move { service.save_scheduled(input) });
                this.operation = Some(cx.spawn(async move |this, cx| {
                    let result = operation.await.map_err(|e| e.to_string()).and_then(|r| r);
                    let _ = this.update(cx, |this, cx| {
                        this.busy = false;
                        match result {
                            Ok(_) => {
                                this.editor = None;
                                this.editor_observation = None;
                                this.editing = false;
                                this.refresh(cx);
                            }
                            Err(err) => {
                                editor.update(cx, |e, cx| {
                                    e.error = Some(err);
                                    cx.notify();
                                });
                            }
                        }
                        cx.notify();
                    });
                }));
                cx.notify();
            }));
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(preview)
                .child(save)
                .child(
                    Button::new("back-task-list")
                        .label("返回列表")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editing = false;
                            if !this.has_unsaved_changes(cx) {
                                this.editor = None;
                                this.editor_observation = None;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("discard-task-draft")
                        .label("放弃修改…")
                        .disabled(self.busy)
                        .on_click(cx.listener(|_, _, window, cx| {
                            let owner = cx.weak_entity();
                            window.open_dialog(cx, move |dialog, _, _| {
                                let owner = owner.clone();
                                dialog
                                    .title("放弃任务修改？")
                                    .child("未保存的任务草稿将被丢弃，已保存的任务不变。")
                                    .footer(
                                        div()
                                            .flex()
                                            .justify_end()
                                            .gap_2()
                                            .child(
                                                Button::new("keep-task-draft")
                                                    .label("继续编辑")
                                                    .on_click(|_, w, cx| w.close_dialog(cx)),
                                            )
                                            .child(
                                                Button::new("confirm-discard-task")
                                                    .danger()
                                                    .label("放弃修改")
                                                    .on_click(move |_, w, cx| {
                                                        let _ = owner.update(cx, |this, cx| {
                                                            this.editor = None;
                                                            this.editing = false;
                                                            this.editor_observation = None;
                                                            cx.notify();
                                                        });
                                                        w.close_dialog(cx);
                                                    }),
                                            ),
                                    )
                            });
                        })),
                ),
        );
        div()
            .size_full()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(div().w_full().max_w(rems(52.)).mx_auto().p_4().child(body))
            .into_any_element()
    }
}
