//! Native input translation window. Request cancellation follows its window lifetime.
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::Button,
    input::{InputEvent, Textarea, TextareaState},
};
use gpui_kit::{prelude::*, *};
use std::path::PathBuf;

actions!(input_translation, [TranslateInput, CloseTranslation]);

struct OpenTranslation(WindowHandle<gpui_kit::base::Root>);
impl Global for OpenTranslation {}
/// Installed by the app keymap owner every time it rebuilds the keymap.
/// Keeping this with the window actions prevents one-time global markers from
/// outliving `clear_key_bindings` when a main window is reopened.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-enter"
            } else {
                "ctrl-enter"
            },
            TranslateInput,
            Some("InputTranslation"),
        ),
        KeyBinding::new("escape", CloseTranslation, Some("InputTranslation")),
    ]);
}

pub fn open(directory: PathBuf, cx: &mut App) -> Result<(), String> {
    open_with_text(directory, None, cx)
}

pub fn open_selection(directory: PathBuf, text: String, cx: &mut App) -> Result<(), String> {
    open_with_text(directory, Some(text), cx)
}

fn open_with_text(
    directory: PathBuf,
    selected: Option<String>,
    cx: &mut App,
) -> Result<(), String> {
    let is_selection = selected.is_some();
    let selection_config = if is_selection {
        Some(kivio_model_runtime::native_preferences::load(&directory)?.screenshot_translation)
    } else {
        None
    };
    if let Some(handle) = cx
        .try_global::<OpenTranslation>()
        .filter(|_| !is_selection)
        .map(|state| state.0)
    {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            cx.activate(true);
            return Ok(());
        }
    }
    let width = selection_config
        .as_ref()
        .map(|config| (config.card_width as f32).clamp(360., 720.))
        .unwrap_or(680.);
    let bounds = Bounds::centered(None, size(px(width), px(560.)), cx);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(if is_selection { 360. } else { 480. }), px(400.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(
                        if is_selection {
                            "Kivio · 选中文本翻译"
                        } else {
                            "Kivio · 输入翻译"
                        }
                        .into(),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| {
                    let mut view = InputTranslation::new(directory, window, cx);
                    if let Some(text) = selected {
                        view.selection = true;
                        view.show_source = !selection_config
                            .as_ref()
                            .is_some_and(|config| config.direct_translate);
                        let auto_translate = !text.trim().is_empty();
                        view.source
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                        if auto_translate {
                            view.translate(window, cx);
                        }
                    }
                    view
                });
                cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
            },
        )
        .map_err(|error| error.to_string())?;
    if !is_selection {
        cx.set_global(OpenTranslation(handle));
    }
    cx.activate(true);
    Ok(())
}
struct InputTranslation {
    directory: PathBuf,
    _appearance: Subscription,
    _submit: Subscription,
    source: Entity<TextareaState>,
    output: Entity<TextareaState>,
    selection: bool,
    show_source: bool,
    busy: bool,
    error: Option<String>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<Task<()>>,
}
impl InputTranslation {
    fn new(directory: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = cx.new(|cx| TextareaState::new(window, cx).rows(6).submit_on_enter(true));
        window.focus(&source.read(cx).focus_handle(cx), cx);
        let submit = cx.subscribe_in(&source, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { shift: false, .. } => this.translate(window, cx),
            InputEvent::Change => {
                this.error = None;
                cx.notify();
            }
            _ => {}
        });
        Self {
            directory,
            _appearance: crate::appearance::observe_window(window, cx),
            source,
            _submit: submit,
            output: cx.new(|cx| TextareaState::new(window, cx).rows(6)),
            selection: false,
            show_source: true,
            busy: false,
            error: None,
            cancel: None,
            task: None,
        }
    }
    fn translate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let text = self.source.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.error = Some("请输入需要翻译的文字".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        let directory = self.directory.clone();
        let selection = self.selection;
        self.output
            .update(cx, |input, cx| input.set_value("", window, cx));
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        self.cancel = Some(cancel);
        let handle = window.window_handle();
        self.task = Some(cx.spawn(async move |this, cx| {
            let (deltas, receiver) = async_channel::unbounded::<String>();
            let work = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(async move {
                    tokio::select! {
                        _ = cancelled => Err("已取消翻译".to_string()),
                        result = async {
                            if selection {
                                let mut sink = |part| {
                                    if let kivio_model_runtime::StreamPart::TextDelta { delta } = part {
                                        deltas.try_send(delta).map_err(|_| kivio_model_runtime::ModelError::new("cancelled"))?;
                                    }
                                    Ok(())
                                };
                                kivio_model_runtime::native_translation::translate_selection(&directory, &text, &mut sink).await
                            } else {
                                kivio_model_runtime::native_translation::translate(&directory, &text).await
                            }
                        } => result,
                    }
                })
            });
            let mut partial = String::new();
            while let Ok(delta) = receiver.recv().await {
                partial.push_str(&delta);
                if cx.update_window(handle, |_, window, cx| {
                    this.update(cx, |this, cx| {
                        this.output.update(cx, |input, cx| input.set_value(partial.clone(), window, cx));
                        cx.notify();
                    })
                }).is_err() { return; }
            }
            let result = work.await;
            let _ = cx.update_window(handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.busy = false; this.cancel = None;
                    match result {
                        Ok(text) => this.output.update(cx, |input, cx| input.set_value(text, window, cx)),
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }
}
impl Render for InputTranslation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("input-translation-scroll")
            .key_context("InputTranslation")
            .on_action(
                cx.listener(|this, _: &TranslateInput, window, cx| this.translate(window, cx)),
            )
            .on_action(|_: &CloseTranslation, window, _| window.remove_window())
            .size_full()
            .overflow_y_scroll()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .p_6()
            .child(
                div()
                    .v_flex()
                    .gap_4()
                    .child(div().text_lg().child(if self.selection {
                        "选中文本翻译"
                    } else {
                        "输入翻译"
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if self.selection {
                                "使用快速翻译配置。Enter 翻译，Shift+Enter 换行。"
                            } else {
                                "使用翻译设置中的模型、语言与提示词。Enter 翻译，Shift+Enter 换行。"
                            }),
                    )
                    .when(self.selection, |view| {
                        view.child(
                            Button::new("toggle-source")
                                .small()
                                .label(if self.show_source {
                                    "收起原文"
                                } else {
                                    "显示原文"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_source = !this.show_source;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.show_source, |view| {
                        view.child(crate::i18n::text("原文")).child(
                            Textarea::new(&self.source)
                                .aria_label("翻译原文")
                                .disabled(self.busy),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("translate-input")
                                    .small()
                                    .label(if self.busy { "翻译中…" } else { "翻译" })
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.translate(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("paste-translation-source")
                                    .small()
                                    .label(crate::i18n::text("粘贴原文"))
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(text) =
                                            cx.read_from_clipboard().and_then(|item| item.text())
                                        {
                                            this.source.update(cx, |input, cx| {
                                                input.set_value(text, window, cx)
                                            });
                                        }
                                    })),
                            )
                            .when(self.busy, |view| {
                                view.child(
                                    Button::new("cancel-translation")
                                        .small()
                                        .label(crate::i18n::text("停止"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(cancel) = this.cancel.take() {
                                                let _ = cancel.send(());
                                            }
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .when_some(self.error.clone(), |view, error| {
                        view.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    })
                    .child(crate::i18n::text("译文"))
                    .child(Textarea::new(&self.output).aria_label("翻译结果"))
                    .child(
                        div().flex().child(
                            Button::new("copy-translation")
                                .small()
                                .label(crate::i18n::text("复制译文"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        this.output.read(cx).value().to_string(),
                                    ))
                                })),
                        ),
                    ),
            )
    }
}
