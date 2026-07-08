//! Loader / CancellableLoader —— PI `components/loader.ts` + `cancellable-loader.ts` 端口。
//!
//! 动画 spinner（braille `⠋⠙⠹…`，80ms 帧）+ 消息。PI 用 `setInterval` 调 `tui.requestRender()`；
//! 本端口把「推进帧」拆成显式 [`Loader::tick`]（由 app 层渲染循环按 `interval_ms` 驱动），保持组件
//! 无 I/O 可测。CancellableLoader 加一个 abort 标志，Esc/Ctrl+C 触发。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::super::keybindings::KeybindingsManager;
use super::super::render::Component;
use super::super::text_width::visible_width;
use super::ColorFn;

const DEFAULT_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DEFAULT_INTERVAL_MS: u64 = 80;

/// spinner 指示器配置。
pub struct LoaderIndicator {
    /// 动画帧（空 vec 隐藏指示器）。
    pub frames: Vec<String>,
    /// 帧间隔。
    pub interval_ms: u64,
}

impl Default for LoaderIndicator {
    fn default() -> Self {
        Self {
            frames: DEFAULT_FRAMES.iter().map(|s| s.to_string()).collect(),
            interval_ms: DEFAULT_INTERVAL_MS,
        }
    }
}

/// 带动画的 loading 组件。
pub struct Loader {
    frames: Vec<String>,
    interval_ms: u64,
    current_frame: usize,
    /// 是否原样渲染帧（外部已着色），否则用 `spinner_color_fn`。
    render_verbatim: bool,
    spinner_color_fn: ColorFn,
    message_color_fn: ColorFn,
    message: String,
    padding_x: usize,
}

impl Loader {
    pub fn new(
        spinner_color_fn: ColorFn,
        message_color_fn: ColorFn,
        message: impl Into<String>,
        indicator: Option<LoaderIndicator>,
    ) -> Self {
        let ind = indicator.unwrap_or_default();
        let render_verbatim = false; // 默认非 verbatim（用 default frames）
        Self {
            frames: ind.frames,
            interval_ms: if ind.interval_ms > 0 {
                ind.interval_ms
            } else {
                DEFAULT_INTERVAL_MS
            },
            current_frame: 0,
            render_verbatim,
            spinner_color_fn,
            message_color_fn,
            message: message.into(),
            padding_x: 1,
        }
    }

    /// 设置自定义指示器（verbatim 渲染）。
    pub fn set_indicator(&mut self, indicator: LoaderIndicator) {
        self.render_verbatim = true;
        self.frames = indicator.frames;
        self.interval_ms = if indicator.interval_ms > 0 {
            indicator.interval_ms
        } else {
            DEFAULT_INTERVAL_MS
        };
        self.current_frame = 0;
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    /// 推进一帧（动画 ≤1 帧时为 no-op）。返回是否实际改变。
    pub fn tick(&mut self) -> bool {
        if self.frames.len() <= 1 {
            return false;
        }
        self.current_frame = (self.current_frame + 1) % self.frames.len();
        true
    }

    fn line(&self) -> String {
        let frame = self
            .frames
            .get(self.current_frame)
            .cloned()
            .unwrap_or_default();
        let indicator = if frame.is_empty() {
            String::new()
        } else {
            let rendered = if self.render_verbatim {
                frame
            } else {
                (self.spinner_color_fn)(&frame)
            };
            format!("{rendered} ")
        };
        let msg = (self.message_color_fn)(&self.message);
        let pad = " ".repeat(self.padding_x);
        format!("{pad}{indicator}{msg}{pad}")
    }
}

impl Component for Loader {
    fn render(&mut self, width: u16) -> Vec<String> {
        let line = self.line();
        let w = width as usize;
        let vis = visible_width(&line);
        let padded = if vis < w {
            format!("{line}{}", " ".repeat(w - vis))
        } else {
            line
        };
        // PI 在内容上方留一行空行
        vec![String::new(), padded]
    }
}

/// 可用 Esc/Ctrl+C 取消的 Loader。
pub struct CancellableLoader {
    inner: Loader,
    aborted: Arc<AtomicBool>,
    kb: KeybindingsManager,
    kitty_active: bool,
    pub on_abort: Option<Box<dyn FnMut() + Send>>,
}

impl CancellableLoader {
    pub fn new(
        spinner_color_fn: ColorFn,
        message_color_fn: ColorFn,
        message: impl Into<String>,
        indicator: Option<LoaderIndicator>,
    ) -> Self {
        Self {
            inner: Loader::new(spinner_color_fn, message_color_fn, message, indicator),
            aborted: Arc::new(AtomicBool::new(false)),
            kb: KeybindingsManager::with_defaults(),
            kitty_active: false,
            on_abort: None,
        }
    }

