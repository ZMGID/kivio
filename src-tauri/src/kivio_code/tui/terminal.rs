//! 终端抽象 —— PI `terminal.ts` 的 Rust 端口（精简版）。
//!
//! 渲染器通过 [`Terminal`] trait 写出，便于用 fake（[`BufferTerminal`]）做单测。本阶段聚焦
//! 差分渲染器需要的接口：尺寸查询、缓冲写入、光标 / 清行原语、resize 订阅。**渲染进 NORMAL
//! buffer**（不进 alt-screen），让内容自然滚入 scrollback，对标 PI。
//!
//! 真正的 raw-mode / Kitty 协议协商 / Windows VT 输入交给后续阶段的 crossterm 后端实现；
//! 这里先提供 trait + 内存 fake + 一个直写 stdout 的最小实现（`StdoutTerminal`）。

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 终端能力 + 写出接口。渲染器只依赖本 trait。
pub trait Terminal {
    /// 当前列数。
    fn columns(&self) -> u16;
    /// 当前行数。
    fn rows(&self) -> u16;
    /// 写出一段已构造好的 ANSI 字节（渲染器把整帧拼好后一次写入）。
    fn write(&mut self, data: &str);
    /// 隐藏硬件光标（`\x1b[?25l`）。
    fn hide_cursor(&mut self) {
        self.write("\x1b[?25l");
    }
    /// 显示硬件光标（`\x1b[?25h`）。
    fn show_cursor(&mut self) {
        self.write("\x1b[?25h");
    }
    /// 相对上下移动光标。正数向下，负数向上。
    fn move_by(&mut self, lines: i32) {
        match lines.cmp(&0) {
            std::cmp::Ordering::Greater => self.write(&format!("\x1b[{lines}B")),
            std::cmp::Ordering::Less => self.write(&format!("\x1b[{}A", -lines)),
            std::cmp::Ordering::Equal => {}
        }
    }
    /// 清除当前行（`\x1b[2K`）。
    fn clear_line(&mut self) {
        self.write("\x1b[2K");
    }
    /// 从光标清到行尾（`\x1b[0K`）。
    fn clear_from_cursor(&mut self) {
        self.write("\x1b[0K");
    }
    /// 清屏 + home（`\x1b[2J\x1b[H`）。
    fn clear_screen(&mut self) {
        self.write("\x1b[2J\x1b[H");
    }
    /// 设置窗口标题（OSC 0）。
    fn set_title(&mut self, title: &str) {
        self.write(&format!("\x1b]0;{title}\x07"));
    }
}

/// 内存终端：累计所有写入，供单测断言精确的转义输出。尺寸固定（可改 via [`set_size`]）。
pub struct BufferTerminal {
    columns: u16,
    rows: u16,
    buffer: String,
}

impl BufferTerminal {
    pub fn new(columns: u16, rows: u16) -> Self {
        Self {
            columns,
            rows,
            buffer: String::new(),
        }
    }

    /// 取出并清空累计的写入（便于一帧一断言）。
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.buffer)
    }

    /// 查看累计的写入而不清空。
    pub fn output(&self) -> &str {
        &self.buffer
    }

    /// 改变报告的尺寸（模拟 resize）。
    pub fn set_size(&mut self, columns: u16, rows: u16) {
        self.columns = columns;
        self.rows = rows;
    }
}

impl Terminal for BufferTerminal {
    fn columns(&self) -> u16 {
        self.columns
    }
    fn rows(&self) -> u16 {
        self.rows
    }
    fn write(&mut self, data: &str) {
        self.buffer.push_str(data);
    }
}

/// 直写 stdout 的最小终端实现：尺寸来自 `COLUMNS`/`LINES` 环境变量（CI / 管道场景），
/// 缺省 80x24。raw-mode / 协议协商等留给后续 crossterm 后端。
pub struct StdoutTerminal {
    columns: u16,
    rows: u16,
}

