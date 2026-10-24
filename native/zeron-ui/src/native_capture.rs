//! Frozen-screen selection and screenshot translation; one live session per app.
use gpui_kit::{prelude::*, actions, App, Global, Task, AnyWindowHandle, Context,
    Window, WindowOptions, WindowBounds, WindowKind, TitlebarOptions, DisplayId,
    KeyBinding, Entity, FocusHandle, Image, ImageFormat, ImageSource, Point, Pixels,
    Bounds, MouseButton, MouseDownEvent, MouseUpEvent, MouseMoveEvent,
    Render, IntoElement, ClipboardItem, div, img, px, point, size, rgba};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, StyledExt, WindowExt,
    button::Button, notification::Notification};
use kivio_model_runtime::{capture_geometry::CaptureRect, native_capture::{CaptureRuntime, FrozenFrame}};
use std::{path::PathBuf, sync::Arc};

actions!(native_capture, [CloseCapture]);
#[derive(Default)]
struct CaptureState {
    runtime: Option<Arc<CaptureRuntime>>,
    window: Option<AnyWindowHandle>,
    task: Option<Task<()>>,
}
impl Global for CaptureState {}
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseCapture, Some("NativeCapture"))]);
}
fn close(cx: &mut App) {
    if !cx.has_global::<CaptureState>() { return; }
    let state = cx.global_mut::<CaptureState>();
    if let Some(runtime) = state.runtime.take() { runtime.close(); }
    state.task = None;
    if let Some(handle) = state.window.take() {
        let _ = cx.update_window(handle, |_, window, _| window.remove_window());
    }
}
fn report(error: String, cx: &mut App) {
    for (handle, _) in crate::shell_windows(cx) {
        let error = error.clone();
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.push_notification(Notification::error(error), cx);
        });
    }
}
pub(crate) fn toggle(root: PathBuf, cx: &mut App) -> Result<(), String> {
    toggle_mode(root, false, cx)
}
pub(crate) fn toggle_lens(root: PathBuf, cx: &mut App) -> Result<(), String> {
    if crate::native_lens::toggle_existing(cx) { return Ok(()); }
    toggle_mode(root, true, cx)
}
fn toggle_mode(root: PathBuf, is_lens: bool, cx: &mut App) -> Result<(), String> {
    if !cx.has_global::<CaptureState>() { cx.set_global(CaptureState::default()); }
    if cx.global::<CaptureState>().runtime.as_ref().is_some_and(|r| r.session.is_session_active(r.session.session_seq())) {
        close(cx);
        return Ok(());
    }
    close(cx);
    let settings = kivio_model_runtime::native_preferences::load(&root)?;
    if is_lens && !settings.lens.enabled { return Err("Lens 已在设置中关闭".into()); }
    let show_hint = !is_lens || settings.lens.show_capture_hint;
    let config = settings.screenshot_translation;
    if !is_lens && !config.enabled { return Err("截图翻译已在设置中关闭".into()); }
    let displays = cx.displays();
    #[cfg(target_os = "macos")]
    let cursor = {
        use core_graphics::{event::CGEvent, event_source::{CGEventSource, CGEventSourceStateID}};
        CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()
            .and_then(|source| CGEvent::new(source).ok())
            .map(|event| { let p = event.location(); point(px(p.x as f32), px(p.y as f32)) })
    };
    #[cfg(not(target_os = "macos"))]
    let cursor: Option<Point<Pixels>> = None;
    let display = cursor.and_then(|point| displays.iter().find(|d| d.bounds().contains(&point)).cloned())
        .or_else(|| cx.primary_display()).or_else(|| displays.first().cloned()).ok_or("没有可用显示器")?;
    let bounds = display.bounds();
    let display_id = display.id();
    let geometry = CaptureRect { x: f32::from(bounds.origin.x) as f64, y: f32::from(bounds.origin.y) as f64,
        width: f32::from(bounds.size.width) as f64, height: f32::from(bounds.size.height) as f64 };
    let runtime = Arc::new(CaptureRuntime::default());
    let session = runtime.session.begin_session(false).map_err(|_| "截图会话正在运行")?;
    cx.global_mut::<CaptureState>().runtime = Some(runtime.clone());
    let task = cx.spawn(async move |cx| {
        let capture = runtime.clone();
        let result = cx.background_spawn(async move {
            let frame = capture.freeze_display(session, geometry)?;
            let bytes = std::fs::read(&frame.image.path).map_err(|error| error.to_string())?;
            Ok::<_, String>((frame, Arc::new(Image::from_bytes(ImageFormat::Png, bytes)), kivio_model_runtime::lens::list_windows()))
        }).await;
        let _ = cx.update(|cx| {
            if !runtime.session.is_session_active(session) { return; }
            match result {
                Ok((frame, image, windows)) => {
                    let result = cx.open_window(WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)), display_id: Some(display_id),
                        titlebar: None, kind: WindowKind::PopUp, is_movable: false, is_resizable: false,
                        is_minimizable: false, ..Default::default()
                    }, |window, cx| {
                        let view = cx.new(|cx| {
                            let released_image = image.clone();
                            cx.on_release(move |_, cx| ImageSource::Image(released_image).remove_asset(cx)).detach();
                            let focus = cx.focus_handle(); window.focus(&focus, cx);
                            CaptureView { root, is_lens, show_hint, display_id, runtime: runtime.clone(), session, frame, image, focus,
                                keep_fullscreen: config.keep_fullscreen_after_capture,
                                card_width: config.card_width.clamp(360, 720) as f32, floating: false,
                                start: None, end: None, windows, hovered: None, captured_window: None, selected: false, busy: false, error: None,
                                source: crate::markdown::IncrementalParser::new(),
                                output: crate::markdown::IncrementalParser::new(),
                                reasoning: crate::markdown::IncrementalParser::new(), reasoning_open: false,
                                cancel: None, task: None }
                        });
                        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
                    });
                    match result {
                        Ok(handle) => { cx.global_mut::<CaptureState>().window = Some(handle.into()); cx.activate(true); }
                        Err(error) => { runtime.close(); report(error.to_string(), cx); }
                    }
                }
                Err(error) => { runtime.close(); report(error, cx); }
            }
        });
    });
    cx.global_mut::<CaptureState>().task = Some(task);
    Ok(())
}
struct CaptureView {
    root: PathBuf,
    is_lens: bool,
    show_hint: bool,
    display_id: DisplayId,
    runtime: Arc<CaptureRuntime>,
    session: u64,
    frame: FrozenFrame,
    image: Arc<Image>,
    focus: FocusHandle,
    keep_fullscreen: bool,
    card_width: f32,
    floating: bool,
    start: Option<Point<Pixels>>,
    end: Option<Point<Pixels>>,
    windows: Vec<kivio_model_runtime::lens::WindowInfo>,
    hovered: Option<usize>,
    captured_window: Option<u32>,
    selected: bool,
    busy: bool,
    error: Option<String>,
    source: crate::markdown::IncrementalParser,
    output: crate::markdown::IncrementalParser,
    reasoning: crate::markdown::IncrementalParser,
    reasoning_open: bool,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<Task<()>>,
}
// Must be called after releasing any CaptureView lease: opening draws synchronously.
fn move_result_window(view: &Entity<CaptureView>, previous: AnyWindowHandle, kind: WindowKind, cx: &mut App) -> Result<AnyWindowHandle, String> {
    if !view.read(cx).runtime.session.is_session_active(view.read(cx).session) {
        return Err("截图会话已结束".into());
    }
    cx.update_window(previous, |_, _, _| ()).map_err(|error| error.to_string())?;
    let focus = view.read(cx).focus.clone();
    let display = view.read(cx).display_id;
    let bounds = Bounds::centered(Some(display), size(px(view.read(cx).card_width), px(560.)), cx);
    let destination = cx.open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)), display_id: Some(display),
        titlebar: Some(TitlebarOptions { title: Some("Kivio · 截图翻译".into()), ..Default::default() }),
        window_min_size: Some(size(px(360.), px(320.))), kind, ..Default::default()
    }, |window, cx| {
        window.focus(&focus, cx);
        cx.new(|cx| gpui_kit::base::Root::new(view.clone(), window, cx))
    }).map_err(|error| error.to_string())?;
    let destination = destination.into();
    cx.global_mut::<CaptureState>().window = Some(destination);
    let _ = cx.update_window(previous, |_, window, _| window.remove_window());
    Ok(destination)
}