    pub fn set_kitty_active(&mut self, active: bool) {
        self.kitty_active = active;
    }

    /// 取一个可在异步任务里轮询的 abort 句柄。
    pub fn abort_handle(&self) -> Arc<AtomicBool> {
        self.aborted.clone()
    }

    pub fn aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.inner.set_message(message);
    }

    pub fn tick(&mut self) -> bool {
        self.inner.tick()
    }

    pub fn interval(&self) -> Duration {
        self.inner.interval()
    }
}

impl Component for CancellableLoader {
    fn handle_input(&mut self, data: &str) {
        if self
            .kb
            .matches(data, "tui.select.cancel", self.kitty_active)
        {
            self.aborted.store(true, Ordering::SeqCst);
            if let Some(cb) = self.on_abort.as_mut() {
                cb();
            }
        }
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        self.inner.render(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> ColorFn {
        Arc::new(|s: &str| s.to_string())
    }

    #[test]
    fn renders_first_frame_and_message() {
        let mut l = Loader::new(id(), id(), "Loading...", None);
        let lines = l.render(40);
        // blank line + content line
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "");
        assert!(lines[1].contains("⠋"));
        assert!(lines[1].contains("Loading..."));
    }

    #[test]
    fn tick_advances_frame() {
        let mut l = Loader::new(id(), id(), "x", None);
        let before = l.render(40)[1].clone();
        assert!(l.tick());
        let after = l.render(40)[1].clone();
        assert_ne!(before, after);
        assert!(after.contains("⠙"));
    }

    #[test]
    fn tick_noop_with_single_frame() {
        let mut l = Loader::new(id(), id(), "x", None);
        l.set_indicator(LoaderIndicator {
            frames: vec!["*".into()],
            interval_ms: 100,
        });
        assert!(!l.tick());
    }

    #[test]
    fn tick_wraps_around() {
        let mut l = Loader::new(id(), id(), "x", None);
        let frames = DEFAULT_FRAMES.len();
        for _ in 0..frames {
            l.tick();
        }
        // back to frame 0
        assert!(l.render(40)[1].contains("⠋"));
    }

    #[test]
    fn empty_frames_hides_indicator() {
        let mut l = Loader::new(id(), id(), "msg", None);
        l.set_indicator(LoaderIndicator {
            frames: vec![],
            interval_ms: 80,
        });
        let lines = l.render(40);
        assert!(lines[1].contains("msg"));
        assert!(!lines[1].contains("⠋"));
    }

    #[test]
    fn render_pads_to_width() {
        let mut l = Loader::new(id(), id(), "x", None);
        let lines = l.render(30);
        assert_eq!(visible_width(&lines[1]), 30);
    }

    #[test]
    fn cancellable_aborts_on_escape() {
        let mut l = CancellableLoader::new(id(), id(), "Working...", None);
        let handle = l.abort_handle();
        assert!(!handle.load(Ordering::SeqCst));
        l.handle_input("\x1b"); // escape
        assert!(l.aborted());
        assert!(handle.load(Ordering::SeqCst));
    }

    #[test]
    fn cancellable_aborts_on_ctrl_c() {
        let mut l = CancellableLoader::new(id(), id(), "Working...", None);
        l.handle_input("\x03"); // ctrl+c
        assert!(l.aborted());
    }

    #[test]
    fn cancellable_ignores_other_keys() {
        let mut l = CancellableLoader::new(id(), id(), "Working...", None);
        l.handle_input("a");
        assert!(!l.aborted());
    }
}
