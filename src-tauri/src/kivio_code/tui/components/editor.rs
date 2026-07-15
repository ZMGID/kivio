//! Editor —— PI `components/editor.ts` 端口（多行编辑核心）。
//!
//! 实现 [`Component`] + Focusable（`focused` 字段）。状态 `{lines, cursor_line, cursor_col}`，
//! `cursor_col` 是当前逻辑行内的**字节偏移**。
//!
//! 覆盖：word-wrap 布局（`word_wrap_line` → [`TextChunk`]）、视觉行映射 + 垂直光标（sticky 首选列）、
//! 字符/词/行首尾/页/jump-to-char 移动、kill-ring、undo（词输入合并）、prompt 历史、bracketed paste
//! +大粘贴 marker（原子分段）、上下边框 + 滚动指示、聚焦时反显假光标 + [`CURSOR_MARKER`]。
//!
//! **wrapping + 光标如何对齐**：`word_wrap_line` 把一条逻辑行切成若干 `TextChunk{text,start,end}`
//! （字节区间），在空白后 / CJK 相邻处断行，长 token 强断。`layout_text` 把每条逻辑行展开成
//! `LayoutLine{text,has_cursor,cursor_pos}`：当某 chunk 的字节区间包含 `cursor_col` 时标记
//! has_cursor 并记录 chunk 内相对偏移；渲染时在该偏移插反显光标 + marker。垂直移动靠
//! `build_visual_line_map`（同 wrapping 规则）把 (line,col) 映射到视觉行，再用 sticky 首选列在
//! 目标视觉行解析落点（`compute_vertical_move_column` 复刻 PI 的决策表）。

use unicode_segmentation::UnicodeSegmentation;

use super::super::keybindings::KeybindingsManager;
use super::super::keys::{decode_printable_key, matches_key};
use super::super::kill_ring::KillRing;
use super::super::render::{Component, CURSOR_MARKER};
use super::super::text_width::{truncate_to_width, visible_width};
use super::super::undo_stack::UndoStack;
use super::super::word_navigation::{find_word_backward, find_word_forward, IsAtomic};
use super::select_list::SelectListTheme;
use super::ColorFn;

// =============================================================================
// paste marker helpers
// =============================================================================

/// 是否一个 paste marker 段（形如 `[paste #1 +123 lines]` / `[paste #2 1234 chars]`）。
fn is_paste_marker(seg: &str) -> bool {
    if seg.len() < 10 || !seg.starts_with("[paste #") || !seg.ends_with(']') {
        return false;
    }
    let inner = &seg[8..seg.len() - 1];
    // "<id>" 或 "<id> +N lines" 或 "<id> N chars"
    let mut parts = inner.splitn(2, ' ');
    let id = parts.next().unwrap_or("");
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(rest) => {
            (rest.starts_with('+')
                && rest.ends_with(" lines")
                && rest[1..rest.len() - 6].chars().all(|c| c.is_ascii_digit()))
                || (rest.ends_with(" chars")
                    && rest[..rest.len() - 6].chars().all(|c| c.is_ascii_digit()))
        }
    }
}

/// 一个简单 CJK 判定（允许相邻断行）。
fn is_cjk_grapheme(seg: &str) -> bool {
    seg.chars().next().is_some_and(|c| {
        let cp = c as u32;
        (0x3000..=0x303f).contains(&cp)
            || (0x3040..=0x30ff).contains(&cp)
            || (0x3100..=0x312f).contains(&cp)
            || (0x3400..=0x4dbf).contains(&cp)
            || (0x4e00..=0x9fff).contains(&cp)
            || (0xac00..=0xd7af).contains(&cp)
            || (0xf900..=0xfaff).contains(&cp)
            || (0xff00..=0xffef).contains(&cp)
    })
}

fn is_whitespace_grapheme(seg: &str) -> bool {
    !seg.is_empty() && seg.chars().all(|c| c.is_whitespace())
}

// =============================================================================
// word wrap
// =============================================================================

/// word-wrap 后的一段（字节区间）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    pub text: String,
    pub start_index: usize,
    pub end_index: usize,
}

/// 把一条逻辑行（无内嵌换行）按 `max_width` 可见列切成 chunk。`pre_segmented` 提供（byte_index, grapheme）
/// 列表（含 paste-marker 合并）；省略时用默认 grapheme 切分。
pub fn word_wrap_line(
    line: &str,
    max_width: usize,
    pre_segmented: Option<&[(usize, String)]>,
) -> Vec<TextChunk> {
    if line.is_empty() || max_width == 0 {
        return vec![TextChunk {
            text: String::new(),
            start_index: 0,
            end_index: 0,
        }];
    }
    if visible_width(line) <= max_width {
        return vec![TextChunk {
            text: line.to_string(),
            start_index: 0,
            end_index: line.len(),
        }];
    }

    let owned: Vec<(usize, String)>;
    let segments: &[(usize, String)] = match pre_segmented {
        Some(s) => s,
        None => {
            owned = line
                .grapheme_indices(true)
                .map(|(i, g)| (i, g.to_string()))
                .collect();
            &owned
        }
    };

    let mut chunks: Vec<TextChunk> = Vec::new();
    let mut current_width = 0usize;
    let mut chunk_start = 0usize;
    let mut wrap_opp_index: i64 = -1;
    let mut wrap_opp_width = 0usize;

    for i in 0..segments.len() {
        let (char_index, ref grapheme) = segments[i];
        let g_width = visible_width(grapheme);
        let is_ws = !is_paste_marker(grapheme) && is_whitespace_grapheme(grapheme);

        if current_width + g_width > max_width {
            if wrap_opp_index >= 0 && current_width - wrap_opp_width + g_width <= max_width {
                let wo = wrap_opp_index as usize;
                chunks.push(TextChunk {
                    text: line[chunk_start..wo].to_string(),
                    start_index: chunk_start,
                    end_index: wo,
                });
                chunk_start = wo;
                current_width -= wrap_opp_width;
            } else if chunk_start < char_index {
                chunks.push(TextChunk {
                    text: line[chunk_start..char_index].to_string(),
                    start_index: chunk_start,
                    end_index: char_index,
                });
                chunk_start = char_index;
                current_width = 0;
            }
            wrap_opp_index = -1;
        }

        if g_width > max_width {
            // 单段比 max_width 还宽：在 grapheme 粒度重新切（视觉切，逻辑仍原子）
            let sub = word_wrap_line(grapheme, max_width, None);
            for sc in &sub[..sub.len() - 1] {
                chunks.push(TextChunk {
                    text: sc.text.clone(),
                    start_index: char_index + sc.start_index,
                    end_index: char_index + sc.end_index,
                });
            }
            let last = &sub[sub.len() - 1];
            chunk_start = char_index + last.start_index;
            current_width = visible_width(&last.text);
            wrap_opp_index = -1;
            continue;
        }

        current_width += g_width;

        // 记录断行机会
        let next = segments.get(i + 1);
        if is_ws {
            if let Some((next_idx, next_seg)) = next {
                if is_paste_marker(next_seg) || !is_whitespace_grapheme(next_seg) {
                    wrap_opp_index = *next_idx as i64;
                    wrap_opp_width = current_width;
                }
            }
        } else if let Some((next_idx, next_seg)) = next {
            if !is_whitespace_grapheme(next_seg) {
                let is_cjk = !is_paste_marker(grapheme) && is_cjk_grapheme(grapheme);
                let next_is_cjk = !is_paste_marker(next_seg) && is_cjk_grapheme(next_seg);
                if is_cjk || next_is_cjk {
                    wrap_opp_index = *next_idx as i64;
                    wrap_opp_width = current_width;
                }
            }
        }
    }

    chunks.push(TextChunk {
        text: line[chunk_start..].to_string(),
        start_index: chunk_start,
        end_index: line.len(),
    });
    chunks
}

