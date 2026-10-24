//! Screenshot questions retain their image and drafts for the window lifetime.
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::Button,
    input::{InputEvent, Textarea, TextareaState},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::settings::{ExplainMessage, LensConfig};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

actions!(native_lens, [CloseLens]);
struct ActiveLens {
    handle: AnyWindowHandle,
    view: WeakEntity<LensView>,
}
impl Global for ActiveLens {}
pub(crate) fn close_active(handle: AnyWindowHandle, cx: &mut App) -> bool {
    if cx
        .try_global::<ActiveLens>()
        .is_none_or(|active| active.handle.window_id() != handle.window_id())
    {
        return false;
    }
    toggle_existing(cx)
}
pub(crate) fn blocks_exit(cx: &mut App) -> bool {
    let Some(active) = cx.try_global::<ActiveLens>() else {
        return false;
    };
    let (handle, view) = (active.handle, active.view.clone());
    let blocked = view.upgrade().is_some_and(|view| view.read(cx).committing);
    if blocked {
        let _ = cx.update_window(handle, |_, window, _| window.activate_window());
    }
    blocked
}
pub(crate) fn toggle_existing(cx: &mut App) -> bool {
    let Some(active) = cx.try_global::<ActiveLens>() else {
        return false;
    };
    let (handle, view) = (active.handle, active.view.clone());
    let Some(view) = view.upgrade() else {
        return false;
    };
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.close_or_stop(window, cx))
    })
    .is_ok()
}
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseLens, Some("NativeLens"))]);
}
pub(crate) fn open(
    root: PathBuf,
    path: tempfile::TempPath,
    bytes: Vec<u8>,
    config: LensConfig,
    display: DisplayId,
    cx: &mut App,
) -> Result<(), String> {
    let session = uuid::Uuid::new_v4();
    let scope = format!("lens-{session}");
    let surface = session.as_u128() as u64;
    let preview = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
    let bounds = Bounds::centered(Some(display), size(px(640.), px(720.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: Some(display),
            window_min_size: Some(size(px(420.), px(420.))),
            kind: WindowKind::Floating,
            titlebar: Some(TitlebarOptions {
                title: Some("Kivio · Lens".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| {
                let question = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .rows(3)
                        .submit_on_enter(true)
                        .placeholder(crate::i18n::text(
                            "针对截图提问，Enter 发送，Shift+Enter 换行",
                        ))
                });
                cx.subscribe_in(
                    &question,
                    window,
                    |this: &mut LensView, _, event, window, cx| {
                        if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                            this.submit(window, cx);
                        }
                    },
                )
                .detach();
                let image = preview.clone();
                let released_scope = scope.clone();
                cx.on_release(move |_, cx| {
                    ImageSource::Image(image).remove_asset(cx);
                    crate::markdown::render::clear_selection_surface(&released_scope);
                })
                .detach();
                LensView {
                    root,
                    path,
                    preview,
                    question,
                    config,
                    scope,
                    surface,
                    handoff_id: format!("conv_{session}"),
                    handoff_draft: None,
                    committing: false,
                    history: vec![],
                    answers: vec![],
                    output: crate::markdown::IncrementalParser::new(),
                    reasoning: crate::markdown::IncrementalParser::new(),
                    reasoning_open: false,
                    web_search: false,
                    busy: false,
                    error: None,
                    cancel: None,
                    task: None,
                }
            });
            let weak = view.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                weak.upgrade().is_none_or(|view| !view.read(cx).committing)
            });
            cx.set_global(ActiveLens {
                handle: window.window_handle(),
                view: view.downgrade(),
            });
            cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
        },
    )
    .map_err(|error| error.to_string())?;
    cx.activate(true);
    Ok(())
}
struct Answer {
    question: String,
    output: crate::markdown::IncrementalParser,
}
#[derive(Default)]
struct Pending {
    text: String,
    reasoning: String,
    bytes: usize,
}
struct LensView {
    scope: String,
    surface: u64,
    handoff_id: String,
    handoff_draft: Option<(String, usize)>,
    committing: bool,
    root: PathBuf,
    path: tempfile::TempPath,
    preview: Arc<Image>,
    question: Entity<TextareaState>,
    config: LensConfig,
    history: Vec<ExplainMessage>,
    answers: Vec<Answer>,
    output: crate::markdown::IncrementalParser,
    reasoning: crate::markdown::IncrementalParser,
    reasoning_open: bool,
    web_search: bool,
    busy: bool,
    error: Option<String>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<Task<()>>,
}
impl Drop for LensView {
    fn drop(&mut self) {
        self.cancel.take();
    }
}
impl LensView {
    fn close_or_stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.committing {
            return;
        }
        if self.busy {
            if let Some(cancel) = self.cancel.take() {
                let _ = cancel.send(());
            }
        } else {
            window.remove_window();
        }
        cx.notify();
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit_mode(false, window, cx);
    }
    fn submit_mode(&mut self, transfer: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let question = if transfer {
            String::new()
        } else {
            self.question.read(cx).value().trim().to_string()
        };
        if question.is_empty() && !(transfer && !self.history.is_empty()) {
            self.error = Some("请先输入问题".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.committing = true;
        self.error = None;
        self.output.reset("");
        self.reasoning.reset("");
        let handle = window.window_handle();
        let root = self.root.clone();
        let image = self.path.to_path_buf();
        let previous_history = self.history.clone();
        let mut history = previous_history.clone();
        if !question.is_empty() {
            history.push(ExplainMessage {
                role: "user".into(),
                content: question.clone(),
            });
        }
        let search = self.web_search;
        let draft_key = (question.clone(), self.history.len());
        if self
            .handoff_draft
            .as_ref()
            .is_some_and(|previous| previous != &draft_key)
        {
            self.handoff_id = format!("conv_{}", uuid::Uuid::new_v4());
        }
        self.handoff_draft = Some(draft_key);
        let handoff_id = self.handoff_id.clone();
        let engine = crate::shell_windows(cx)
            .first()
            .and_then(|(_, shell)| shell.read(cx).lens_engine(cx));
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        self.cancel = Some(cancel);
        self.task = Some(cx.spawn(async move |this, cx| {
            let settings_root = root.clone();
            let settings = cx.background_spawn(async move { kivio_model_runtime::native_preferences::load(&settings_root) }).await;
            let settings = match settings {
                Ok(settings) => settings,
                Err(error) => { let _ = this.update(cx, |this, cx| { this.busy = false; this.committing = false; this.cancel = None; this.error = Some(error); cx.notify(); }); return; }
            };
            let committing = transfer || settings.lens.send_to_chat;
            if this.update(cx, |this, cx| { this.config = settings.lens.clone(); this.committing = committing; cx.notify(); }).is_err() { return; }
            let (sender, receiver) = async_channel::bounded::<()>(1);
            let pending = Arc::new(Mutex::new(Pending::default()));
            let worker_pending = pending.clone();
            let worker_history = history.clone();
            let worker_question = question.clone();
            let work = cx.background_spawn(async move {
                let config = settings.lens.clone();
                if !config.enabled { return Err("Lens 已在设置中关闭".into()); }
                crate::chat_runtime::runtime()?.block_on(async move {
                    let mut sink = |part| {
                        use kivio_model_runtime::{StreamPart, ModelError};
                        let mut pending = worker_pending.lock().map_err(|_| ModelError::new("Lens stream unavailable"))?;
                        let (reasoning, delta) = match part {
                            StreamPart::TextDelta { delta } => (false, delta),
                            StreamPart::ReasoningDelta { delta } => (true, delta), _ => return Ok(()),
                        };
                        pending.bytes = pending.bytes.saturating_add(delta.len());
                        if pending.bytes > 16 * 1024 * 1024 { return Err(ModelError::new("Lens response exceeds 16MB")); }
                        if reasoning { pending.reasoning.push_str(&delta); } else { pending.text.push_str(&delta); }
                        let _ = sender.try_send(());
                        Ok(())
                    };
                    let request = async {
                        if committing {
                            let engine = engine.ok_or("聊天服务尚未连接，请稍后重试。")?;
                            let chat = crate::kivio::lens::handoff(&engine, &handoff_id, &worker_question, &previous_history, &image).await?;
                            Ok((config, None, Some(chat)))
                        } else {
                            let answer = kivio_model_runtime::native_lens::ask_with_settings(&root, &settings, Some(&image), &worker_history, search, &mut sink).await?;
                            Ok((config, Some(answer), None))
                        }
                    };
                    if committing { request.await } else { tokio::select! { _ = cancelled => Err("已取消".into()), result = request => result } }
                })
            });
            while receiver.recv().await.is_ok() {
                let (text, reasoning) = match pending.lock() { Ok(mut pending) => (std::mem::take(&mut pending.text), std::mem::take(&mut pending.reasoning)), Err(_) => break };
                if this.update(cx, |this, cx| { this.output.append(&text); this.reasoning.append(&reasoning); cx.notify(); }).is_err() { return; }
            }
            let result = work.await;
            let _ = cx.update_window(handle, |_, window, cx| this.update(cx, |this, cx| {
                this.busy = false; this.committing = false; this.cancel = None;
                match result {
                    Ok((config, answer, chat)) => {
                        this.config = config;
                        if let Some(answer) = answer {
                            history.push(ExplainMessage { role: "assistant".into(), content: answer.clone() });
                            this.history = history;
                            let mut parsed = crate::markdown::IncrementalParser::new(); parsed.reset(&answer);
                            this.answers.push(Answer { question: question.clone(), output: parsed });
                            this.output.reset("");
                        }
                        if let Some(chat) = chat {
                            crate::activate_main_window(cx);
                            if let Some((handle, shell)) = crate::shell_windows(cx).first().cloned() {
                                let _ = cx.update_window(handle.into(), |_, window, cx| { shell.update(cx, |shell, cx| shell.open_lens_chat(chat, cx)); window.activate_window(); });
                            }
                            window.remove_window();
                        }
                        if this.question.read(cx).value().trim() == question {
                            this.question.update(cx, |input, cx| input.set_value("", window, cx));
                        }

                    }
                    Err(error) => {
                        if error == "已取消" && !this.output.source().trim().is_empty() {
                            let partial = this.output.source().to_string();
                            history.push(ExplainMessage { role: "assistant".into(), content: partial.clone() });
                            this.history = history;
                            let mut output = crate::markdown::IncrementalParser::new(); output.reset(&partial);
                            this.answers.push(Answer { question: question.clone(), output }); this.output.reset("");
                            if this.question.read(cx).value().trim() == question { this.question.update(cx, |input, cx| input.set_value("", window, cx)); }
                        }
                        this.error = Some(error);
                    }
                }
                cx.notify();
            }));
        }));
        cx.notify();
    }
}
impl Render for LensView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::Theme::of(cx).clone();
        let render = |id: String, parser: &crate::markdown::IncrementalParser| {
            crate::markdown::render::render_tree(
                &parser.display_tree(),
                &crate::markdown::render::RenderOptions::settled(
                    format!("{}-{id}", self.scope).into(),
                ),
                &theme,
                window,
                &|_| None,
            )
        };
        let mut history = div().v_flex().gap_4();
        let indices: Vec<_> = if self.config.message_order == "desc" {
            (0..self.answers.len()).rev().collect()
        } else {
            (0..self.answers.len()).collect()
        };
        for index in indices {
            let answer = &self.answers[index];
            history = history.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(answer.question.clone()),
                    )
                    .child(render(format!("lens-answer-{index}"), &answer.output))
                    .child(
                        Button::new(SharedString::from(format!("lens-copy-{index}")))
                            .small()
                            .label(crate::i18n::text("复制回答"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(answer) = this.answers.get(index) {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        answer.output.source().to_string(),
                                    ));
                                }
                            })),
                    ),
            );
        }
        if !self.output.source().is_empty() {
            history = history.child(render("lens-stream".into(), &self.output));
        }
        if !self.reasoning.source().is_empty() {
            history = history.child(
                Button::new("lens-reasoning")
                    .small()
                    .label(crate::i18n::text(if self.reasoning_open {
                        "收起思考过程"
                    } else {
                        "查看思考过程"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reasoning_open = !this.reasoning_open;
                        cx.notify();
                    })),
            );
            if self.reasoning_open {
                history = history.child(render("lens-reasoning-body".into(), &self.reasoning));
            }
        }
        div()
            .key_context("NativeLens")
            .size_full()
            .v_flex()
            .gap_3()
            .p_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(
                cx.listener(|this, _: &CloseLens, window, cx| this.close_or_stop(window, cx)),
            )
            .child(crate::markdown::render::selection_frame_reset_for(
                self.surface,
            ))
            .child(
                div()
                    .h(px(140.))
                    .w_full()
                    .overflow_hidden()
                    .rounded_md()
                    .child(
                        img(self.preview.clone())
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    ),
            )
            .child(crate::settings::widgets::scroll_faded(
                "lens-history",
                div()
                    .id("lens-history-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(history),
            ))
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(Textarea::new(&self.question).disabled(self.committing))
            .when(
                !self.config.send_to_chat && !self.history.is_empty(),
                |view| {
                    view.child(
                        Button::new("lens-continue-chat")
                            .small()
                            .disabled(self.busy)
                            .label(crate::i18n::text("在 Kivio Agent 中继续"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit_mode(true, window, cx)
                            })),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(div().flex().items_center().gap_2().when(
                        !self.config.send_to_chat,
                        |view| {
                            view.child(crate::i18n::text("联网搜索")).child(
                                Switch::new("lens-search")
                                    .checked(self.web_search)
                                    .disabled(self.busy || self.config.send_to_chat)
                                    .on_click(cx.listener(|this, value, _, cx| {
                                        this.web_search = *value;
                                        cx.notify();
                                    })),
                            )
                        },
                    ))
                    .child(
                        Button::new("lens-submit")
                            .disabled(self.committing)
                            .label(crate::i18n::text(if self.committing {
                                "正在发送…"
                            } else if self.busy {
                                "取消"
                            } else if self.config.send_to_chat {
                                "发送到 Kivio Agent"
                            } else {
                                "提问"
                            }))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(cancel) = this.cancel.take() {
                                    let _ = cancel.send(());
                                } else {
                                    this.submit(window, cx);
                                }
                            })),
                    ),
            )
    }
}
