//! 差分行渲染器 —— PI `tui.ts` 的核心端口（不含 overlay / Kitty 图片，留待后续阶段）。
//!
//! 模型：每个 [`Component`] 的 `render(width)` 返回 `Vec<String>`（每项 = 一终端行，可见列 ≤ width）。
//! [`Container`] 纵向拼接子组件。[`Tui`] 持有上一帧行数组，新帧来后 diff 行数组，只发出最小的
//! 相对光标移动 + `\x1b[2K` 重写改动行（`first_changed..last_changed`），全程包在 synchronized-output
//! （`\x1b[?2026h/l`）里。宽度变化 → 全量重绘（含清 scrollback）；高度变化 → 全量重绘。
//!
//! Focusable 组件在光标处 emit 零宽 APC 标记 [`CURSOR_MARKER`]，渲染器扫描底部 height 行找到并
//! 剥离，记录 {row,col} 用于定位硬件光标（IME 候选窗）。

use super::terminal::Terminal;
use super::text_width::{normalize_terminal_output, visible_width};

/// 光标位置标记：零宽 APC 序列，终端忽略。Focusable 组件在文本光标处 emit，渲染器找到后剥离
/// 并据此定位硬件光标。对应 PI 的 `CURSOR_MARKER`。
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// 行尾重置：SGR reset + OSC 8 超链接 reset。每个非图片行都追加它，确保样式不跨行渗透。
const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

/// 所有 UI 组件实现本 trait。`render` 返回每行一个 ANSI 字符串（可见列 ≤ width）。
pub trait Component {
    /// 渲染到给定视口宽度的行数组。
    fn render(&mut self, width: u16) -> Vec<String>;
    /// 聚焦时处理键盘输入（可选）。
    fn handle_input(&mut self, _data: &str) {}
    /// 是否接收 Kitty 释放事件（默认 false）。
    fn wants_key_release(&self) -> bool {
        false
    }
    /// 丢弃缓存（主题变更 / resize 时）。
    fn invalidate(&mut self) {}
}

/// 纵向拼接子组件的容器。
#[derive(Default)]
pub struct Container {
    pub children: Vec<Box<dyn Component>>,
}

impl Container {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_child(&mut self, c: Box<dyn Component>) {
        self.children.push(c);
    }
    pub fn clear(&mut self) {
        self.children.clear();
    }
}

impl Component for Container {
    fn render(&mut self, width: u16) -> Vec<String> {
        let mut lines = Vec::new();
        for child in &mut self.children {
            lines.extend(child.render(width));
        }
        lines
    }
    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.invalidate();
        }
    }
}

/// 扫描底部 `height` 行找 CURSOR_MARKER，记录 {row,col}（可见列）并从行中剥离。
fn extract_cursor_position(lines: &mut [String], height: usize) -> Option<(usize, usize)> {
    let viewport_top = lines.len().saturating_sub(height);
    for row in (viewport_top..lines.len()).rev() {
        if let Some(idx) = lines[row].find(CURSOR_MARKER) {
            let before = &lines[row][..idx];
            let col = visible_width(before);
            let mut new_line = String::with_capacity(lines[row].len() - CURSOR_MARKER.len());
            new_line.push_str(&lines[row][..idx]);
            new_line.push_str(&lines[row][idx + CURSOR_MARKER.len()..]);
            lines[row] = new_line;
            return Some((row, col));
        }
    }
    None
}

/// 给每个非空（非图片）行追加 SEGMENT_RESET，并做 Thai/Lao 规整。
fn apply_line_resets(lines: &mut [String]) {
    for line in lines.iter_mut() {
        *line = format!("{}{}", normalize_terminal_output(line), SEGMENT_RESET);
    }
}

/// 差分行渲染器。持有上一帧行数组 + 光标 / 视口簿记。
pub struct Tui<T: Terminal> {
    pub terminal: T,
    root: Container,
    previous_lines: Vec<String>,
    previous_width: u16,
    previous_height: u16,
    cursor_row: usize,
    hardware_cursor_row: usize,
    max_lines_rendered: usize,
    previous_viewport_top: usize,
    full_redraw_count: u32,
    stopped: bool,
    show_hardware_cursor: bool,
}

