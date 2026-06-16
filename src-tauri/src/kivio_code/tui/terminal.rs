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
        Self { columns, rows, buffer: String::new() }
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
        let columns = std::env::var("COLUMNS").ok().and_then(|v| v.parse().ok()).unwrap_or(80);
        let rows = std::env::var("LINES").ok().and_then(|v| v.parse().ok()).unwrap_or(24);
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
}
