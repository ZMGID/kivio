//! The retained view owns emulator, focus, selection and viewport geometry;
//! the host owns session creation, PTY I/O, errors and session shutdown.
use super::emulator::{CellSnapshot, CursorSnapshot, Emulator, SelectionType};
use super::view::{TerminalElement, cell_at, keystroke_bytes, paste_bytes};
use gpui_kit::component::{ActiveTheme, ThemeStyled};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}
#[derive(Clone, Copy)]
pub(super) struct GridGeometry {
    pub bounds: Bounds<Pixels>,
    pub origin: Point<Pixels>,
    pub cell_w: f32,
    pub line_h: f32,
    pub cols: u16,
    pub rows: u16,
}
pub(super) struct GridSnapshot {
    pub lines: Vec<Vec<CellSnapshot>>,
    pub cursor: Option<CursorSnapshot>,
}
pub struct TerminalView {
    emulator: Emulator,
    focus: FocusHandle,
    geometry: Option<GridGeometry>,
    selecting: bool,
    selection_press: Option<Point<Pixels>>,
}
// Original Zeron element refers to this owner name.
pub(super) type TerminalPanel = TerminalView;
impl EventEmitter<TerminalEvent> for TerminalView {}
impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            emulator: Emulator::new(80, 24),
            focus: cx.focus_handle().tab_stop(true),
            geometry: None,
            selecting: false,
            selection_press: None,
        }
    }
    /// Feed unmodified PTY output. Emulator query responses use the same Write
    /// event as keyboard input; the host must forward both to the same session.
    pub fn feed_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let response = self.emulator.feed(bytes);
        if !response.is_empty() {
            cx.emit(TerminalEvent::Write(response));
        }
        cx.notify();
    }
    pub fn resend_size(&self, cx: &mut Context<Self>) {
        if let Some(geometry) = self.geometry {
            cx.emit(TerminalEvent::Resize {
                cols: geometry.cols,
                rows: geometry.rows,
            });
        }
    }
    pub fn paste(&mut self, text: &str, cx: &mut Context<Self>) {
        self.send(paste_bytes(text, self.emulator.bracketed_paste_mode()), cx);
    }
    fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.emulator.clear_selection();
        self.emulator.scroll_to_bottom();
        cx.emit(TerminalEvent::Write(bytes));
        cx.notify();
    }
    pub(super) fn on_grid_metrics(&mut self, geometry: GridGeometry, cx: &mut Context<Self>) {
        let changed = self
            .geometry
            .is_none_or(|old| old.cols != geometry.cols || old.rows != geometry.rows);
        self.geometry = Some(geometry);
        if changed {
            self.emulator.resize(geometry.cols, geometry.rows);
            cx.emit(TerminalEvent::Resize {
                cols: geometry.cols,
                rows: geometry.rows,
            });
        }
    }
    pub(super) fn active_grid_snapshot(&self, _: &App) -> Option<GridSnapshot> {
        Some(GridSnapshot {
            lines: self.emulator.lines(),
            cursor: self.emulator.cursor(),
        })
    }
    fn select_at(&mut self, position: Point<Pixels>, start: bool) {
        let Some(g) = self.geometry else { return };
        let hit = cell_at(
            f32::from(position.x - g.origin.x),
            f32::from(position.y - g.origin.y),
            g.cell_w,
            g.line_h,
            g.cols as usize,
            g.rows as usize,
        );
        let point = self.emulator.grid_point(hit.row, hit.col);
        if start {
            self.emulator
                .start_selection(SelectionType::Simple, point, hit.side);
        } else {
            self.emulator.update_selection(point, hit.side);
        }
    }
}
impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal-grid")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .track_focus(&self.focus)
            .key_context("Terminal")
            .when(focused, |el| el.focus_ring_style(window, cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.selecting = true;
                    this.selection_press = Some(event.position);
                    this.emulator.clear_selection();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.selecting {
                    if let Some(press) = this.selection_press {
                        let delta = event.position - press;
                        if f32::from(delta.x).abs().max(f32::from(delta.y).abs())
                            < super::view::SELECTION_DRAG_THRESHOLD
                        {
                            return;
                        }
                        this.select_at(press, true);
                        this.selection_press = None;
                    }
                    this.select_at(event.position, false);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.selecting = false;
                    this.selection_press = None;
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(18.));
                this.emulator
                    .scroll((f32::from(delta.y) / 18.).round() as i32);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let key = &event.keystroke;
                if key.modifiers.platform && key.key == "c" {
                    if let Some(text) = this.emulator.selection_text() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        cx.stop_propagation();
                    }
                    return;
                }
                if key.modifiers.platform && key.key == "v" {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        this.paste(&text, cx);
                        cx.stop_propagation();
                    }
                    return;
                }
                if let Some(bytes) = keystroke_bytes(
                    &key.key,
                    key.key_char.as_deref(),
                    &key.modifiers,
                    this.emulator.app_cursor_mode(),
                ) {
                    this.send(bytes, cx);
                    cx.stop_propagation();
                }
            }))
            .child(TerminalElement::new(cx.entity(), focused))
    }
}