impl<T: Terminal> Tui<T> {
    pub fn new(terminal: T) -> Self {
        Self {
            terminal,
            root: Container::new(),
            previous_lines: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            cursor_row: 0,
            hardware_cursor_row: 0,
            max_lines_rendered: 0,
            previous_viewport_top: 0,
            full_redraw_count: 0,
            stopped: false,
            show_hardware_cursor: false,
        }
    }

    pub fn add_child(&mut self, c: Box<dyn Component>) {
        self.root.add_child(c);
    }

    pub fn clear_children(&mut self) {
        self.root.clear();
    }

    pub fn invalidate(&mut self) {
        self.root.invalidate();
    }

    /// 全量重绘次数（测试用）。
    pub fn full_redraws(&self) -> u32 {
        self.full_redraw_count
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.show_hardware_cursor = enabled;
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// 主渲染入口：渲染组件树，与上一帧 diff，写出最小转义输出。
    pub fn render(&mut self) {
        if self.stopped {
            return;
        }
        self.do_render();
    }

    fn position_hardware_cursor(&mut self, cursor_pos: Option<(usize, usize)>, total_lines: usize) {
        if !self.show_hardware_cursor {
            return;
        }
        let Some((row, col)) = cursor_pos else { return };
        // 从 hardware_cursor_row 相对移动到目标行，再绝对设置列。
        let line_diff = row as i64 - self.hardware_cursor_row as i64;
        let mut buf = String::new();
        match line_diff.cmp(&0) {
            std::cmp::Ordering::Greater => buf.push_str(&format!("\x1b[{line_diff}B")),
            std::cmp::Ordering::Less => buf.push_str(&format!("\x1b[{}A", -line_diff)),
            std::cmp::Ordering::Equal => {}
        }
        buf.push('\r');
        if col > 0 {
            buf.push_str(&format!("\x1b[{col}C"));
        }
        let _ = total_lines;
        self.terminal.write(&buf);
        self.hardware_cursor_row = row;
    }

    fn full_render(&mut self, clear: bool, new_lines: &[String], cursor_pos: Option<(usize, usize)>, width: u16, height: usize) {
        self.full_redraw_count += 1;
        let mut buffer = String::from("\x1b[?2026h"); // begin synchronized output
        if clear {
            buffer.push_str("\x1b[2J\x1b[H\x1b[3J"); // clear screen, home, clear scrollback
        }
        for (i, line) in new_lines.iter().enumerate() {
            if i > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str(line);
        }
        buffer.push_str("\x1b[?2026l"); // end synchronized output
        self.terminal.write(&buffer);

        self.cursor_row = new_lines.len().saturating_sub(1);
        self.hardware_cursor_row = self.cursor_row;
        if clear {
            self.max_lines_rendered = new_lines.len();
        } else {
            self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        }
        let buffer_len = height.max(new_lines.len());
        self.previous_viewport_top = buffer_len.saturating_sub(height);
        self.position_hardware_cursor(cursor_pos, new_lines.len());
        self.previous_lines = new_lines.to_vec();
        self.previous_width = width;
        self.previous_height = height as u16;
    }

    fn do_render(&mut self) {
        let width = self.terminal.columns();
        let height = self.terminal.rows() as usize;
        let width_changed = self.previous_width != 0 && self.previous_width != width;
        let height_changed = self.previous_height != 0 && self.previous_height as usize != height;

        let previous_buffer_len = if self.previous_height > 0 {
            self.previous_viewport_top + self.previous_height as usize
        } else {
            height
        };
        let mut prev_viewport_top =
            if height_changed { previous_buffer_len.saturating_sub(height) } else { self.previous_viewport_top };
        let mut viewport_top = prev_viewport_top;
        let mut hardware_cursor_row = self.hardware_cursor_row;

        // 渲染组件树
        let mut new_lines = self.root.render(width);
        let cursor_pos = extract_cursor_position(&mut new_lines, height);
        apply_line_resets(&mut new_lines);

        // 首帧：直接全量输出，不清屏（假设屏幕干净）
        if self.previous_lines.is_empty() && !width_changed && !height_changed {
            self.full_render(false, &new_lines, cursor_pos, width, height);
            return;
        }
        // 宽度变化：wrap 改变 → 全量重绘（清 scrollback）
        if width_changed {
            self.full_render(true, &new_lines, cursor_pos, width, height);
            return;
        }
        // 高度变化：对齐视口 → 全量重绘
        if height_changed {
            self.full_render(true, &new_lines, cursor_pos, width, height);
            return;
        }

        // 找首个 / 末个改动行
        let mut first_changed: i64 = -1;
        let mut last_changed: i64 = -1;
        let max_lines = new_lines.len().max(self.previous_lines.len());
        for i in 0..max_lines {
            let old_line = self.previous_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            let new_line = new_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            if old_line != new_line {
                if first_changed == -1 {
                    first_changed = i as i64;
                }
                last_changed = i as i64;
            }
        }
        let appended_lines = new_lines.len() > self.previous_lines.len();
        if appended_lines {
            if first_changed == -1 {
                first_changed = self.previous_lines.len() as i64;
            }
            last_changed = new_lines.len() as i64 - 1;
        }
        let append_start =
            appended_lines && first_changed == self.previous_lines.len() as i64 && first_changed > 0;

        // 无变化：仅更新硬件光标
        if first_changed == -1 {
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_viewport_top = prev_viewport_top;
            self.previous_height = height as u16;
            return;
        }

        // 所有变化都在被删除的行里（仅需清除）
        if first_changed >= new_lines.len() as i64 {
            if self.previous_lines.len() > new_lines.len() {
                let target_row = new_lines.len().saturating_sub(1);
                if (target_row as i64) < prev_viewport_top as i64 {
                    self.full_render(true, &new_lines, cursor_pos, width, height);
                    return;
                }
                let mut buffer = String::from("\x1b[?2026h");
                let line_diff = (target_row as i64 - viewport_top as i64)
                    - (hardware_cursor_row as i64 - prev_viewport_top as i64);
                match line_diff.cmp(&0) {
                    std::cmp::Ordering::Greater => buffer.push_str(&format!("\x1b[{line_diff}B")),
                    std::cmp::Ordering::Less => buffer.push_str(&format!("\x1b[{}A", -line_diff)),
                    std::cmp::Ordering::Equal => {}
                }
                buffer.push('\r');
                let extra_lines = self.previous_lines.len() - new_lines.len();
                if extra_lines > height {
                    self.full_render(true, &new_lines, cursor_pos, width, height);
                    return;
                }
                let clear_start_offset = if new_lines.is_empty() { 0 } else { 1 };
                if extra_lines > 0 && clear_start_offset > 0 {
                    buffer.push_str(&format!("\x1b[{clear_start_offset}B"));
                }
                for i in 0..extra_lines {
                    buffer.push_str("\r\x1b[2K");
                    if i < extra_lines - 1 {
                        buffer.push_str("\x1b[1B");
                    }
                }
                let move_back = (extra_lines as i64 - 1 + clear_start_offset).max(0);
                if move_back > 0 {
                    buffer.push_str(&format!("\x1b[{move_back}A"));
                }
                buffer.push_str("\x1b[?2026l");
                self.terminal.write(&buffer);
                self.cursor_row = target_row;
                self.hardware_cursor_row = target_row;
            }
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_width = width;
            self.previous_height = height as u16;
            self.previous_viewport_top = prev_viewport_top;
            return;
        }

        // 首个改动行在上一视口之上 —— 无法差分，全量重绘
        if (first_changed as usize) < prev_viewport_top {
            self.full_render(true, &new_lines, cursor_pos, width, height);
            return;
        }

        // 差分输出
        let mut buffer = String::from("\x1b[?2026h");
        let prev_viewport_bottom = prev_viewport_top + height - 1;
        let move_target_row = if append_start { (first_changed - 1) as usize } else { first_changed as usize };
        if move_target_row > prev_viewport_bottom {
            let current_screen_row =
                ((hardware_cursor_row as i64 - prev_viewport_top as i64).clamp(0, height as i64 - 1)) as usize;
            let move_to_bottom = height - 1 - current_screen_row;
            if move_to_bottom > 0 {
                buffer.push_str(&format!("\x1b[{move_to_bottom}B"));
            }
            let scroll = move_target_row - prev_viewport_bottom;
            for _ in 0..scroll {
                buffer.push_str("\r\n");
            }
            prev_viewport_top += scroll;
            viewport_top += scroll;
            hardware_cursor_row = move_target_row;
        }

        // 移动到首个改动行
        let line_diff = (move_target_row as i64 - viewport_top as i64)
            - (hardware_cursor_row as i64 - prev_viewport_top as i64);
        match line_diff.cmp(&0) {
            std::cmp::Ordering::Greater => buffer.push_str(&format!("\x1b[{line_diff}B")),
            std::cmp::Ordering::Less => buffer.push_str(&format!("\x1b[{}A", -line_diff)),
            std::cmp::Ordering::Equal => {}
        }
        buffer.push_str(if append_start { "\r\n" } else { "\r" });

        // 只重写改动范围
        let render_end = (last_changed as usize).min(new_lines.len() - 1);
        for i in (first_changed as usize)..=render_end {
            if i > first_changed as usize {
                buffer.push_str("\r\n");
            }
            let line = &new_lines[i];
            buffer.push_str("\x1b[2K"); // clear current line
            // 宽度溢出守卫：差分模型会被超宽行破坏，直接 panic（PI 在此 throw）。
            debug_assert!(
                visible_width(line) <= width as usize,
                "rendered line {i} exceeds terminal width ({} > {width})",
                visible_width(line)
            );
            buffer.push_str(line);
        }

        let mut final_cursor_row = render_end;
        // 之前更多行 —— 清除多余尾行
        if self.previous_lines.len() > new_lines.len() {
            if render_end < new_lines.len() - 1 {
                let move_down = new_lines.len() - 1 - render_end;
                buffer.push_str(&format!("\x1b[{move_down}B"));
                final_cursor_row = new_lines.len() - 1;
            }
            let extra_lines = self.previous_lines.len() - new_lines.len();
            for _ in 0..extra_lines {
                buffer.push_str("\r\n\x1b[2K");
            }
            buffer.push_str(&format!("\x1b[{extra_lines}A"));
        }

        buffer.push_str("\x1b[?2026l");
        self.terminal.write(&buffer);

        self.cursor_row = new_lines.len().saturating_sub(1);
        self.hardware_cursor_row = final_cursor_row;
        self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        self.previous_viewport_top =
            prev_viewport_top.max((final_cursor_row as i64 - height as i64 + 1).max(0) as usize);
        self.position_hardware_cursor(cursor_pos, new_lines.len());
        self.previous_lines = new_lines;
        self.previous_width = width;
        self.previous_height = height as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::super::terminal::BufferTerminal;
    use super::*;

    /// 一个返回固定行的测试组件。
    struct Fixed(Vec<String>);
    impl Component for Fixed {
        fn render(&mut self, _width: u16) -> Vec<String> {
            self.0.clone()
        }
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cursor_marker_extracted_and_stripped() {
        let mut ls = lines(&["abc", &format!("de{CURSOR_MARKER}f")]);
        let pos = extract_cursor_position(&mut ls, 24);
        assert_eq!(pos, Some((1, 2)));
        assert_eq!(ls[1], "def");
    }

    #[test]
    fn apply_resets_appends_segment_reset() {
        let mut ls = lines(&["x"]);
        apply_line_resets(&mut ls);
        assert_eq!(ls[0], format!("x{SEGMENT_RESET}"));
    }

    #[test]
    fn container_concatenates() {
        let mut c = Container::new();
        c.add_child(Box::new(Fixed(lines(&["a", "b"]))));
        c.add_child(Box::new(Fixed(lines(&["c"]))));
        assert_eq!(c.render(80), lines(&["a", "b", "c"]));
    }

    #[test]
    fn first_render_no_clear() {
        let mut tui = Tui::new(BufferTerminal::new(80, 24));
        tui.add_child(Box::new(Fixed(lines(&["hello", "world"]))));
        tui.render();
        let out = tui.terminal.take_output();
        // synchronized output wrapping, no scrollback clear, lines joined by \r\n
        assert!(out.starts_with("\x1b[?2026h"));
        assert!(out.ends_with("\x1b[?2026l"));
        assert!(!out.contains("\x1b[3J")); // no scrollback clear on first render
        assert!(out.contains("hello"));
        assert!(out.contains("\r\nworld"));
        assert_eq!(tui.full_redraws(), 1);
    }

    #[test]
    fn single_line_change_is_minimal() {
        // First frame
        let mut tui = Tui::new(BufferTerminal::new(80, 5));
        tui.add_child(Box::new(LineSource::new(lines(&["aaa", "bbb", "ccc"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        // Change only the middle line
        tui.set_lines(lines(&["aaa", "XXX", "ccc"]));
        tui.render();
        let out = tui.terminal.take_output();
        // Must be wrapped in synchronized output
        assert!(out.starts_with("\x1b[?2026h"));
        assert!(out.ends_with("\x1b[?2026l"));
        // Should NOT be a full redraw (no scrollback clear)
        assert!(!out.contains("\x1b[3J"));
        assert_eq!(tui.full_redraws(), 1); // only the first frame was a full redraw
        // After a full render the hardware cursor sits on the last line (row 2);
        // rewriting row 1 means moving UP 1 line, then clear + write XXX.
        assert!(out.contains("\x1b[1A"));
        assert!(out.contains("\x1b[2KXXX"));
        // Must NOT rewrite the unchanged lines aaa/ccc
        assert!(!out.contains("aaa"));
        assert!(!out.contains("ccc"));
    }

    #[test]
    fn width_change_full_redraw() {
        let mut tui = Tui::new(BufferTerminal::new(80, 5));
        tui.add_child(Box::new(LineSource::new(lines(&["aaa"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        // Change width -> must full redraw with scrollback clear
        tui.terminal.set_size(60, 5);
        tui.render();
        let out = tui.terminal.take_output();
        assert!(out.contains("\x1b[2J\x1b[H\x1b[3J"));
        assert_eq!(tui.full_redraws(), 2);
    }

    #[test]
    fn height_change_full_redraw() {
        let mut tui = Tui::new(BufferTerminal::new(80, 5));
        tui.add_child(Box::new(LineSource::new(lines(&["aaa"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        tui.terminal.set_size(80, 8);
        tui.render();
        let out = tui.terminal.take_output();
        assert!(out.contains("\x1b[3J"));
        assert_eq!(tui.full_redraws(), 2);
    }

    #[test]
    fn no_change_emits_nothing() {
        let mut tui = Tui::new(BufferTerminal::new(80, 5));
        tui.add_child(Box::new(LineSource::new(lines(&["aaa", "bbb"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        tui.render(); // identical frame
        let out = tui.terminal.take_output();
        assert_eq!(out, "");
    }

    #[test]
    fn appended_lines_diff() {
        let mut tui = Tui::new(BufferTerminal::new(80, 10));
        tui.add_child(Box::new(LineSource::new(lines(&["a", "b"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        tui.set_lines(lines(&["a", "b", "c"]));
        tui.render();
        let out = tui.terminal.take_output();
        assert!(out.starts_with("\x1b[?2026h"));
        assert!(out.contains("c"));
        // unchanged "a"/"b" not rewritten
        assert_eq!(out.matches("\x1b[2K").count(), 1);
    }

    // 一个测试组件，渲染固定行。Tui 便捷方法 set_lines 通过重建子组件改内容。
    struct LineSource {
        lines: Vec<String>,
    }
    impl LineSource {
        fn new(initial: Vec<String>) -> Self {
            Self { lines: initial }
        }
    }
    impl Component for LineSource {
        fn render(&mut self, _width: u16) -> Vec<String> {
            self.lines.clone()
        }
    }

    // 给 Tui 加测试便捷方法：替换唯一子组件的行内容（不影响 Tui 的 previous_lines 簿记）。
    impl Tui<BufferTerminal> {
        fn set_lines(&mut self, new: Vec<String>) {
            self.root.clear();
            self.root.add_child(Box::new(LineSource::new(new)));
        }
    }
}
