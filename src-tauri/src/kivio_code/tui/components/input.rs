//! Input —— PI `components/input.ts` 端口。
//!
//! 单行可编辑输入：`> ` 提示符、水平滚动（保持光标可见）、反显假光标 + [`CURSOR_MARKER`]（聚焦时，
//! 供 IME 定位）。完整 kill-ring + undo + 词导航 + bracketed paste（粘贴时剥换行）。
//!
//! 光标 `cursor` 是 value 的**字节偏移**，按 grapheme cluster 步进（emoji / combining 正确）。

use unicode_segmentation::UnicodeSegmentation;

use super::super::keybindings::KeybindingsManager;
use super::super::keys::decode_printable_key;
use super::super::kill_ring::KillRing;
use super::super::render::{Component, CURSOR_MARKER};
use super::super::text_width::{slice_by_column, visible_width};
use super::super::undo_stack::UndoStack;
use super::super::word_navigation::{find_word_backward, find_word_forward};

#[derive(Clone)]
struct InputState {
    value: String,
    cursor: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

/// 单行文本输入组件（实现 [`Component`]，并是 Focusable —— 用 `focused` 字段表达）。
pub struct Input {
    value: String,
    cursor: usize,
    /// 聚焦标志（Focusable）。TUI 在焦点变化时设置。
    pub focused: bool,

    paste_buffer: String,
    is_in_paste: bool,

    kill_ring: KillRing,
    last_action: Option<LastAction>,
    undo_stack: UndoStack<InputState>,

    kb: KeybindingsManager,
    kitty_active: bool,