impl StdoutTerminal {
    pub fn new() -> Self {
        let columns = std::env::var("COLUMNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80);
        let rows = std::env::var("LINES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        Self { columns, rows }
    }

    /// 显式设置尺寸（接入真实终端尺寸查询前的桥接点）。
    pub fn set_size(&mut self, columns: u16, rows: u16) {
        self.columns = columns;
        self.rows = rows;
    }
}

impl Default for StdoutTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal for StdoutTerminal {
    fn columns(&self) -> u16 {
        self.columns
    }
    fn rows(&self) -> u16 {
        self.rows
    }
    fn write(&mut self, data: &str) {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(data.as_bytes());
        let _ = lock.flush();
    }
}

/// resize 信号：一个可被信号处理 / resize 事件 set 的标志，渲染循环轮询后清除并重渲染。
#[derive(Clone, Default)]
pub struct ResizeFlag(Arc<AtomicBool>);

impl ResizeFlag {
    pub fn new() -> Self {
        Self::default()
    }
    /// 标记发生了 resize。
    pub fn signal(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    /// 若发生过 resize 则返回 true 并清除标志。
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }
}

/// crossterm 后端的真实终端：写到 stdout，尺寸来自 `crossterm::terminal::size()`。
///
/// 与 [`StdoutTerminal`] 的差别是它缓存了实际的终端尺寸（由事件循环在 resize 时刷新），并提供
/// raw-mode 的启停（[`RawModeGuard`]）。**仍渲染进 NORMAL buffer**（不进 alt-screen），让内容自然
/// 滚入 scrollback，对齐 PI / [`Tui`]。raw-mode / 尺寸查询用 crossterm，按键解码仍走本库的
/// [`super::stdin_buffer`] + [`super::keys`]（保真度高于 crossterm 的 key parser）。
pub struct CrosstermTerminal {
    columns: u16,
    rows: u16,
}

impl CrosstermTerminal {
    /// 查询当前终端尺寸构造（失败回退 80x24）。
    pub fn new() -> Self {
        let (columns, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        Self {
            columns: columns.max(1),
            rows: rows.max(1),
        }
    }

    /// 从终端重新查询尺寸（resize 后调用）。返回是否变化。
    pub fn refresh_size(&mut self) -> bool {
        if let Ok((columns, rows)) = crossterm::terminal::size() {
            let columns = columns.max(1);
            let rows = rows.max(1);
            if columns != self.columns || rows != self.rows {
                self.columns = columns;
                self.rows = rows;
                return true;
            }
        }
        false
    }

    /// 显式设置尺寸（测试 / 已知值时）。
    pub fn set_size(&mut self, columns: u16, rows: u16) {
        self.columns = columns.max(1);
        self.rows = rows.max(1);
    }
}

impl Default for CrosstermTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal for CrosstermTerminal {
    fn columns(&self) -> u16 {
        self.columns
    }
    fn rows(&self) -> u16 {
        self.rows
    }
    fn write(&mut self, data: &str) {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(data.as_bytes());
        let _ = lock.flush();
    }
}

/// RAII raw-mode 守卫：构造时 `enable_raw_mode` + 开启 bracketed paste，drop 时还原（即使 panic 也还原）。
///
/// 不进 alt-screen；仅做 raw I/O。drop 顺序：关 bracketed paste → 显示光标 → 关 raw mode → 还原 VT console 模式。
pub struct RawModeGuard {
    active: bool,
    // Windows：保存进入前的 console 模式，drop 时还原。其它平台无此字段。
    #[cfg(windows)]
    saved_in: Option<u32>,
    #[cfg(windows)]
    saved_out: Option<u32>,
}

impl RawModeGuard {
    /// 进入 raw mode。失败时返回错误且不改变终端状态。
    pub fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        // Windows：crossterm 的 enable_raw_mode 只清 line/echo/processed，不开 VT。本 TUI 自己
        // 读 stdin 原始字节并解码 VT 序列（见 stdin_buffer + keys），故必须显式开 VT 输入，否则
        // 方向键 / Enter / 控制键到达的字节与解析器预期不符（即「输入会错误」）。输出同时开 VT
        // 处理，让光标移动 / 清行 / bracketed paste 转义被解释。
        #[cfg(windows)]
        let (saved_in, saved_out) = enable_vt_console_modes();
        // 开启 bracketed paste，让粘贴整段到达（StdinBuffer 据此聚合）。
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(b"\x1b[?2004h");
        let _ = lock.flush();
        Ok(Self {
            active: true,
            #[cfg(windows)]
            saved_in,
            #[cfg(windows)]
            saved_out,
        })
    }
}

/// Windows：把 stdin 的 `ENABLE_VIRTUAL_TERMINAL_INPUT` 与 stdout 的
/// `ENABLE_VIRTUAL_TERMINAL_PROCESSING` 打开，返回还原用的旧模式（取不到句柄/模式则为 None）。
#[cfg(windows)]
fn enable_vt_console_modes() -> (Option<u32>, Option<u32>) {
    use windows::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    unsafe {
        let mut saved_in = None;
        if let Ok(h) = GetStdHandle(STD_INPUT_HANDLE) {
            let mut mode = CONSOLE_MODE(0);
            if GetConsoleMode(h, &mut mode).is_ok() {
                saved_in = Some(mode.0);
                let _ = SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_INPUT);
            }
        }
        let mut saved_out = None;
        if let Ok(h) = GetStdHandle(STD_OUTPUT_HANDLE) {
            let mut mode = CONSOLE_MODE(0);
            if GetConsoleMode(h, &mut mode).is_ok() {
                saved_out = Some(mode.0);
                let _ = SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
        (saved_in, saved_out)
    }
}

/// Windows：把保存的 console 模式还原回去（best-effort）。
#[cfg(windows)]
fn restore_vt_console_modes(saved_in: Option<u32>, saved_out: Option<u32>) {
    use windows::Win32::System::Console::{
        GetStdHandle, SetConsoleMode, CONSOLE_MODE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    unsafe {
        if let Some(m) = saved_in {
            if let Ok(h) = GetStdHandle(STD_INPUT_HANDLE) {
                let _ = SetConsoleMode(h, CONSOLE_MODE(m));
            }
        }
        if let Some(m) = saved_out {
            if let Ok(h) = GetStdHandle(STD_OUTPUT_HANDLE) {
                let _ = SetConsoleMode(h, CONSOLE_MODE(m));
            }
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let stdout = std::io::stdout();
        {
            let mut lock = stdout.lock();
            // 关 bracketed paste + 显示硬件光标。
            let _ = lock.write_all(b"\x1b[?2004l\x1b[?25h");
            let _ = lock.flush();
        }
        let _ = crossterm::terminal::disable_raw_mode();
        // Windows：还原进入前的 console 模式（VT 输入 / 输出处理）。
        #[cfg(windows)]
        restore_vt_console_modes(self.saved_in, self.saved_out);
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_terminal_accumulates() {
        let mut t = BufferTerminal::new(80, 24);
        assert_eq!(t.columns(), 80);
        assert_eq!(t.rows(), 24);
        t.write("hello");
        t.write(" world");
        assert_eq!(t.output(), "hello world");
        assert_eq!(t.take_output(), "hello world");
        assert_eq!(t.output(), "");
    }

    #[test]
    fn move_by_emits_relative() {
        let mut t = BufferTerminal::new(80, 24);
        t.move_by(3);
        assert_eq!(t.take_output(), "\x1b[3B");
        t.move_by(-2);
        assert_eq!(t.take_output(), "\x1b[2A");
        t.move_by(0);
        assert_eq!(t.take_output(), "");
    }

    #[test]
    fn cursor_and_clear_primitives() {
        let mut t = BufferTerminal::new(10, 5);
        t.hide_cursor();
        t.clear_line();
        t.show_cursor();
        assert_eq!(t.take_output(), "\x1b[?25l\x1b[2K\x1b[?25h");
    }

    #[test]
    fn set_size_changes_dims() {
        let mut t = BufferTerminal::new(80, 24);
        t.set_size(120, 40);
        assert_eq!(t.columns(), 120);
        assert_eq!(t.rows(), 40);
    }

    #[test]
    fn resize_flag_signals_once() {
        let f = ResizeFlag::new();
        assert!(!f.take());
        f.signal();
        assert!(f.take());
        assert!(!f.take());
    }

    #[test]
    fn crossterm_terminal_set_size_clamps_and_reports() {
        let mut t = CrosstermTerminal::new();
        t.set_size(120, 40);
        assert_eq!(t.columns(), 120);
        assert_eq!(t.rows(), 40);
        // zero clamps to 1 (never report a 0-width/height viewport).
        t.set_size(0, 0);
        assert_eq!(t.columns(), 1);
        assert_eq!(t.rows(), 1);
    }
}