// =============================================================================
// editor
// =============================================================================

#[derive(Clone)]
struct EditorState {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
}

struct LayoutLine {
    text: String,
    has_cursor: bool,
    cursor_pos: Option<usize>,
}

#[derive(Clone, Copy)]
struct VisualLine {
    logical_line: usize,
    start_col: usize,
    length: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

#[derive(Clone, Copy, PartialEq)]
enum JumpMode {
    Forward,
    Backward,
}

/// Editor 主题（边框色 + select-list 主题用于补全下拉）。
pub struct EditorTheme {
    pub border_color: ColorFn,
    pub select_list: SelectListTheme,
}

/// 多行编辑组件。
pub struct Editor {
    state: EditorState,
    pub focused: bool,
    border_color: ColorFn,
    padding_x: usize,
    last_width: usize,
    scroll_offset: usize,
    terminal_rows: usize,

    pastes: std::collections::HashMap<u64, String>,
    paste_counter: u64,
    paste_buffer: String,
    is_in_paste: bool,

    history: Vec<String>,
    history_index: i64,
    history_draft: Option<EditorState>,

    kill_ring: KillRing,
    last_action: Option<LastAction>,
    jump_mode: Option<JumpMode>,
    preferred_visual_col: Option<usize>,
    snapped_from_cursor_col: Option<usize>,
    undo_stack: UndoStack<EditorState>,

    kb: KeybindingsManager,
    kitty_active: bool,

    pub disable_submit: bool,
    pub on_submit: Option<Box<dyn FnMut(String) + Send>>,
    pub on_change: Option<Box<dyn FnMut(String) + Send>>,
}

impl Editor {
    pub fn new(theme: EditorTheme) -> Self {
        Self {
            state: EditorState {
                lines: vec![String::new()],
                cursor_line: 0,
                cursor_col: 0,
            },
            focused: false,
            border_color: theme.border_color,
            padding_x: 0,
            last_width: 80,
            scroll_offset: 0,
            terminal_rows: 24,
            pastes: std::collections::HashMap::new(),
            paste_counter: 0,
            paste_buffer: String::new(),
            is_in_paste: false,
            history: Vec::new(),
            history_index: -1,
            history_draft: None,
            kill_ring: KillRing::new(),
            last_action: None,
            jump_mode: None,
            preferred_visual_col: None,
            snapped_from_cursor_col: None,
            undo_stack: UndoStack::new(),
            kb: KeybindingsManager::with_defaults(),
            kitty_active: false,
            disable_submit: false,
            on_submit: None,
            on_change: None,
        }
    }

    pub fn set_kitty_active(&mut self, active: bool) {
        self.kitty_active = active;
    }

    /// 设置终端高度（决定可见行数 / 页大小）。app 层在 resize 时调用。
    pub fn set_terminal_rows(&mut self, rows: usize) {
        self.terminal_rows = rows.max(1);
    }

    pub fn set_padding_x(&mut self, padding: usize) {
        self.padding_x = padding;
    }

    pub fn get_text(&self) -> String {
        self.state.lines.join("\n")
    }

    #[cfg(test)]
    pub fn get_cursor(&self) -> (usize, usize) {
        (self.state.cursor_line, self.state.cursor_col)
    }

    pub fn add_to_history(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.history.first().map(|s| s.as_str()) == Some(trimmed) {
            return;
        }
        self.history.insert(0, trimmed.to_string());
        if self.history.len() > 100 {
            self.history.pop();
        }
    }

    fn matches(&self, data: &str, id: &str) -> bool {
        self.kb.matches(data, id, self.kitty_active)
    }

    fn fire_change(&mut self) {
        if let Some(cb) = self.on_change.as_mut() {
            let text = self.state.lines.join("\n");
            cb(text);
        }
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(&self.state);
    }

    fn valid_paste_ids(&self) -> std::collections::HashSet<u64> {
        self.pastes.keys().copied().collect()
    }

    /// 带 paste-marker 感知的 grapheme 切分：合并有效 marker 为单个原子段。
    fn segment_graphemes(&self, text: &str) -> Vec<(usize, String)> {
        let ids = self.valid_paste_ids();
        if ids.is_empty() || !text.contains("[paste #") {
            return text
                .grapheme_indices(true)
                .map(|(i, g)| (i, g.to_string()))
                .collect();
        }
        // 找出所有有效 marker 的字节区间
        let markers = self.find_markers(text, &ids);
        if markers.is_empty() {
            return text
                .grapheme_indices(true)
                .map(|(i, g)| (i, g.to_string()))
                .collect();
        }
        let mut result: Vec<(usize, String)> = Vec::new();
        let mut marker_idx = 0;
        for (idx, g) in text.grapheme_indices(true) {
            while marker_idx < markers.len() && markers[marker_idx].1 <= idx {
                marker_idx += 1;
            }
            if let Some(&(mstart, mend)) = markers.get(marker_idx) {
                if idx >= mstart && idx < mend {
                    if idx == mstart {
                        result.push((mstart, text[mstart..mend].to_string()));
                    }
                    continue;
                }
            }
            result.push((idx, g.to_string()));
        }
        result
    }