    /// 提交回调（Enter）。
    pub on_submit: Option<Box<dyn FnMut(String) + Send>>,
    /// 取消回调（Esc）。
    pub on_escape: Option<Box<dyn FnMut() + Send>>,
}

impl Input {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            focused: false,
            paste_buffer: String::new(),
            is_in_paste: false,
            kill_ring: KillRing::new(),
            last_action: None,
            undo_stack: UndoStack::new(),
            kb: KeybindingsManager::with_defaults(),
            kitty_active: false,
            on_submit: None,
            on_escape: None,
        }
    }

    pub fn set_kitty_active(&mut self, active: bool) {
        self.kitty_active = active;
    }

    pub fn get_value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.cursor.min(self.value.len());
    }

    fn matches(&self, data: &str, id: &str) -> bool {
        self.kb.matches(data, id, self.kitty_active)
    }

    /// 取 `before` 末尾一个 grapheme 的字节长度。
    fn last_grapheme_len(s: &str) -> usize {
        s.graphemes(true).next_back().map(|g| g.len()).unwrap_or(1)
    }

    /// 取 `after` 起始一个 grapheme 的字节长度。
    fn first_grapheme_len(s: &str) -> usize {
        s.graphemes(true).next().map(|g| g.len()).unwrap_or(1)
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(&InputState {
            value: self.value.clone(),
            cursor: self.cursor,
        });
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo_stack.pop() {
            self.value = s.value;
            self.cursor = s.cursor;
            self.last_action = None;
        }
    }

    fn insert_character(&mut self, ch: &str) {
        let is_ws = ch.chars().all(|c| c.is_whitespace()) && !ch.is_empty();
        if is_ws || self.last_action != Some(LastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(LastAction::TypeWord);
        self.value.insert_str(self.cursor, ch);
        self.cursor += ch.len();
    }

    fn handle_backspace(&mut self) {
        self.last_action = None;
        if self.cursor > 0 {
            self.push_undo();
            let glen = Self::last_grapheme_len(&self.value[..self.cursor]);
            let start = self.cursor - glen;
            self.value.replace_range(start..self.cursor, "");
            self.cursor = start;
        }
    }

    fn handle_forward_delete(&mut self) {
        self.last_action = None;
        if self.cursor < self.value.len() {
            self.push_undo();
            let glen = Self::first_grapheme_len(&self.value[self.cursor..]);
            self.value
                .replace_range(self.cursor..self.cursor + glen, "");
        }
    }

    fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let deleted = self.value[..self.cursor].to_string();
        let acc = self.last_action == Some(LastAction::Kill);
        self.kill_ring.push(&deleted, true, acc);
        self.last_action = Some(LastAction::Kill);
        self.value = self.value[self.cursor..].to_string();
        self.cursor = 0;
    }

    fn delete_to_line_end(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.push_undo();
        let deleted = self.value[self.cursor..].to_string();
        let acc = self.last_action == Some(LastAction::Kill);
        self.kill_ring.push(&deleted, false, acc);
        self.last_action = Some(LastAction::Kill);
        self.value = self.value[..self.cursor].to_string();
    }

    fn delete_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let old_cursor = self.cursor;
        let delete_from = find_word_backward(&self.value, self.cursor, None);
        let deleted = self.value[delete_from..old_cursor].to_string();
        self.kill_ring.push(&deleted, true, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.value.replace_range(delete_from..old_cursor, "");
        self.cursor = delete_from;
    }

    fn delete_word_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let delete_to = find_word_forward(&self.value, self.cursor, None);
        let deleted = self.value[self.cursor..delete_to].to_string();
        self.kill_ring.push(&deleted, false, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.value.replace_range(self.cursor..delete_to, "");
    }

    fn yank(&mut self) {
        let Some(text) = self.kill_ring.peek().map(|s| s.to_string()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.push_undo();
        self.value.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();
        let prev = self
            .kill_ring
            .peek()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let start = self.cursor.saturating_sub(prev.len());
        self.value.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.kill_ring.rotate();
        let text = self
            .kill_ring
            .peek()
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.value.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    fn move_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.last_action = None;
        self.cursor = find_word_backward(&self.value, self.cursor, None);
    }

    fn move_word_forwards(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.last_action = None;
        self.cursor = find_word_forward(&self.value, self.cursor, None);
    }

    fn handle_paste(&mut self, pasted: &str) {
        self.last_action = None;
        self.push_undo();
        let clean: String = pasted
            .replace("\r\n", "")
            .replace('\r', "")
            .replace('\n', "")
            .replace('\t', "    ");
        self.value.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Input {
    fn handle_input(&mut self, data: &str) {
        let mut data = data.to_string();

        // bracketed paste
        if data.contains("\x1b[200~") {
            self.is_in_paste = true;
            self.paste_buffer.clear();
            data = data.replacen("\x1b[200~", "", 1);
        }
        if self.is_in_paste {
            self.paste_buffer.push_str(&data);
            if let Some(end) = self.paste_buffer.find("\x1b[201~") {
                let content = self.paste_buffer[..end].to_string();
                self.handle_paste(&content);
                self.is_in_paste = false;
                let remaining = self.paste_buffer[end + "\x1b[201~".len()..].to_string();
                self.paste_buffer.clear();
                if !remaining.is_empty() {
                    self.handle_input(&remaining);
                }
            }
            return;
        }

        if self.matches(&data, "tui.select.cancel") {
            if let Some(cb) = self.on_escape.as_mut() {
                cb();
            }
            return;
        }
        if self.matches(&data, "tui.editor.undo") {
            self.undo();
            return;
        }
        if self.matches(&data, "tui.input.submit") || data == "\n" {
            let value = self.value.clone();
            if let Some(cb) = self.on_submit.as_mut() {
                cb(value);
            }
            return;
        }
        if self.matches(&data, "tui.editor.deleteCharBackward") {
            self.handle_backspace();
            return;
        }
        if self.matches(&data, "tui.editor.deleteCharForward") {
            self.handle_forward_delete();
            return;
        }
        if self.matches(&data, "tui.editor.deleteWordBackward") {
            self.delete_word_backwards();
            return;
        }
        if self.matches(&data, "tui.editor.deleteWordForward") {
            self.delete_word_forward();
            return;
        }
        if self.matches(&data, "tui.editor.deleteToLineStart") {
            self.delete_to_line_start();
            return;
        }
        if self.matches(&data, "tui.editor.deleteToLineEnd") {
            self.delete_to_line_end();
            return;
        }
        if self.matches(&data, "tui.editor.yank") {
            self.yank();
            return;
        }
        if self.matches(&data, "tui.editor.yankPop") {
            self.yank_pop();
            return;
        }
        if self.matches(&data, "tui.editor.cursorLeft") {
            self.last_action = None;
            if self.cursor > 0 {
                self.cursor -= Self::last_grapheme_len(&self.value[..self.cursor]);
            }
            return;
        }
        if self.matches(&data, "tui.editor.cursorRight") {
            self.last_action = None;
            if self.cursor < self.value.len() {
                self.cursor += Self::first_grapheme_len(&self.value[self.cursor..]);
            }
            return;
        }
        if self.matches(&data, "tui.editor.cursorLineStart") {
            self.last_action = None;
            self.cursor = 0;
            return;
        }
        if self.matches(&data, "tui.editor.cursorLineEnd") {
            self.last_action = None;
            self.cursor = self.value.len();
            return;
        }
        if self.matches(&data, "tui.editor.cursorWordLeft") {
            self.move_word_backwards();
            return;
        }
        if self.matches(&data, "tui.editor.cursorWordRight") {
            self.move_word_forwards();
            return;
        }

        // Kitty CSI-u 可打印字符
        if let Some(ch) = decode_printable_key(&data) {
            self.insert_character(&ch.to_string());
            return;
        }

        // 普通可打印输入：拒绝控制字符
        let has_control = data.chars().any(|c| {
            let code = c as u32;
            code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
        });
        if !has_control {
            self.insert_character(&data);
        }
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let prompt = "> ";
        let prompt_w = prompt.len();
        if width <= prompt_w {
            return vec![prompt.to_string()];
        }
        let available = width - prompt_w;

        let total_w = visible_width(&self.value);
        let visible_text: String;
        let cursor_display: usize; // byte offset into visible_text

        if total_w < available {
            visible_text = self.value.clone();
            cursor_display = self.cursor;
        } else {
            let at_end = self.cursor == self.value.len();
            let scroll_width = if at_end { available - 1 } else { available };
            let cursor_col = visible_width(&self.value[..self.cursor]);
            if scroll_width > 0 {
                let half = scroll_width / 2;
                let start_col = if cursor_col < half {
                    0
                } else if cursor_col > total_w.saturating_sub(half) {
                    total_w.saturating_sub(scroll_width)
                } else {
                    cursor_col.saturating_sub(half)
                };
                visible_text = slice_by_column(&self.value, start_col, scroll_width, true);
                let before = slice_by_column(
                    &self.value,
                    start_col,
                    cursor_col.saturating_sub(start_col),
                    true,
                );
                cursor_display = before.len();
            } else {
                visible_text = String::new();
                cursor_display = 0;
            }
        }

        let before_cursor = &visible_text[..cursor_display.min(visible_text.len())];
        let after = &visible_text[cursor_display.min(visible_text.len())..];
        let at_cursor = after.graphemes(true).next().unwrap_or(" ");
        let after_cursor = &after[at_cursor.len().min(after.len())..];

        let marker = if self.focused { CURSOR_MARKER } else { "" };
        let cursor_char = format!("\x1b[7m{at_cursor}\x1b[27m");
        let text_with_cursor = format!("{before_cursor}{marker}{cursor_char}{after_cursor}");

        let visual_len = visible_width(&text_with_cursor);
        let padding = " ".repeat(available.saturating_sub(visual_len));
        vec![format!("{prompt}{text_with_cursor}{padding}")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(input: &mut Input, s: &str) {
        for ch in s.chars() {
            input.handle_input(&ch.to_string());
        }
    }

    #[test]
    fn insert_and_value() {
        let mut input = Input::new();
        type_str(&mut input, "hello");
        assert_eq!(input.get_value(), "hello");
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn backspace_deletes_grapheme() {
        let mut input = Input::new();
        type_str(&mut input, "ab");
        input.handle_input("\x7f"); // backspace
        assert_eq!(input.get_value(), "a");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn forward_delete() {
        let mut input = Input::new();
        type_str(&mut input, "abc");
        input.handle_input("\x01"); // ctrl+a -> line start
        input.handle_input("\x04"); // ctrl+d -> forward delete
        assert_eq!(input.get_value(), "bc");
    }

    #[test]
    fn cursor_movement() {
        let mut input = Input::new();
        type_str(&mut input, "abc");
        input.handle_input("\x1b[D"); // left
        assert_eq!(input.cursor, 2);
        input.handle_input("\x1b[C"); // right
        assert_eq!(input.cursor, 3);
        input.handle_input("\x01"); // ctrl+a -> start
        assert_eq!(input.cursor, 0);
        input.handle_input("\x05"); // ctrl+e -> end
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn word_kill_backward() {
        let mut input = Input::new();
        type_str(&mut input, "hello world");
        input.handle_input("\x17"); // ctrl+w -> delete word backward
        assert_eq!(input.get_value(), "hello ");
        // yank it back
        input.handle_input("\x19"); // ctrl+y
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn delete_to_line_end_and_yank() {
        let mut input = Input::new();
        type_str(&mut input, "hello world");
        input.handle_input("\x01"); // ctrl+a
        input.handle_input("\x0b"); // ctrl+k -> kill to end
        assert_eq!(input.get_value(), "");
        input.handle_input("\x19"); // ctrl+y
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn undo_restores() {
        let mut input = Input::new();
        type_str(&mut input, "abc");
        input.handle_input(" "); // space captures pre-space state "abc" then becomes "abc "
        type_str(&mut input, "def"); // coalesces with the space's undo unit
                                     // one undo removes the space+word unit back to "abc"
        input.handle_input("\x1f"); // ctrl+- undo
        assert_eq!(input.get_value(), "abc");
    }

    #[test]
    fn cursor_marker_emitted_when_focused() {
        let mut input = Input::new();
        input.focused = true;
        type_str(&mut input, "ab");
        let lines = input.render(40);
        assert!(lines[0].contains(CURSOR_MARKER));
    }

    #[test]
    fn cursor_marker_absent_when_unfocused() {
        let mut input = Input::new();
        type_str(&mut input, "ab");
        let lines = input.render(40);
        assert!(!lines[0].contains(CURSOR_MARKER));
    }

    #[test]
    fn submit_callback_fires() {
        use std::sync::{Arc, Mutex};
        let captured = Arc::new(Mutex::new(String::new()));
        let c2 = captured.clone();
        let mut input = Input::new();
        input.on_submit = Some(Box::new(move |v| *c2.lock().unwrap() = v));
        type_str(&mut input, "hi");
        input.handle_input("\r");
        assert_eq!(*captured.lock().unwrap(), "hi");
    }

    #[test]
    fn paste_strips_newlines() {
        let mut input = Input::new();
        input.handle_input("\x1b[200~foo\nbar\x1b[201~");
        assert_eq!(input.get_value(), "foobar");
    }

    #[test]
    fn render_horizontal_scroll_keeps_width() {
        let mut input = Input::new();
        input.focused = true;
        type_str(&mut input, &"x".repeat(100));
        let lines = input.render(20);
        // visible width of the single rendered line is <= terminal width
        assert!(visible_width(&lines[0]) <= 20);
    }
}