impl Drop for CaptureView {
    fn drop(&mut self) { self.cancel.take(); self.runtime.close(); }
}
impl CaptureView {
    fn open_lens(&mut self, window: &Window, cx: &mut Context<Self>) {
        let display = self.display_id;
        let Some(rect) = self.capture_rect() else { self.busy = false; return; };
        let root = self.root.clone();
        let runtime = self.runtime.clone();
        let session = self.session;
        let captured_window = self.captured_window;
        let frame = self.frame.clone();
        let handle = window.window_handle();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                let image = match captured_window {
                    Some(id) => runtime.capture_window(session, id)?,
                    None => runtime.capture_region(session, rect, Some(&frame))?,
                };
                let settings = kivio_model_runtime::native_preferences::load(&root)?;
                if let Err(error) = kivio_model_runtime::capture_runtime::archive_captured_image(&settings, &image.path, &image.id) {
                    tracing::warn!(%error, "Lens capture archive failed");
                }
                let bytes = std::fs::read(&image.path).map_err(|error| error.to_string())?;
                let retained = tempfile::Builder::new().prefix("kivio-lens-").suffix(".png").tempfile().map_err(|error| error.to_string())?.into_temp_path();
                std::fs::copy(&image.path, &retained).map_err(|error| error.to_string())?;
                Ok::<_, String>((root, retained, bytes, settings.lens))
            }).await;
            let _ = cx.update_window(handle, |_, _, cx| this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((root, path, bytes, config)) => cx.defer(move |cx| {
                        match crate::native_lens::open(root, path, bytes, config, display, cx) {
                            Ok(()) => close(cx),
                            Err(error) => report(error, cx),
                        }
                    }),
                    Err(error) => { this.error = Some(error); this.selected = false; cx.notify(); }
                }
            }));
        }));
    }
    fn selection(&self) -> Option<Bounds<Pixels>> {
        if self.start.is_none() {
            let target = self.windows.get(self.hovered?)?;
            return Some(self.window_bounds(target));
        }
        let a = self.start?; let b = self.end?;
        Some(Bounds { origin: point(a.x.min(b.x), a.y.min(b.y)), size: size((a.x-b.x).abs(), (a.y-b.y).abs()) })
    }
    fn window_bounds(&self, target: &kivio_model_runtime::lens::WindowInfo) -> Bounds<Pixels> {
        let x = (target.x - self.frame.bounds.x).clamp(0., self.frame.bounds.width);
        let y = (target.y - self.frame.bounds.y).clamp(0., self.frame.bounds.height);
        let right = (target.x + target.width - self.frame.bounds.x).clamp(x, self.frame.bounds.width);
        let bottom = (target.y + target.height - self.frame.bounds.y).clamp(y, self.frame.bounds.height);
        Bounds { origin: point(px(x as f32), px(y as f32)), size: size(px((right-x) as f32), px((bottom-y) as f32)) }
    }
    fn hover_window(&mut self, point: Point<Pixels>) {
        let x = self.frame.bounds.x + f32::from(point.x) as f64;
        let y = self.frame.bounds.y + f32::from(point.y) as f64;
        self.hovered = self.windows.iter().position(|target| x >= target.x && x < target.x + target.width && y >= target.y && y < target.y + target.height);
    }
    fn release(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected || self.start.is_none() { return; }
        self.end = Some(point(event.position.x.clamp(px(0.), px(self.frame.bounds.width as f32)),
            event.position.y.clamp(px(0.), px(self.frame.bounds.height as f32))));
        let bounds = self.selection().expect("selection started");
        if bounds.size.width < px(4.) && bounds.size.height < px(4.) {
            self.hover_window(event.position);
            if let Some(target) = self.hovered.and_then(|index| self.windows.get(index)) {
                let bounds = self.window_bounds(target);
                self.captured_window = Some(target.id);
                self.start = Some(bounds.origin);
                self.end = Some(point(bounds.origin.x + bounds.size.width, bounds.origin.y + bounds.size.height));
            }
        }
        self.finish(window, cx);
    }
    fn capture_rect(&self) -> Option<CaptureRect> {
        let bounds = self.selection()?;
        Some(CaptureRect { x: self.frame.bounds.x + f32::from(bounds.origin.x) as f64,
            y: self.frame.bounds.y + f32::from(bounds.origin.y) as f64,
            width: f32::from(bounds.size.width) as f64, height: f32::from(bounds.size.height) as f64 })
    }
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected || self.busy { return; }
        let Some(bounds) = self.selection() else { return; };
        if bounds.size.width < px(10.) || bounds.size.height < px(10.) {
            self.start = None; self.end = None; self.captured_window = None; cx.notify(); return;
        }
        self.selected = true; self.busy = true;
        if self.is_lens { self.open_lens(window, cx); return; }
        let Some(rect) = self.capture_rect() else { return; };
        let frame = self.frame.clone();
        let handle = window.window_handle();
        if !self.keep_fullscreen {
            let view = cx.entity();
            self.floating = true;
            // open_window draws synchronously. Release the CaptureView lease before
            // attaching the same entity to the result window.
            cx.defer(move |cx| {
                if !view.read(cx).runtime.session.is_session_active(view.read(cx).session)
                    || cx.update_window(handle, |_, _, _| ()).is_err() { return; }
                let destination = match move_result_window(&view, handle, WindowKind::Floating, cx) {
                    Ok(destination) => destination,
                    Err(error) => {
                        view.update(cx, |this, _| { this.floating = false; this.error = Some(format!("无法打开翻译浮窗：{error}")); });
                        handle
                    }
                };
                view.update(cx, |this, cx| this.translate_capture(rect, frame, destination, cx));
            });
        } else {
            self.translate_capture(rect, frame, handle, cx);
        }
        cx.notify();
    }
    fn translate_capture(&mut self, rect: CaptureRect, frame: FrozenFrame, handle: AnyWindowHandle, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = None;
        self.output.reset("");
        self.reasoning.reset("");
        let retained = self.runtime.session.current_image_id().and_then(|id| self.runtime.session.image_path(&id));
        let root = self.root.clone();
        let captured_window = self.captured_window;
        let runtime = self.runtime.clone();
        let session = self.session;
        let (cancel, cancelled) = tokio::sync::oneshot::channel(); self.cancel = Some(cancel);
        self.task = Some(cx.spawn(async move |this, cx| {
            let (sender, receiver) = async_channel::unbounded::<(bool, String)>();
            let work = cx.background_spawn(async move {
                let image_path = if let Some(path) = retained {
                    path
                } else {
                    let image = match captured_window {
                        Some(id) => runtime.capture_window(session, id)?,
                        None => runtime.capture_region(session, rect, Some(&frame))?,
                    };
                    let settings = kivio_model_runtime::native_preferences::load(&root)?;
                    if let Err(error) = kivio_model_runtime::capture_runtime::archive_captured_image(&settings, &image.path, &image.id) {
                        tracing::warn!(%error, "capture archive failed; retaining selected image");
                    }
                    image.path
                };
                crate::chat_runtime::runtime()?.block_on(async move {
                    let mut sink = |part| {
                        let delta = match part {
                            kivio_model_runtime::StreamPart::TextDelta { delta } => Some((false, delta)),
                            kivio_model_runtime::StreamPart::ReasoningDelta { delta } => Some((true, delta)),
                            _ => None,
                        };
                        if let Some(delta) = delta {
                            sender.try_send(delta).map_err(|_| kivio_model_runtime::ModelError::new("cancelled"))?;
                        }
                        Ok(())
                    };
                    tokio::select! {
                        _ = cancelled => Err("已取消翻译".into()),
                        result = kivio_model_runtime::native_translation::translate_image(&root, &image_path, &mut sink) => result,
                    }
                })
            });
            while let Ok((reasoning, delta)) = receiver.recv().await {
                if cx.update_window(handle, |_, _, cx| this.update(cx, |this, cx| {
                    if reasoning { this.reasoning.append(&delta); } else { this.output.append(&delta); }
                    cx.notify();
                })).is_err() { return; }
            }
            let result = work.await;
            let _ = cx.update_window(handle, |_, window, cx| this.update(cx, |this, cx| {
                this.busy = false; this.cancel = None;
                match result {
                    Ok(result) => {
                        this.source.reset(&result.original);
                        this.output.reset(&result.translated);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            }));
        }));
        cx.notify();
    }
}
impl Render for CaptureView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selection = self.selection();
        let theme = crate::theme::Theme::of(cx).clone();
        let markdown = |name: &str, parser: &crate::markdown::IncrementalParser| {
            crate::markdown::render::render_tree(&parser.display_tree(),
                &crate::markdown::render::RenderOptions::settled(format!("capture-{}-{name}", self.session).into()),
                &theme, window, &|_| None)
        };
        div().id("native-capture").key_context("NativeCapture").track_focus(&self.focus)
            .size_full().relative().overflow_hidden()
            .on_action(|_: &CloseCapture, window, _| window.remove_window())
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if !this.selected { this.captured_window = None; this.start = Some(event.position); this.end = Some(event.position); cx.notify(); }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if !this.selected && this.start.is_none() { this.hover_window(event.position); cx.notify(); }
                if !this.selected && this.start.is_some() {
                    this.end = Some(point(event.position.x.clamp(px(0.), px(this.frame.bounds.width as f32)),
                        event.position.y.clamp(px(0.), px(this.frame.bounds.height as f32)))); cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, event: &MouseUpEvent, window, cx| this.release(event, window, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, event: &MouseUpEvent, window, cx| this.release(event, window, cx)))
            .when(!self.floating, |view| view.child(img(self.image.clone()).absolute().inset_0().size_full())
                .child(div().absolute().inset_0().bg(rgba(0x00000044))))
            .when_some(selection.filter(|_| !self.floating), |view, bounds| view.child(div().absolute().left(bounds.origin.x).top(bounds.origin.y)
                .w(bounds.size.width).h(bounds.size.height).border_2().border_color(cx.theme().primary)))
            .when(self.is_lens && self.busy, |view| view.child(div().absolute().top_4().left_4().px_4().py_2().rounded_md()
                .bg(cx.theme().background).text_color(cx.theme().foreground).child(crate::i18n::text("正在准备截图…"))))
            .when_some(self.error.clone().filter(|_| !self.selected), |view, error| view.child(div().absolute().bottom_4().left_4().p_3()
                .bg(cx.theme().background).text_color(cx.theme().danger).child(error)))
            .when(!self.selected && self.show_hint, |view| view.child(div().absolute().top_4().left_4().px_4().py_2()
                .rounded_md().bg(cx.theme().background).text_color(cx.theme().foreground)
                .child(crate::i18n::text(if self.is_lens { "点击窗口或拖动选择提问区域 · Esc 取消" } else { "点击窗口或拖动选择翻译区域 · Esc 取消" }))))
            .when(self.selected && !self.is_lens, |view| view.child(div().absolute()
                .map(|view| if self.floating { view.inset_0() } else { view.right_4().top_4().bottom_4().w(px(self.card_width)) })
                .rounded_lg().overflow_hidden()
                .child(crate::settings::widgets::scroll_faded("capture-result-scroll", div()
                .id("capture-result").size_full().overflow_y_scroll().v_flex().gap_3().p_4()
                .bg(cx.theme().background).text_color(cx.theme().foreground)
                .child(div().flex().justify_between().child(crate::i18n::text("截图翻译"))
                    .child(Button::new("close-capture").small().label(crate::i18n::text("关闭")).on_click(|_, window, _| window.remove_window())))
                .when(self.busy, |view| view.child(crate::i18n::text("正在翻译…")))
                .child(crate::markdown::render::selection_frame_reset())
                .when_some(self.error.clone(), |view, error| {
                    let missing = error == "rapidocr_models_missing";
                    view.child(div().text_color(cx.theme().danger).child(if missing { "尚未安装离线识别模型，请在翻译设置中下载后重试。".into() } else { error }))
                        .when(missing, |view| view.child(Button::new("capture-ocr-settings").label(crate::i18n::text("打开翻译设置"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let view = cx.entity();
                                let handle = window.window_handle();
                                this.floating = true;
                                cx.defer(move |cx| {
                                    // Preserve the selected image and result owner while opening
                                    // settings; a fullscreen PopUp must no longer cover the main UI.
                                    match move_result_window(&view, handle, WindowKind::Normal, cx) {
                                        Ok(result) => { let _ = cx.update_window(result, |_, window, _| window.minimize_window()); }
                                        Err(error) => { report(error, cx); return; }
                                    }
                                    crate::activate_main_window(cx);
                                    for (handle, shell) in crate::shell_windows(cx) {
                                        let _ = cx.update_window(handle.into(), |_, window, cx| {
                                            shell.update(cx, |shell, cx| shell.open_settings(crate::shell::SettingsSection::Kivio(crate::kivio_settings::Section::Translate), cx));
                                            window.activate_window();
                                        });
                                    }
                                });
                            }))))
                })
                .when(!self.reasoning.source().is_empty(), |view| view
                    .child(Button::new("capture-reasoning").small().label(if self.reasoning_open { "收起思考过程" } else { "查看思考过程" })
                        .on_click(cx.listener(|this, _, _, cx| { this.reasoning_open = !this.reasoning_open; cx.notify(); })))
                    .when(self.reasoning_open, |view| view.child(markdown("reasoning", &self.reasoning))))
                .when(!self.output.source().is_empty(), |view| view.child(markdown("translation", &self.output)))
                .when(self.error.is_some(), |view| view.child(Button::new("retry-capture-translation").label(crate::i18n::text("重试翻译")).disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if !this.busy && let Some(rect) = this.capture_rect() {
                            this.translate_capture(rect, this.frame.clone(), window.window_handle(), cx);
                        }
                    }))))
                .child(Button::new("copy-capture-translation").label(crate::i18n::text("复制译文")).disabled(self.output.source().is_empty())
                    .on_click(cx.listener(|this, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(this.output.source().to_string())))))
                .when(!self.source.source().is_empty(), |view| view.child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::text("原文")))
                    .child(markdown("original", &self.source)))))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, PlatformInput};

    #[gpui_kit::test]
    fn releasing_selection_opens_floating_result_and_escape_cleans_session(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("frozen.png");
        image::RgbaImage::from_pixel(800, 600, image::Rgba([255,255,255,255])).save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let runtime = Arc::new(CaptureRuntime::default());
        let session = runtime.session.begin_session(false).unwrap();
        runtime.session.register_freeze_frame(session, "fixture".into(), path.clone()).unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(crate::theme::Theme::default());
            cx.set_global(CaptureState { runtime: Some(runtime.clone()), ..Default::default() });
            bind_keys(cx);
        });
        let mut capture = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| {
                let focus = cx.focus_handle(); window.focus(&focus, cx);
                CaptureView {
                    root: root.path().into(), is_lens: false, show_hint: true, display_id: cx.primary_display().unwrap().id(),
                    runtime: runtime.clone(), session,
                    frame: FrozenFrame { image: kivio_model_runtime::native_capture::CapturedImage {
                        id: "fixture".into(), path: path.clone(), width: 800, height: 600,
                    }, bounds: CaptureRect { x:0.,y:0.,width:800.,height:600. } },
                    image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)), focus,
                    keep_fullscreen: false, card_width: 480., floating: false,
                    start: None, end: None, windows: vec![], hovered: None, captured_window: None,
                    selected: false, busy: false, error: None,
                    source: crate::markdown::IncrementalParser::new(),
                    output: crate::markdown::IncrementalParser::new(),
                                reasoning: crate::markdown::IncrementalParser::new(), reasoning_open: false,
                    cancel: None, task: None,
                }
            });
            capture = Some(view.clone());
            gpui_kit::base::Root::new(view, window, cx)
        });
        cx.update(|cx| cx.global_mut::<CaptureState>().window = Some(window.into()));
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left, position: point(px(200.),px(200.)),
                modifiers: Default::default(), click_count: 1, first_mouse: false,
            }), cx);
            // No intermediate move: the release position must determine the crop.
            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left, position: point(px(100.),px(120.)),
                modifiers: Default::default(), click_count: 1,
            }), cx);
        }).unwrap();
        cx.run_until_parked();
        let capture = capture.unwrap();
        let destination = cx.update(|cx| {
            let view = capture.read(cx);
            assert!(view.floating);
            assert_eq!(view.selection().unwrap().size, size(px(100.),px(80.)));
            assert_eq!(cx.windows().len(), 1, "selection overlay must close after successful handoff");
            cx.global::<CaptureState>().window.unwrap()
        });
        drop(capture);
        cx.update_window(destination, |_, window, cx| {
            window.dispatch_action(Box::new(CloseCapture), cx);
        }).unwrap();
        cx.run_until_parked();
        assert!(!runtime.session.is_session_active(session));
        assert!(!path.exists());
    }
}