    fn find_markers(
        &self,
        text: &str,
        ids: &std::collections::HashSet<u64>,
    ) -> Vec<(usize, usize)> {
        let mut markers: Vec<(usize, usize)> = Vec::new();
        let bytes = text.as_bytes();
        let needle = b"[paste #";
        let mut i = 0;
        while i + needle.len() <= bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                // 找到匹配的 ']'
                if let Some(close) = text[i..].find(']') {
                    let end = i + close + 1;
                    let seg = &text[i..end];
                    if is_paste_marker(seg) {
                        // 提取 id
                        if let Some(id) = seg[8..]
                            .split([' ', ']'])
                            .next()
                            .and_then(|s| s.parse::<u64>().ok())
                        {
                            if ids.contains(&id) {
                                markers.push((i, end));
                                i = end;
                                continue;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        markers
    }

    fn marker_is_atomic_fn() -> IsAtomic<'static> {
        &|s: &str| is_paste_marker(s)
    }

    fn last_grapheme_len(&self, before: &str) -> usize {
        // 用 paste-marker 感知的切分
        let segs = self.segment_graphemes(before);
        segs.last().map(|(_, g)| g.len()).unwrap_or(1)
    }

    fn first_grapheme_len(&self, after: &str) -> usize {
        let segs = self.segment_graphemes(after);
        segs.first().map(|(_, g)| g.len()).unwrap_or(1)
    }

    fn set_cursor_col(&mut self, col: usize) {
        self.state.cursor_col = col;
        self.preferred_visual_col = None;
        self.snapped_from_cursor_col = None;
    }

    fn exit_history_browsing(&mut self) {
        self.history_index = -1;
        self.history_draft = None;
    }

    fn normalize_text(text: &str) -> String {
        text.replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ")
    }

    pub fn set_text(&mut self, text: &str) {
        self.last_action = None;
        self.exit_history_browsing();
        let normalized = Self::normalize_text(text);
        if self.get_text() != normalized {
            self.push_undo();
        }
        self.set_text_internal(&normalized, false);
    }

    /// Insert plain text at the cursor (no newlines expected). Used for programmatic
    /// inserts such as the `[Image #N]` attachment placeholder. Pushes one undo step
    /// and advances the cursor past the inserted text.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.exit_history_browsing();
        self.push_undo();
        self.last_action = None;
        let line = &mut self.state.lines[self.state.cursor_line];
        line.insert_str(self.state.cursor_col, text);
        self.state.cursor_col += text.len();
        self.preferred_visual_col = None;
        self.snapped_from_cursor_col = None;
        self.fire_change();
    }

    fn set_text_internal(&mut self, text: &str, cursor_at_start: bool) {
        let lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        self.state.lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        self.state.cursor_line = if cursor_at_start {
            0
        } else {
            self.state.lines.len() - 1
        };
        let col = if cursor_at_start {
            0
        } else {
            self.state.lines[self.state.cursor_line].len()
        };
        self.set_cursor_col(col);
        self.scroll_offset = 0;
        self.fire_change();
    }

    fn insert_character(&mut self, ch: &str) {
        self.exit_history_browsing();
        let is_ws = is_whitespace_grapheme(ch);
        if is_ws || self.last_action != Some(LastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(LastAction::TypeWord);
        let line = &mut self.state.lines[self.state.cursor_line];
        line.insert_str(self.state.cursor_col, ch);
        self.state.cursor_col += ch.len();
        self.preferred_visual_col = None;
        self.snapped_from_cursor_col = None;
        self.fire_change();
    }

    fn add_new_line(&mut self) {
        self.exit_history_browsing();
        self.last_action = None;
        self.push_undo();
        let line = self.state.lines[self.state.cursor_line].clone();
        let before = line[..self.state.cursor_col].to_string();
        let after = line[self.state.cursor_col..].to_string();
        self.state.lines[self.state.cursor_line] = before;
        self.state.lines.insert(self.state.cursor_line + 1, after);
        self.state.cursor_line += 1;
        self.set_cursor_col(0);
        self.fire_change();
    }

    fn submit_value(&mut self) {
        let result = self
            .expand_paste_markers(&self.state.lines.join("\n"))
            .trim()
            .to_string();
        self.state = EditorState {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
        };
        self.pastes.clear();
        self.paste_counter = 0;
        self.exit_history_browsing();
        self.scroll_offset = 0;
        self.undo_stack.clear();
        self.last_action = None;
        if let Some(cb) = self.on_change.as_mut() {
            cb(String::new());
        }
        if let Some(cb) = self.on_submit.as_mut() {
            cb(result);
        }
    }

    fn expand_paste_markers(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (id, content) in &self.pastes {
            let ids = std::iter::once(*id).collect();
            // 找 marker 并替换
            loop {
                let markers = self.find_markers(&result, &ids);
                let Some(&(start, end)) = markers.first() else {
                    break;
                };
                result.replace_range(start..end, content);
            }
        }
        result
    }

    pub fn get_expanded_text(&self) -> String {
        self.expand_paste_markers(&self.state.lines.join("\n"))
    }

    fn handle_backspace(&mut self) {
        self.exit_history_browsing();
        self.last_action = None;
        if self.state.cursor_col > 0 {
            self.push_undo();
            let line = self.state.lines[self.state.cursor_line].clone();
            let before = &line[..self.state.cursor_col];
            let glen = self.last_grapheme_len(before);
            let new_line = format!(
                "{}{}",
                &line[..self.state.cursor_col - glen],
                &line[self.state.cursor_col..]
            );
            self.state.lines[self.state.cursor_line] = new_line;
            self.set_cursor_col(self.state.cursor_col - glen);
        } else if self.state.cursor_line > 0 {
            self.push_undo();
            let current = self.state.lines[self.state.cursor_line].clone();
            let prev = self.state.lines[self.state.cursor_line - 1].clone();
            self.state.lines[self.state.cursor_line - 1] = format!("{prev}{current}");
            self.state.lines.remove(self.state.cursor_line);
            self.state.cursor_line -= 1;
            self.set_cursor_col(prev.len());
        }
        self.fire_change();
    }

    fn handle_forward_delete(&mut self) {
        self.exit_history_browsing();
        self.last_action = None;
        let line = self.state.lines[self.state.cursor_line].clone();
        if self.state.cursor_col < line.len() {
            self.push_undo();
            let glen = self.first_grapheme_len(&line[self.state.cursor_col..]);
            let new_line = format!(
                "{}{}",
                &line[..self.state.cursor_col],
                &line[self.state.cursor_col + glen..]
            );
            self.state.lines[self.state.cursor_line] = new_line;
        } else if self.state.cursor_line < self.state.lines.len() - 1 {
            self.push_undo();
            let next = self.state.lines[self.state.cursor_line + 1].clone();
            self.state.lines[self.state.cursor_line] = format!("{line}{next}");
            self.state.lines.remove(self.state.cursor_line + 1);
        }
        self.fire_change();
    }

    fn delete_to_start_of_line(&mut self) {
        self.exit_history_browsing();
        let line = self.state.lines[self.state.cursor_line].clone();
        if self.state.cursor_col > 0 {
            self.push_undo();
            let deleted = line[..self.state.cursor_col].to_string();
            let acc = self.last_action == Some(LastAction::Kill);
            self.kill_ring.push(&deleted, true, acc);
            self.last_action = Some(LastAction::Kill);
            self.state.lines[self.state.cursor_line] = line[self.state.cursor_col..].to_string();
            self.set_cursor_col(0);
        } else if self.state.cursor_line > 0 {
            self.push_undo();
            let acc = self.last_action == Some(LastAction::Kill);
            self.kill_ring.push("\n", true, acc);
            self.last_action = Some(LastAction::Kill);
            let prev = self.state.lines[self.state.cursor_line - 1].clone();
            self.state.lines[self.state.cursor_line - 1] = format!("{prev}{line}");
            self.state.lines.remove(self.state.cursor_line);
            self.state.cursor_line -= 1;
            self.set_cursor_col(prev.len());
        }
        self.fire_change();
    }

    fn delete_to_end_of_line(&mut self) {
        self.exit_history_browsing();
        let line = self.state.lines[self.state.cursor_line].clone();
        if self.state.cursor_col < line.len() {
            self.push_undo();
            let deleted = line[self.state.cursor_col..].to_string();
            let acc = self.last_action == Some(LastAction::Kill);
            self.kill_ring.push(&deleted, false, acc);
            self.last_action = Some(LastAction::Kill);
            self.state.lines[self.state.cursor_line] = line[..self.state.cursor_col].to_string();
        } else if self.state.cursor_line < self.state.lines.len() - 1 {
            self.push_undo();
            let acc = self.last_action == Some(LastAction::Kill);
            self.kill_ring.push("\n", false, acc);
            self.last_action = Some(LastAction::Kill);
            let next = self.state.lines[self.state.cursor_line + 1].clone();
            self.state.lines[self.state.cursor_line] = format!("{line}{next}");
            self.state.lines.remove(self.state.cursor_line + 1);
        }
        self.fire_change();
    }

    fn delete_word_backwards(&mut self) {
        self.exit_history_browsing();
        let line = self.state.lines[self.state.cursor_line].clone();
        if self.state.cursor_col == 0 {
            if self.state.cursor_line > 0 {
                self.push_undo();
                let acc = self.last_action == Some(LastAction::Kill);
                self.kill_ring.push("\n", true, acc);
                self.last_action = Some(LastAction::Kill);
                let prev = self.state.lines[self.state.cursor_line - 1].clone();
                self.state.lines[self.state.cursor_line - 1] = format!("{prev}{line}");
                self.state.lines.remove(self.state.cursor_line);
                self.state.cursor_line -= 1;
                self.set_cursor_col(prev.len());
            }
        } else {
            self.push_undo();
            let was_kill = self.last_action == Some(LastAction::Kill);
            let old_col = self.state.cursor_col;
            let delete_from = find_word_backward(&line, old_col, Some(Self::marker_is_atomic_fn()));
            let deleted = line[delete_from..old_col].to_string();
            self.kill_ring.push(&deleted, true, was_kill);
            self.last_action = Some(LastAction::Kill);
            self.state.lines[self.state.cursor_line] =
                format!("{}{}", &line[..delete_from], &line[old_col..]);
            self.set_cursor_col(delete_from);
        }
        self.fire_change();
    }

    fn delete_word_forward(&mut self) {
        self.exit_history_browsing();
        let line = self.state.lines[self.state.cursor_line].clone();
        if self.state.cursor_col >= line.len() {
            if self.state.cursor_line < self.state.lines.len() - 1 {
                self.push_undo();
                let acc = self.last_action == Some(LastAction::Kill);
                self.kill_ring.push("\n", false, acc);
                self.last_action = Some(LastAction::Kill);
                let next = self.state.lines[self.state.cursor_line + 1].clone();
                self.state.lines[self.state.cursor_line] = format!("{line}{next}");
                self.state.lines.remove(self.state.cursor_line + 1);
            }
        } else {
            self.push_undo();
            let was_kill = self.last_action == Some(LastAction::Kill);
            let delete_to = find_word_forward(
                &line,
                self.state.cursor_col,
                Some(Self::marker_is_atomic_fn()),
            );
            let deleted = line[self.state.cursor_col..delete_to].to_string();
            self.kill_ring.push(&deleted, false, was_kill);
            self.last_action = Some(LastAction::Kill);
            self.state.lines[self.state.cursor_line] =
                format!("{}{}", &line[..self.state.cursor_col], &line[delete_to..]);
        }
        self.fire_change();
    }

    fn move_to_line_start(&mut self) {
        self.last_action = None;
        self.set_cursor_col(0);
    }

    fn move_to_line_end(&mut self) {
        self.last_action = None;
        let len = self.state.lines[self.state.cursor_line].len();
        self.set_cursor_col(len);
    }

    fn move_word_backwards(&mut self) {
        self.last_action = None;
        if self.state.cursor_col == 0 {
            if self.state.cursor_line > 0 {
                self.state.cursor_line -= 1;
                let len = self.state.lines[self.state.cursor_line].len();
                self.set_cursor_col(len);
            }
            return;
        }
        let line = self.state.lines[self.state.cursor_line].clone();
        self.set_cursor_col(find_word_backward(
            &line,
            self.state.cursor_col,
            Some(Self::marker_is_atomic_fn()),
        ));
    }

    fn move_word_forwards(&mut self) {
        self.last_action = None;
        let line = self.state.lines[self.state.cursor_line].clone();
        if self.state.cursor_col >= line.len() {
            if self.state.cursor_line < self.state.lines.len() - 1 {
                self.state.cursor_line += 1;
                self.set_cursor_col(0);
            }
            return;
        }
        self.set_cursor_col(find_word_forward(
            &line,
            self.state.cursor_col,
            Some(Self::marker_is_atomic_fn()),
        ));
    }

    // ---- kill ring yank ----
    fn yank(&mut self) {
        if self.kill_ring.is_empty() {
            return;
        }
        self.push_undo();
        let text = self.kill_ring.peek().unwrap().to_string();
        self.insert_yanked_text(&text);
        self.last_action = Some(LastAction::Yank);
    }

    fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();
        self.delete_yanked_text();
        self.kill_ring.rotate();
        let text = self.kill_ring.peek().unwrap().to_string();
        self.insert_yanked_text(&text);
        self.last_action = Some(LastAction::Yank);
    }

    fn insert_yanked_text(&mut self, text: &str) {
        self.exit_history_browsing();
        let lines: Vec<&str> = text.split('\n').collect();
        let current = self.state.lines[self.state.cursor_line].clone();
        let before = current[..self.state.cursor_col].to_string();
        let after = current[self.state.cursor_col..].to_string();
        if lines.len() == 1 {
            self.state.lines[self.state.cursor_line] = format!("{before}{text}{after}");
            self.set_cursor_col(self.state.cursor_col + text.len());
        } else {
            self.state.lines[self.state.cursor_line] = format!("{before}{}", lines[0]);
            for (i, &mid) in lines[1..lines.len() - 1].iter().enumerate() {
                self.state
                    .lines
                    .insert(self.state.cursor_line + 1 + i, mid.to_string());
            }
            let last_line_index = self.state.cursor_line + lines.len() - 1;
            self.state.lines.insert(
                last_line_index,
                format!("{}{after}", lines[lines.len() - 1]),
            );
            self.state.cursor_line = last_line_index;
            self.set_cursor_col(lines[lines.len() - 1].len());
        }
        self.fire_change();
    }

    fn delete_yanked_text(&mut self) {
        let Some(yanked) = self.kill_ring.peek().map(|s| s.to_string()) else {
            return;
        };
        let yank_lines: Vec<&str> = yanked.split('\n').collect();
        if yank_lines.len() == 1 {
            let current = self.state.lines[self.state.cursor_line].clone();
            let del_len = yanked.len();
            let start = self.state.cursor_col.saturating_sub(del_len);
            self.state.lines[self.state.cursor_line] =
                format!("{}{}", &current[..start], &current[self.state.cursor_col..]);
            self.set_cursor_col(start);
        } else {
            let start_line = self.state.cursor_line - (yank_lines.len() - 1);
            let start_col = self.state.lines[start_line]
                .len()
                .saturating_sub(yank_lines[0].len());
            let after_cursor =
                self.state.lines[self.state.cursor_line][self.state.cursor_col..].to_string();
            let before_yank = self.state.lines[start_line][..start_col].to_string();
            let merged = format!("{before_yank}{after_cursor}");
            self.state.lines.splice(
                start_line..start_line + yank_lines.len(),
                std::iter::once(merged),
            );
            self.state.cursor_line = start_line;
            self.set_cursor_col(start_col);
        }
        self.fire_change();
    }

    fn undo(&mut self) {
        self.exit_history_browsing();
        if let Some(snapshot) = self.undo_stack.pop() {
            self.state = snapshot;
            self.last_action = None;
            self.preferred_visual_col = None;
            self.fire_change();
        }
    }

    fn jump_to_char(&mut self, ch: &str, forward: bool) {
        self.last_action = None;
        let n = self.state.lines.len() as i64;
        let step: i64 = if forward { 1 } else { -1 };
        let mut line_idx = self.state.cursor_line as i64;
        while line_idx >= 0 && line_idx < n {
            let line = &self.state.lines[line_idx as usize];
            let is_current = line_idx as usize == self.state.cursor_line;
            let found = if forward {
                let from = if is_current {
                    // cursor_col is a byte offset on a char boundary; skip the
                    // full character under the cursor (may be multi-byte, e.g.
                    // CJK) instead of +1, which would slice mid-character.
                    self.state.cursor_col
                        + line[self.state.cursor_col.min(line.len())..]
                            .chars()
                            .next()
                            .map_or(1, |c| c.len_utf8())
                } else {
                    0
                };
                if from <= line.len() {
                    line[from..].find(ch).map(|i| i + from)
                } else {
                    None
                }
            } else {
                let to = if is_current {
                    self.state.cursor_col
                } else {
                    line.len()
                };
                line[..to.min(line.len())].rfind(ch)
            };
            if let Some(idx) = found {
                self.state.cursor_line = line_idx as usize;
                self.set_cursor_col(idx);
                return;
            }
            line_idx += step;
        }
    }

    // ---- visual line map / vertical movement ----
    fn build_visual_line_map(&self, width: usize) -> Vec<VisualLine> {
        let mut vls: Vec<VisualLine> = Vec::new();
        for (i, line) in self.state.lines.iter().enumerate() {
            if line.is_empty() {
                vls.push(VisualLine {
                    logical_line: i,
                    start_col: 0,
                    length: 0,
                });
            } else if visible_width(line) <= width {
                vls.push(VisualLine {
                    logical_line: i,
                    start_col: 0,
                    length: line.len(),
                });
            } else {
                let segs = self.segment_graphemes(line);
                for chunk in word_wrap_line(line, width, Some(&segs)) {
                    vls.push(VisualLine {
                        logical_line: i,
                        start_col: chunk.start_index,
                        length: chunk.end_index - chunk.start_index,
                    });
                }
            }
        }
        vls
    }

    fn find_visual_line_at(vls: &[VisualLine], line: usize, col: usize) -> usize {
        for (i, vl) in vls.iter().enumerate() {
            if vl.logical_line != line {
                continue;
            }
            if col < vl.start_col {
                continue;
            }
            let offset = col - vl.start_col;
            let is_last_of_line = i == vls.len() - 1 || vls[i + 1].logical_line != vl.logical_line;
            if offset < vl.length || (is_last_of_line && offset == vl.length) {
                return i;
            }
        }
        vls.len().saturating_sub(1)
    }

    fn find_current_visual_line(&self, vls: &[VisualLine]) -> usize {
        Self::find_visual_line_at(vls, self.state.cursor_line, self.state.cursor_col)
    }

    fn is_on_first_visual_line(&self) -> bool {
        let vls = self.build_visual_line_map(self.last_width);
        self.find_current_visual_line(&vls) == 0
    }

    fn is_on_last_visual_line(&self) -> bool {
        let vls = self.build_visual_line_map(self.last_width);
        self.find_current_visual_line(&vls) == vls.len().saturating_sub(1)
    }

    fn compute_vertical_move_column(
        &mut self,
        current_visual_col: usize,
        source_max: usize,
        target_max: usize,
    ) -> usize {
        let has_preferred = self.preferred_visual_col.is_some();
        let cursor_in_middle = current_visual_col < source_max;
        let target_too_short = target_max < current_visual_col;

        if !has_preferred || cursor_in_middle {
            if target_too_short {
                self.preferred_visual_col = Some(current_visual_col);
                return target_max;
            }
            self.preferred_visual_col = None;
            return current_visual_col;
        }

        let preferred = self.preferred_visual_col.unwrap();
        let target_cant_fit = target_max < preferred;
        if target_too_short || target_cant_fit {
            return target_max;
        }
        self.preferred_visual_col = None;
        preferred
    }

    fn move_to_visual_line(&mut self, vls: &[VisualLine], current_vl: usize, target_vl: usize) {
        let Some(&current) = vls.get(current_vl) else {
            return;
        };
        let Some(&target) = vls.get(target_vl) else {
            return;
        };

        let current_visual_col = if let Some(snap) = self.snapped_from_cursor_col {
            let vl_idx = Self::find_visual_line_at(vls, current.logical_line, snap);
            snap - vls[vl_idx].start_col
        } else {
            self.state.cursor_col - current.start_col
        };

        let is_last_source =
            current_vl == vls.len() - 1 || vls[current_vl + 1].logical_line != current.logical_line;
        let source_max = if is_last_source {
            current.length
        } else {
            current.length.saturating_sub(1)
        };
        let is_last_target =
            target_vl == vls.len() - 1 || vls[target_vl + 1].logical_line != target.logical_line;
        let target_max = if is_last_target {
            target.length
        } else {
            target.length.saturating_sub(1)
        };

        let move_to_visual_col =
            self.compute_vertical_move_column(current_visual_col, source_max, target_max);

        self.state.cursor_line = target.logical_line;
        let target_col = target.start_col + move_to_visual_col;
        let logical_line = self.state.lines[target.logical_line].clone();
        self.state.cursor_col = target_col.min(logical_line.len());

        // snap 到原子段边界
        let segs = self.segment_graphemes(&logical_line);
        for (seg_index, seg) in segs {
            if seg_index > self.state.cursor_col {
                break;
            }
            if seg.len() <= 1 {
                continue;
            }
            if self.state.cursor_col < seg_index + seg.len() {
                let is_continuation = seg_index < target.start_col;
                let is_moving_down = target_vl > current_vl;
                if is_continuation && is_moving_down {
                    let seg_end = seg_index + seg.len();
                    let mut next = target_vl + 1;
                    while next < vls.len()
                        && vls[next].logical_line == target.logical_line
                        && vls[next].start_col < seg_end
                    {
                        next += 1;
                    }
                    if next < vls.len() {
                        self.move_to_visual_line(vls, current_vl, next);
                        return;
                    }
                }
                self.snapped_from_cursor_col = Some(self.state.cursor_col);
                self.state.cursor_col = seg_index;
                return;
            }
        }
        self.snapped_from_cursor_col = None;
    }

    fn move_cursor(&mut self, delta_line: i64, delta_col: i64) {
        self.last_action = None;
        let vls = self.build_visual_line_map(self.last_width);
        let current_vl = self.find_current_visual_line(&vls);

        if delta_line != 0 {
            let target = current_vl as i64 + delta_line;
            if target >= 0 && (target as usize) < vls.len() {
                self.move_to_visual_line(&vls, current_vl, target as usize);
            }
        }

        if delta_col != 0 {
            let line = self.state.lines[self.state.cursor_line].clone();
            if delta_col > 0 {
                if self.state.cursor_col < line.len() {
                    let glen = self.first_grapheme_len(&line[self.state.cursor_col..]);
                    self.set_cursor_col(self.state.cursor_col + glen);
                } else if self.state.cursor_line < self.state.lines.len() - 1 {
                    self.state.cursor_line += 1;
                    self.set_cursor_col(0);
                } else if let Some(vl) = vls.get(current_vl) {
                    self.preferred_visual_col = Some(self.state.cursor_col - vl.start_col);
                }
            } else if self.state.cursor_col > 0 {
                let glen = self.last_grapheme_len(&line[..self.state.cursor_col]);
                self.set_cursor_col(self.state.cursor_col - glen);
            } else if self.state.cursor_line > 0 {
                self.state.cursor_line -= 1;
                let len = self.state.lines[self.state.cursor_line].len();
                self.set_cursor_col(len);
            }
        }
    }

    fn page_scroll(&mut self, direction: i64) {
        self.last_action = None;
        let page_size = (self.terminal_rows as f64 * 0.3).floor().max(5.0) as i64;
        let vls = self.build_visual_line_map(self.last_width);
        let current_vl = self.find_current_visual_line(&vls) as i64;
        let target = (current_vl + direction * page_size).clamp(0, vls.len() as i64 - 1);
        self.move_to_visual_line(&vls, current_vl as usize, target as usize);
    }

    fn navigate_history(&mut self, direction: i64) {
        self.last_action = None;
        if self.history.is_empty() {
            return;
        }
        let new_index = self.history_index - direction;
        if new_index < -1 || new_index >= self.history.len() as i64 {
            return;
        }
        if self.history_index == -1 && new_index >= 0 {
            self.push_undo();
            self.history_draft = Some(self.state.clone());
        }
        self.history_index = new_index;
        if self.history_index == -1 {
            if let Some(draft) = self.history_draft.take() {
                self.state = draft;
                self.preferred_visual_col = None;
                self.snapped_from_cursor_col = None;
                self.scroll_offset = 0;
                self.fire_change();
            } else {
                self.set_text_internal("", false);
            }
        } else {
            let text = self.history[self.history_index as usize].clone();
            self.set_text_internal(&text, direction == -1);
        }
    }

    // ---- paste ----
    fn handle_paste(&mut self, pasted: &str) {
        self.exit_history_browsing();
        self.last_action = None;
        self.push_undo();
        let clean = Self::normalize_text(pasted);
        let filtered: String = clean
            .chars()
            .filter(|&c| c == '\n' || (c as u32) >= 32)
            .collect();
        let pasted_lines: Vec<&str> = filtered.split('\n').collect();
        let total_chars = filtered.chars().count();
        if pasted_lines.len() > 10 || total_chars > 1000 {
            self.paste_counter += 1;
            let paste_id = self.paste_counter;
            self.pastes.insert(paste_id, filtered.clone());
            let marker = if pasted_lines.len() > 10 {
                format!("[paste #{paste_id} +{} lines]", pasted_lines.len())
            } else {
                format!("[paste #{paste_id} {total_chars} chars]")
            };
            self.insert_text_at_cursor_internal(&marker);
            return;
        }
        self.insert_text_at_cursor_internal(&filtered);
    }

    fn insert_text_at_cursor_internal(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let normalized = Self::normalize_text(text);
        let inserted_lines: Vec<&str> = normalized.split('\n').collect();
        let current = self.state.lines[self.state.cursor_line].clone();
        let before = current[..self.state.cursor_col].to_string();
        let after = current[self.state.cursor_col..].to_string();
        if inserted_lines.len() == 1 {
            self.state.lines[self.state.cursor_line] = format!("{before}{normalized}{after}");
            self.set_cursor_col(self.state.cursor_col + normalized.len());
        } else {
            let mut new_lines: Vec<String> = Vec::new();
            new_lines.extend(self.state.lines[..self.state.cursor_line].iter().cloned());
            new_lines.push(format!("{before}{}", inserted_lines[0]));
            new_lines.extend(
                inserted_lines[1..inserted_lines.len() - 1]
                    .iter()
                    .map(|s| s.to_string()),
            );
            new_lines.push(format!(
                "{}{after}",
                inserted_lines[inserted_lines.len() - 1]
            ));
            new_lines.extend(
                self.state.lines[self.state.cursor_line + 1..]
                    .iter()
                    .cloned(),
            );
            self.state.cursor_line += inserted_lines.len() - 1;
            let col = inserted_lines[inserted_lines.len() - 1].len();
            self.state.lines = new_lines;
            self.set_cursor_col(col);
        }
        self.fire_change();
    }

    fn layout_text(&self, content_width: usize) -> Vec<LayoutLine> {
        let mut layout: Vec<LayoutLine> = Vec::new();
        if self.state.lines.len() == 1 && self.state.lines[0].is_empty() {
            layout.push(LayoutLine {
                text: String::new(),
                has_cursor: true,
                cursor_pos: Some(0),
            });
            return layout;
        }
        for (i, line) in self.state.lines.iter().enumerate() {
            let is_current = i == self.state.cursor_line;
            if visible_width(line) <= content_width {
                layout.push(LayoutLine {
                    text: line.clone(),
                    has_cursor: is_current,
                    cursor_pos: if is_current {
                        Some(self.state.cursor_col)
                    } else {
                        None
                    },
                });
            } else {
                let segs = self.segment_graphemes(line);
                let chunks = word_wrap_line(line, content_width, Some(&segs));
                let n = chunks.len();
                for (chunk_index, chunk) in chunks.iter().enumerate() {
                    let cursor_pos = self.state.cursor_col;
                    let is_last = chunk_index == n - 1;
                    let mut has_cursor = false;
                    let mut adjusted = 0usize;
                    if is_current {
                        if is_last {
                            has_cursor = cursor_pos >= chunk.start_index;
                            adjusted = cursor_pos.saturating_sub(chunk.start_index);
                        } else {
                            has_cursor =
                                cursor_pos >= chunk.start_index && cursor_pos < chunk.end_index;
                            if has_cursor {
                                adjusted = (cursor_pos - chunk.start_index).min(chunk.text.len());
                            }
                        }
                    }
                    layout.push(LayoutLine {
                        text: chunk.text.clone(),
                        has_cursor,
                        cursor_pos: if has_cursor { Some(adjusted) } else { None },
                    });
                }
            }
        }
        layout
    }

    fn handle_jump_pending(&mut self, data: &str) -> bool {
        // returns true if handled
        if self.jump_mode.is_none() {
            return false;
        }
        if self.matches(data, "tui.editor.jumpForward")
            || self.matches(data, "tui.editor.jumpBackward")
        {
            self.jump_mode = None;
            return true;
        }
        let printable = decode_printable_key(data)
            .map(|c| c.to_string())
            .or_else(|| {
                if data.chars().next().map(|c| c as u32 >= 32).unwrap_or(false) {
                    Some(data.to_string())
                } else {
                    None
                }
            });
        if let Some(p) = printable {
            let dir = self.jump_mode.take().unwrap();
            self.jump_to_char(&p, dir == JumpMode::Forward);
            return true;
        }
        self.jump_mode = None;
        false
    }
}

impl Component for Editor {
    fn handle_input(&mut self, data: &str) {
        if self.handle_jump_pending(data) {
            return;
        }

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
                if !content.is_empty() {
                    self.handle_paste(&content);
                }
                self.is_in_paste = false;
                let remaining = self.paste_buffer[end + "\x1b[201~".len()..].to_string();
                self.paste_buffer.clear();
                if !remaining.is_empty() {
                    self.handle_input(&remaining);
                }
            }
            return;
        }

        // Ctrl+C - 让父级处理
        if self.matches(&data, "tui.input.copy") {
            return;
        }
        if self.matches(&data, "tui.editor.undo") {
            self.undo();
            return;
        }

        // Tab has no editor action (historically reserved for the now-removed
        // slash-command autocomplete); swallow it so it never inserts a literal tab.
        if self.matches(&data, "tui.input.tab") {
            return;
        }

        // 删除
        if self.matches(&data, "tui.editor.deleteToLineEnd") {
            self.delete_to_end_of_line();
            return;
        }
        if self.matches(&data, "tui.editor.deleteToLineStart") {
            self.delete_to_start_of_line();
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
        if self.matches(&data, "tui.editor.deleteCharBackward")
            || matches_key(&data, "shift+backspace", self.kitty_active)
        {
            self.handle_backspace();
            return;
        }
        if self.matches(&data, "tui.editor.deleteCharForward")
            || matches_key(&data, "shift+delete", self.kitty_active)
        {
            self.handle_forward_delete();
            return;
        }

        // kill ring
        if self.matches(&data, "tui.editor.yank") {
            self.yank();
            return;
        }
        if self.matches(&data, "tui.editor.yankPop") {
            self.yank_pop();
            return;
        }

        // 光标移动（行首尾 / 词）
        if self.matches(&data, "tui.editor.cursorLineStart") {
            self.move_to_line_start();
            return;
        }
        if self.matches(&data, "tui.editor.cursorLineEnd") {
            self.move_to_line_end();
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

        // 换行
        if self.matches(&data, "tui.input.newLine")
            || data == "\x1b\r"
            || data == "\x1b[13;2~"
            || (data == "\n" && data.len() == 1)
        {
            self.add_new_line();
            return;
        }

        // 提交
        if self.matches(&data, "tui.input.submit") {
            if self.disable_submit {
                return;
            }
            // 反斜杠+回车换行 workaround
            let current = &self.state.lines[self.state.cursor_line];
            if self.state.cursor_col > 0
                && current.as_bytes().get(self.state.cursor_col - 1) == Some(&b'\\')
            {
                self.handle_backspace();
                self.add_new_line();
                return;
            }
            self.submit_value();
            return;
        }

        // 方向键（含历史）
        if self.matches(&data, "tui.editor.cursorUp") {
            if self.is_on_first_visual_line() && !self.history.is_empty() {
                self.navigate_history(-1);
            } else if self.is_on_first_visual_line() {
                self.move_to_line_start();
            } else {
                self.move_cursor(-1, 0);
            }
            return;
        }
        if self.matches(&data, "tui.editor.cursorDown") {
            if self.history_index > -1 && self.is_on_last_visual_line() {
                self.navigate_history(1);
            } else if self.is_on_last_visual_line() {
                self.move_to_line_end();
            } else {
                self.move_cursor(1, 0);
            }
            return;
        }
        if self.matches(&data, "tui.editor.cursorRight") {
            self.move_cursor(0, 1);
            return;
        }
        if self.matches(&data, "tui.editor.cursorLeft") {
            self.move_cursor(0, -1);
            return;
        }

        if self.matches(&data, "tui.editor.pageUp") {
            self.page_scroll(-1);
            return;
        }
        if self.matches(&data, "tui.editor.pageDown") {
            self.page_scroll(1);
            return;
        }

        if self.matches(&data, "tui.editor.jumpForward") {
            self.jump_mode = Some(JumpMode::Forward);
            return;
        }
        if self.matches(&data, "tui.editor.jumpBackward") {
            self.jump_mode = Some(JumpMode::Backward);
            return;
        }

        if matches_key(&data, "shift+space", self.kitty_active) {
            self.insert_character(" ");
            return;
        }

        if let Some(printable) = decode_printable_key(&data) {
            self.insert_character(&printable.to_string());
            return;
        }
        if data.chars().next().map(|c| c as u32 >= 32).unwrap_or(false) {
            self.insert_character(&data);
        }
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let max_padding = (width.saturating_sub(1)) / 2;
        let padding_x = self.padding_x.min(max_padding);
        let content_width = width.saturating_sub(padding_x * 2).max(1);
        let layout_width = content_width
            .saturating_sub(if padding_x > 0 { 0 } else { 1 })
            .max(1);
        self.last_width = layout_width;

        let horizontal = (self.border_color)("─");
        let layout_lines = self.layout_text(layout_width);

        let max_visible_lines = (self.terminal_rows as f64 * 0.3).floor().max(5.0) as usize;

        let mut cursor_line_index = layout_lines.iter().position(|l| l.has_cursor).unwrap_or(0);
        let _ = &mut cursor_line_index;
        if cursor_line_index < self.scroll_offset {
            self.scroll_offset = cursor_line_index;
        } else if cursor_line_index >= self.scroll_offset + max_visible_lines {
            self.scroll_offset = cursor_line_index - max_visible_lines + 1;
        }
        let max_scroll = layout_lines.len().saturating_sub(max_visible_lines);
        self.scroll_offset = self.scroll_offset.min(max_scroll);

        let visible_end = (self.scroll_offset + max_visible_lines).min(layout_lines.len());
        let visible_lines = &layout_lines[self.scroll_offset..visible_end];

        let mut result: Vec<String> = Vec::new();
        let left_padding = " ".repeat(padding_x);
        let right_padding = left_padding.clone();

        // top border
        if self.scroll_offset > 0 {
            let indicator = format!("─── ↑ {} more ", self.scroll_offset);
            let remaining = width as i64 - visible_width(&indicator) as i64;
            if remaining >= 0 {
                result.push((self.border_color)(&format!(
                    "{indicator}{}",
                    "─".repeat(remaining as usize)
                )));
            } else {
                result.push((self.border_color)(&truncate_to_width(
                    &indicator, width, "", false,
                )));
            }
        } else {
            result.push(horizontal.repeat(width));
        }

        let emit_marker = self.focused;
        for layout_line in visible_lines {
            let mut display_text = layout_line.text.clone();
            let mut line_visible_width = visible_width(&layout_line.text);
            let mut cursor_in_padding = false;

            if layout_line.has_cursor {
                if let Some(cpos) = layout_line.cursor_pos {
                    let cpos = cpos.min(display_text.len());
                    let before = display_text[..cpos].to_string();
                    let after = display_text[cpos..].to_string();
                    let marker = if emit_marker { CURSOR_MARKER } else { "" };
                    if !after.is_empty() {
                        let segs = self.segment_graphemes(&after);
                        let first = segs.first().map(|(_, g)| g.clone()).unwrap_or_default();
                        let rest = after[first.len()..].to_string();
                        let cursor = format!("\x1b[7m{first}\x1b[0m");
                        display_text = format!("{before}{marker}{cursor}{rest}");
                    } else {
                        let cursor = "\x1b[7m \x1b[0m";
                        display_text = format!("{before}{marker}{cursor}");
                        line_visible_width += 1;
                        if line_visible_width > content_width && padding_x > 0 {
                            cursor_in_padding = true;
                        }
                    }
                }
            }

            let padding = " ".repeat(content_width.saturating_sub(line_visible_width));
            let line_right_padding = if cursor_in_padding && !right_padding.is_empty() {
                &right_padding[1..]
            } else {
                &right_padding[..]
            };
            result.push(format!(
                "{left_padding}{display_text}{padding}{line_right_padding}"
            ));
        }

        // bottom border
        let lines_below =
            layout_lines.len() as i64 - (self.scroll_offset + visible_lines.len()) as i64;
        if lines_below > 0 {
            let indicator = format!("─── ↓ {} more ", lines_below);
            let remaining = (width as i64 - visible_width(&indicator) as i64).max(0) as usize;
            result.push((self.border_color)(&format!(
                "{indicator}{}",
                "─".repeat(remaining)
            )));
        } else {
            result.push(horizontal.repeat(width));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn theme() -> EditorTheme {
        let id: ColorFn = Arc::new(|s: &str| s.to_string());
        EditorTheme {
            border_color: id.clone(),
            select_list: SelectListTheme {
                selected_prefix: id.clone(),
                selected_text: id.clone(),
                description: id.clone(),
                scroll_info: id.clone(),
                no_match: id,
            },
        }
    }

    fn editor() -> Editor {
        let mut e = Editor::new(theme());
        e.set_terminal_rows(24);
        e
    }

    fn type_str(e: &mut Editor, s: &str) {
        for ch in s.chars() {
            e.handle_input(&ch.to_string());
        }
    }

    #[test]
    fn insert_text() {
        let mut e = editor();
        type_str(&mut e, "hello");
        assert_eq!(e.get_text(), "hello");
        assert_eq!(e.get_cursor(), (0, 5));
    }

    #[test]
    fn newline_creates_logical_line() {
        let mut e = editor();
        type_str(&mut e, "ab");
        e.handle_input("\x1b\r"); // alt+enter style newline (matches "\x1b\r")
        type_str(&mut e, "cd");
        assert_eq!(e.get_text(), "ab\ncd");
        assert_eq!(e.get_cursor(), (1, 2));
    }

    #[test]
    fn backspace_merges_lines() {
        let mut e = editor();
        type_str(&mut e, "ab");
        e.handle_input("\x1b\r");
        type_str(&mut e, "cd");
        // cursor at (1,2); move to line start then backspace merges
        e.handle_input("\x01"); // ctrl+a
        e.handle_input("\x7f"); // backspace
        assert_eq!(e.get_text(), "abcd");
        assert_eq!(e.get_cursor(), (0, 2));
    }

    #[test]
    fn wrapping_long_line() {
        // 忠实端口的 editor wordWrapLine 在「余下内容+当前字仍能放下」时回退到上一断点，
        // 因此可能比朴素贪心更早断行。只保证每段不超宽、且首段非空。
        let chunks = word_wrap_line("the quick brown fox", 9, None);
        for c in &chunks {
            assert!(visible_width(&c.text) <= 9, "chunk too wide: {:?}", c.text);
        }
        assert!(!chunks.is_empty());
        assert!(chunks[0].text.starts_with("the"));
    }

    #[test]
    fn wrapping_indices_consistent() {
        let line = "hello world foobar";
        let chunks = word_wrap_line(line, 11, None);
        for c in &chunks {
            assert_eq!(&line[c.start_index..c.end_index], c.text);
        }
    }

    #[test]
    fn wrapping_breaks_long_word() {
        let chunks = word_wrap_line("abcdefghij", 4, None);
        assert_eq!(
            chunks.iter().map(|c| c.text.clone()).collect::<Vec<_>>(),
            vec!["abcd", "efgh", "ij"]
        );
    }

    #[test]
    fn vertical_nav_moves_between_logical_lines() {
        let mut e = editor();
        type_str(&mut e, "first");
        e.handle_input("\x1b\r");
        type_str(&mut e, "second");
        // cursor on line 1; move up
        e.handle_input("\x1b[A"); // up
        assert_eq!(e.get_cursor().0, 0);
        e.handle_input("\x1b[B"); // down
        assert_eq!(e.get_cursor().0, 1);
    }

    #[test]
    fn vertical_nav_in_wrapped_line() {
        let mut e = editor();
        e.set_terminal_rows(40);
        // a single logical line that wraps across several visual lines
        let long = "word ".repeat(20); // 100 chars
        type_str(&mut e, &long);
        // narrow width forces wrapping
        e.render(20);
        let (_l0, c0) = e.get_cursor();
        e.handle_input("\x1b[A"); // up one visual line
        let (_l1, c1) = e.get_cursor();
        assert!(
            c1 < c0,
            "cursor should move earlier in the logical line when going up a visual line"
        );
    }

    #[test]
    fn viewport_scrolls_when_cursor_below() {
        let mut e = editor();
        e.set_terminal_rows(20); // max_visible = max(5, 6) = 6
        for i in 0..15 {
            type_str(&mut e, &format!("line{i}"));
            if i < 14 {
                e.handle_input("\x1b\r");
            }
        }
        e.render(40);
        assert!(
            e.scroll_offset > 0,
            "expected viewport to scroll down with cursor at the bottom"
        );
    }

    #[test]
    fn word_movement_and_delete() {
        let mut e = editor();
        type_str(&mut e, "foo bar baz");
        e.handle_input("\x17"); // ctrl+w delete word backward
        assert_eq!(e.get_text(), "foo bar ");
        e.handle_input("\x19"); // ctrl+y yank
        assert_eq!(e.get_text(), "foo bar baz");
    }

    #[test]
    fn undo_restores_state() {
        let mut e = editor();
        type_str(&mut e, "abc");
        e.handle_input(" "); // captures "abc", becomes "abc "
        type_str(&mut e, "def"); // coalesces into the space's undo unit
        e.handle_input("\x1f"); // ctrl+- undo -> back to "abc"
        assert_eq!(e.get_text(), "abc");
    }

    #[test]
    fn render_has_borders() {
        let mut e = editor();
        type_str(&mut e, "hi");
        let lines = e.render(20);
        // top + content + bottom
        assert!(lines.len() >= 3);
        assert!(lines[0].contains("─"));
        assert!(lines[lines.len() - 1].contains("─"));
    }

    #[test]
    fn render_cursor_marker_when_focused() {
        let mut e = editor();
        e.focused = true;
        type_str(&mut e, "hi");
        let lines = e.render(20);
        assert!(lines.iter().any(|l| l.contains(CURSOR_MARKER)));
    }

    #[test]
    fn submit_clears_and_fires() {
        use std::sync::Mutex;
        let captured = Arc::new(Mutex::new(String::new()));
        let c2 = captured.clone();
        let mut e = editor();
        e.on_submit = Some(Box::new(move |v| *c2.lock().unwrap() = v));
        type_str(&mut e, "hello");
        e.handle_input("\r");
        assert_eq!(*captured.lock().unwrap(), "hello");
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn history_navigation() {
        let mut e = editor();
        e.add_to_history("old prompt");
        // up on first visual line with history -> load it
        e.handle_input("\x1b[A");
        assert_eq!(e.get_text(), "old prompt");
    }

    #[test]
    fn jump_to_char_forward() {
        let mut e = editor();
        type_str(&mut e, "hello world");
        e.handle_input("\x01"); // ctrl+a -> line start
        e.handle_input("\x1d"); // ctrl+] jump forward
        e.handle_input("w");
        assert_eq!(e.get_cursor(), (0, 6));
    }

    #[test]
    fn jump_to_char_forward_over_multibyte_char() {
        // Regression: cursor on a multi-byte grapheme (CJK). The old
        // `cursor_col + 1` produced a mid-character byte index and
        // `line[from..]` panicked ("byte index 1 is not a char boundary").
        let mut e = editor();
        type_str(&mut e, "你好x");
        e.handle_input("\x01"); // ctrl+a -> line start (cursor_col = 0, on 你)
        e.handle_input("\x1d"); // ctrl+] jump forward
        e.handle_input("x");
        assert_eq!(e.get_cursor(), (0, 6)); // "你好" = 6 bytes

        // Jumping to a CJK target also lands on its byte offset.
        e.handle_input("\x01");
        e.handle_input("\x1d");
        e.handle_input("好");
        assert_eq!(e.get_cursor(), (0, 3)); // "你" = 3 bytes
    }

    #[test]
    fn paste_marker_for_large_paste() {
        let mut e = editor();
        let big = (0..20)
            .map(|i| format!("row{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        e.handle_input(&format!("\x1b[200~{big}\x1b[201~"));
        // editor text shows a marker, expanded text is the full paste
        assert!(e.get_text().contains("[paste #1 +"));
        assert!(e.get_expanded_text().contains("row19"));
    }
}
