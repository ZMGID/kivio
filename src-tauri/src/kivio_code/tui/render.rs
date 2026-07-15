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
use super::text_width::{normalize_terminal_output, truncate_to_width, visible_width};

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

/// 一帧的静/动双区切分（static = 已定稿历史，提交一次进 scrollback 不再 diff；dynamic = 仍在变的
/// 尾部 + spinner + editor/overlay + footer，就地差分，永远在底部）。`static_lines` 是**累积**的全部
/// 已定稿行（单调增长，渲染器只提交其中尚未提交的尾段 `static_lines[committed..]`）；`dynamic_lines`
/// 是当前帧底部仍在变的全部行。CURSOR_MARKER 只可能出现在 `dynamic_lines` 内（光标在 editor/overlay）。
#[derive(Default, Clone)]
pub struct Frame {
    /// 累积的已定稿行（单调增长）。渲染器提交其中尚未写出的尾段进 scrollback，不参与 diff。
    pub static_lines: Vec<String>,
    /// 底部仍在变的行（spinner / 流式消息 / Running 工具卡 / editor/overlay / footer）。就地差分。
    pub dynamic_lines: Vec<String>,
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

/// 防御性裁剪：把一行裁到最多 `width` 可见列再写入终端。
///
/// 差分渲染器的全部正确性都建立在「每个物理行 ≤ width」这一不变式上：超宽行会让相对光标移动
/// 计算与上一帧 diff 全部错位（行被终端自动 wrap，渲染器却以为它只占一行）。组件**应当**自己 wrap
/// 到 width（见 `Text` / tool_card 的 `body_line`），但无论组件 emit 了什么，这里都兜底保证不变式
/// 成立——绝不 panic，绝不让超宽行破坏差分模型。
///
/// 用 ANSI-aware 的 [`truncate_to_width`]（不计转义序列宽度、按 grapheme 计宽、CJK=2），裁掉
/// 超出部分（无 ellipsis、不 padding，保持与未裁行的视觉一致）。≤ width 的行原样返回（零拷贝快路径）。
fn clip_line_to_width(line: &str, width: u16) -> std::borrow::Cow<'_, str> {
    if visible_width(line) <= width as usize {
        std::borrow::Cow::Borrowed(line)
    } else {
        std::borrow::Cow::Owned(truncate_to_width(line, width as usize, "", false))
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
    /// 已提交进 scrollback 的 static 行数（单调不减）。下一帧只写出 `static_lines[committed..]`，
    /// 那部分自然滚入 scrollback、不再参与 diff。spinner 永远在 dynamic 区，物理上不可能漏进这里。
    committed_static_count: usize,
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
            committed_static_count: 0,
        }
    }

    pub fn add_child(&mut self, c: Box<dyn Component>) {
        self.root.add_child(c);
    }

    pub fn invalidate(&mut self) {
        self.root.invalidate();
    }

    /// 全量重绘次数（测试用）。
    #[cfg(test)]
    pub fn full_redraws(&self) -> u32 {
        self.full_redraw_count
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.show_hardware_cursor = enabled;
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// 主渲染入口（单区，向后兼容）：渲染组件树，与上一帧 diff，写出最小转义输出。
    ///
    /// 等价于「全部内容都是 dynamic、无 static」的 [`Self::render_frame`]。
    pub fn render(&mut self) {
        if self.stopped {
            return;
        }
        let width = self.terminal.columns();
        let height = self.terminal.rows() as usize;
        let mut dynamic = self.root.render(width);
        let cursor_pos = extract_cursor_position(&mut dynamic, height);
        apply_line_resets(&mut dynamic);
        self.do_render(dynamic, cursor_pos, width, height);
    }

    /// 双区渲染入口：先把新定稿的 static 行直接提交进 scrollback（不参与 diff），再就地差分 dynamic 区。
    ///
    /// `frame.static_lines` 是**累积**的全部已定稿行（单调增长）；本方法只写出尚未提交的尾段
    /// `static_lines[committed_static_count..]`，那部分自然滚入 scrollback、不再 diff。
    /// `frame.dynamic_lines` 是底部仍在变的全部行（spinner / 流式消息 / Running 工具卡 /
    /// editor/overlay / footer），与上一帧的 dynamic 区做最小差分，永远在 static 之下。
    /// spinner 物理上只存在于 dynamic 区，不可能漏进 scrollback。
    /// CURSOR_MARKER（IME 候选窗硬件光标）只在 dynamic 区内提取并定位。
    pub fn render_frame(&mut self, frame: Frame) {
        if self.stopped {
            return;
        }
        let width = self.terminal.columns();
        let height = self.terminal.rows() as usize;
        let Frame {
            static_lines,
            mut dynamic_lines,
        } = frame;

        let cursor_pos = extract_cursor_position(&mut dynamic_lines, height);
        apply_line_resets(&mut dynamic_lines);

        let width_changed = self.previous_width != 0 && self.previous_width != width;
        let height_changed = self.previous_height != 0 && self.previous_height as usize != height;

        // width 变化 → wrap 改变 → 全量重绘（清屏 + 清 scrollback）；height 变化同样走全量重绘以对齐
        // 视口（normal-buffer 终端无法重新 wrap scrollback，是固有限制，但至少不漏 spinner）。
        if width_changed || height_changed {
            // 清屏 + 清 scrollback，从 home 重新提交全部 static 再重画 dynamic。
            self.committed_static_count = 0;
            let lead_newline = self.commit_static(&static_lines, width, true);
            self.committed_static_count = static_lines.len();
            self.previous_lines.clear();
            self.previous_width = width;
            self.previous_height = height as u16;
            self.hardware_cursor_row = 0;
            self.full_render(
                false,
                lead_newline,
                &dynamic_lines,
                cursor_pos,
                width,
                height,
            );
            return;
        }
        // 新定稿前缀出现：把「新 static 尾段 + dynamic」就地重画到上一帧 dynamic 区的位置（覆盖旧
        // dynamic），新 static 写在 dynamic 之上、随后续帧自然滚入 scrollback、不再 diff。
        if static_lines.len() > self.committed_static_count {
            self.commit_and_redraw(&static_lines, &dynamic_lines, cursor_pos, width, height);
            return;
        }

        // 无新 static、无 resize：纯就地差分 dynamic 区。
        self.do_render(dynamic_lines, cursor_pos, width, height);
    }

    /// 出现新定稿 static 前缀时：把「新 static 尾段 ++ dynamic」就地重画到上一帧 dynamic 区所在位置，
    /// 覆盖旧 dynamic。新 static 行写在 dynamic 之上，之后被后续帧的输出自然顶进 scrollback、不再参与
    /// diff（`committed_static_count` 推进）。重画后 diff baseline 只保留 dynamic 行。
    ///
    /// 关键：上一帧 dynamic 区在屏幕上从「光标上方 `hardware_cursor_row` 行」起（do_render/full_render
    /// 维护的 dynamic 自身坐标系）。先把光标移到 dynamic 顶部，再逐行 `\x1b[2K` 覆盖写出新内容，
    /// 末尾清除旧 dynamic 比新内容多出的残留尾行。
    fn commit_and_redraw(
        &mut self,
        static_lines: &[String],
        dynamic_lines: &[String],
        cursor_pos: Option<(usize, usize)>,
        width: u16,
        height: usize,
    ) {
        self.full_redraw_count += 1;
        let new_static_tail = &static_lines[self.committed_static_count..];
        let prev_dynamic_len = self.previous_lines.len();
        // 上一帧 dynamic 区屏幕行数（受 viewport 约束）。光标在 dynamic 区内 hardware_cursor_row 行处。
        // 若上一帧 dynamic 超过一屏并已滚动，超出视口顶部的部分已进 scrollback、无法覆盖（也无需——
        // 那正是本次要定稿成 static 的内容，已是正确历史）；故上移量 clamp 到视口内（≤ height-1）。
        let move_up = self.hardware_cursor_row.min(height.saturating_sub(1));

        let mut buffer = String::from("\x1b[?2026h");
        if move_up > 0 {
            buffer.push_str(&format!("\x1b[{move_up}A"));
        }
        buffer.push('\r');

        // 就地写出 new_static_tail ++ dynamic_lines（每行先 \x1b[2K 清掉旧 dynamic 残留）。
        let combined_len = new_static_tail.len() + dynamic_lines.len();
        for (written, line) in new_static_tail
            .iter()
            .chain(dynamic_lines.iter())
            .enumerate()
        {
            if written > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\x1b[2K");
            buffer.push_str(&clip_line_to_width(line, width));
        }
        // 旧 dynamic 比新内容长 → 清除多出的尾行。
        if prev_dynamic_len > combined_len {
            let extra = prev_dynamic_len - combined_len;
            for _ in 0..extra {
                buffer.push_str("\r\n\x1b[2K");
            }
            // 回到新内容末行（多走了 extra 行）。
            buffer.push_str(&format!("\x1b[{extra}A"));
        }
        buffer.push_str("\x1b[?2026l");
        self.terminal.write(&buffer);

        self.committed_static_count = static_lines.len();
        // diff baseline 只保留 dynamic：new_static_tail 视为已提交（之上、不再 diff）。
        // 光标现停在「new_static_tail ++ dynamic」的末行；dynamic 坐标系里它在 dynamic 末行
        // （dynamic.len()-1），其上方的 new_static_tail 不属于 dynamic baseline。
        self.cursor_row = dynamic_lines.len().saturating_sub(1);
        self.hardware_cursor_row = dynamic_lines.len().saturating_sub(1);
        self.max_lines_rendered = self.max_lines_rendered.max(combined_len);
        let buffer_len = height.max(dynamic_lines.len());
        self.previous_viewport_top = buffer_len.saturating_sub(height);
        self.position_hardware_cursor(cursor_pos, dynamic_lines.len());
        self.previous_lines = dynamic_lines.to_vec();
        self.previous_width = width;
        self.previous_height = height as u16;
    }

    /// 把尚未提交的 static 尾段写进 scrollback（普通打印，自然滚动）。`clear` 时先清屏 + 清 scrollback
    /// 并从头提交全部 static（width/height 变化）。返回 `true` 表示提交后光标停在已写内容的行首/行尾、
    /// dynamic 区需要先 `\r\n` 换到新行（始终为 true，除非什么都没提交且非清屏）。
    fn commit_static(&mut self, static_lines: &[String], width: u16, clear: bool) -> bool {
        let mut buffer = String::from("\x1b[?2026h");
        let start = if clear {
            buffer.push_str("\x1b[2J\x1b[H\x1b[3J");
            0
        } else {
            self.committed_static_count
        };
        let mut wrote_any = false;
        for (idx, i) in (start..static_lines.len()).enumerate() {
            // 首行：clear 后从 home 起（无前导换行）；非 clear 时承接上一帧已写内容的行尾，需换行另起。
            if idx > 0 || !clear {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\x1b[2K");
            buffer.push_str(&clip_line_to_width(&static_lines[i], width));
            wrote_any = true;
        }
        buffer.push_str("\x1b[?2026l");
        self.terminal.write(&buffer);
        // dynamic 区起始：若提交过任何 static 行，光标停在最后一行行尾 → 需换行。
        // 清屏但无 static（start..len 为空且 clear）：光标在 home(0,0) → dynamic 第一行直接写（无换行）。
        !clear || wrote_any
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

    fn full_render(
        &mut self,
        clear: bool,
        lead_newline: bool,
        new_lines: &[String],
        cursor_pos: Option<(usize, usize)>,
        width: u16,
        height: usize,
    ) {
        self.full_redraw_count += 1;
        let mut buffer = String::from("\x1b[?2026h"); // begin synchronized output
        if clear {
            buffer.push_str("\x1b[2J\x1b[H\x1b[3J"); // clear screen, home, clear scrollback
        }
        for (i, line) in new_lines.iter().enumerate() {
            // lead_newline：dynamic 区接在已提交 static 之下时，第一行也要先换行另起。
            if i > 0 || lead_newline {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\x1b[2K");
            // 防御性裁剪：保证每个物理行 ≤ width（见 `clip_line_to_width`）。
            buffer.push_str(&clip_line_to_width(line, width));
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

    fn do_render(
        &mut self,
        new_lines: Vec<String>,
        cursor_pos: Option<(usize, usize)>,
        width: u16,
        height: usize,
    ) {
        let width_changed = self.previous_width != 0 && self.previous_width != width;
        let height_changed = self.previous_height != 0 && self.previous_height as usize != height;

        let previous_buffer_len = if self.previous_height > 0 {
            self.previous_viewport_top + self.previous_height as usize
        } else {
            height
        };
        let mut prev_viewport_top = if height_changed {
            previous_buffer_len.saturating_sub(height)
        } else {
            self.previous_viewport_top
        };
        let mut viewport_top = prev_viewport_top;
        let mut hardware_cursor_row = self.hardware_cursor_row;

        // 首帧：直接全量输出，不清屏（假设屏幕干净）
        if self.previous_lines.is_empty() && !width_changed && !height_changed {
            self.full_render(false, false, &new_lines, cursor_pos, width, height);
            return;
        }
        // 宽度变化：wrap 改变 → 全量重绘（清 scrollback）
        if width_changed {
            self.full_render(true, false, &new_lines, cursor_pos, width, height);
            return;
        }
        // 高度变化：对齐视口 → 全量重绘
        if height_changed {
            self.full_render(true, false, &new_lines, cursor_pos, width, height);
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
        let append_start = appended_lines
            && first_changed == self.previous_lines.len() as i64
            && first_changed > 0;

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
                    self.full_render(true, false, &new_lines, cursor_pos, width, height);
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
                    self.full_render(true, false, &new_lines, cursor_pos, width, height);
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
            self.full_render(true, false, &new_lines, cursor_pos, width, height);
            return;
        }

        // 差分输出
        let mut buffer = String::from("\x1b[?2026h");
        let prev_viewport_bottom = prev_viewport_top + height - 1;
        let move_target_row = if append_start {
            (first_changed - 1) as usize
        } else {
            first_changed as usize
        };
        if move_target_row > prev_viewport_bottom {
            let current_screen_row = ((hardware_cursor_row as i64 - prev_viewport_top as i64)
                .clamp(0, height as i64 - 1)) as usize;
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
            buffer.push_str("\x1b[2K"); // clear current line
                                        // 防御性裁剪：差分模型要求每个物理行 ≤ width。无论组件 emit 了什么（如 bash/rustc
                                        // 的超宽输出行），都先裁到 width 再写——绝不 panic、绝不让超宽行破坏 diff。
            let clipped = clip_line_to_width(&new_lines[i], width);
            buffer.push_str(&clipped);
            debug_assert!(
                visible_width(&clipped) <= width as usize,
                "clipped line {i} still exceeds terminal width ({} > {width})",
                visible_width(&clipped)
            );
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
        // synchronized output wrapping, no scrollback clear, lines joined by \r\n.
        // 每行前缀一个 \x1b[2K（防御性清行，双区重构后首帧也清行，保证 static 之下/屏幕残留不渗透）。
        assert!(out.starts_with("\x1b[?2026h"));
        assert!(out.ends_with("\x1b[?2026l"));
        assert!(!out.contains("\x1b[3J")); // no scrollback clear on first render
        assert!(out.contains("hello"));
        assert!(out.contains("\r\n\x1b[2Kworld"));
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

    /// 取一帧输出里被 `\x1b[2K`（清行）打头的各内容段，校验每段可见列 ≤ width。差分写出的每个
    /// 物理行都以 `\x1b[2K` 起头，故据此切出渲染器实际写到终端的行内容。每段在下一个 `\r\n`
    /// 或同步输出结束符 `\x1b[?2026l` 处截断（后者非标准 CSI final，`visible_width` 不识别，
    /// 须显式剥离，否则会把它当可见文本误计入宽度）。
    fn diff_written_lines(out: &str) -> Vec<String> {
        out.split("\x1b[2K")
            .skip(1) // 第一段是 `\x1b[2K` 之前的光标定位前缀，非内容行
            .map(|seg| {
                let seg = seg.split("\r\n").next().unwrap_or("");
                // 剥离行尾的同步输出结束符（`\x1b[?2026l` 不是 CSI m/G/K/H/J，visible_width 不跳过它）。
                seg.split("\x1b[?2026l").next().unwrap_or("").to_string()
            })
            .collect()
    }

    /// 首帧（full_render）路径：组件 emit 的超宽行必须被裁到 ≤ width，不破坏后续 diff。
    #[test]
    fn first_render_clips_overwide_line() {
        let width = 20u16;
        let mut tui = Tui::new(BufferTerminal::new(width, 5));
        let wide = "x".repeat(80); // 远超 20 列
        tui.add_child(Box::new(LineSource::new(vec![wide])));
        tui.render();
        let out = tui.terminal.take_output();
        // strip the synchronized-output / cursor frame, find the content line.
        // full_render joins lines with \r\n; the single content line is between
        // the `\x1b[?2026h` prefix and the `\x1b[?2026l` suffix.
        let body = out
            .trim_start_matches("\x1b[?2026h")
            .trim_end_matches("\x1b[?2026l");
        assert!(
            visible_width(body) <= width as usize,
            "first-render line not clipped: {} cols",
            visible_width(body)
        );
    }

    /// 差分写出路径：当一个改动行的可见宽度超过 width 时，不 panic（debug 也不），且写出的该行
    /// 字节可见列 ≤ width。覆盖纯 ASCII 超宽行。
    #[test]
    fn diff_clips_overwide_ascii_line_without_panic() {
        let width = 16u16;
        let mut tui = Tui::new(BufferTerminal::new(width, 5));
        tui.add_child(Box::new(LineSource::new(lines(&["short"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        // 改成一行远超 width 的 ASCII —— 旧代码会在 debug 下 panic。
        tui.set_lines(vec!["y".repeat(100)]);
        tui.render();
        let out = tui.terminal.take_output();
        for line in diff_written_lines(&out) {
            assert!(
                visible_width(&line) <= width as usize,
                "diff-written line exceeds width: {} cols ({line:?})",
                visible_width(&line)
            );
        }
    }

    /// 差分写出路径：CJK / 全角超宽行也被裁到 ≤ width（每个 CJK 字符占 2 列，按列裁剪而非字节）。
    #[test]
    fn diff_clips_overwide_cjk_line() {
        let width = 10u16;
        let mut tui = Tui::new(BufferTerminal::new(width, 5));
        tui.add_child(Box::new(LineSource::new(lines(&["短"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        // 30 个全角字符 = 60 可见列，远超 10。
        tui.set_lines(vec!["全角字符串".repeat(6)]);
        tui.render();
        let out = tui.terminal.take_output();
        for line in diff_written_lines(&out) {
            assert!(
                visible_width(&line) <= width as usize,
                "CJK diff line exceeds width: {} cols ({line:?})",
                visible_width(&line)
            );
        }
    }

    /// 差分写出路径：带 ANSI 颜色的超宽行 —— 转义序列不计宽，裁剪后可见列仍 ≤ width。
    #[test]
    fn diff_clips_overwide_ansi_colored_line() {
        let width = 12u16;
        let mut tui = Tui::new(BufferTerminal::new(width, 5));
        tui.add_child(Box::new(LineSource::new(lines(&["plain"]))));
        tui.render();
        let _ = tui.terminal.take_output();
        // 红色的 80 列文本：可见宽度 80，转义序列不计。
        let colored = format!("\x1b[31m{}\x1b[0m", "z".repeat(80));
        tui.set_lines(vec![colored]);
        tui.render();
        let out = tui.terminal.take_output();
        for line in diff_written_lines(&out) {
            assert!(
                visible_width(&line) <= width as usize,
                "ANSI diff line exceeds width: {} cols ({line:?})",
                visible_width(&line)
            );
        }
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

    fn frame(static_l: &[&str], dynamic_l: &[&str]) -> Frame {
        Frame {
            static_lines: lines(static_l),
            dynamic_lines: lines(dynamic_l),
        }
    }

    /// 双区：static 行随帧增长时，每行只提交（写出）一次进 scrollback，且不在后续帧重写。
    #[test]
    fn static_lines_committed_once() {
        let mut tui = Tui::new(BufferTerminal::new(40, 10));
        // 帧1：1 行 static + footer dynamic。
        tui.render_frame(frame(&["history-1"], &["footer"]));
        let out1 = tui.terminal.take_output();
        assert!(out1.contains("history-1"));
        // 帧2：static 增长到 2 行（history-1 已提交，不应再出现），新增 history-2。
        tui.render_frame(frame(&["history-1", "history-2"], &["footer"]));
        let out2 = tui.terminal.take_output();
        assert!(
            out2.contains("history-2"),
            "new static line must be committed"
        );
        assert!(
            !out2.contains("history-1"),
            "already-committed static must NOT be rewritten"
        );
        // 帧3：static 不变、dynamic 不变 —— 无输出。
        tui.render_frame(frame(&["history-1", "history-2"], &["footer"]));
        let out3 = tui.terminal.take_output();
        assert!(!out3.contains("history-1") && !out3.contains("history-2"));
    }

    /// 双区：首帧即带 static（如 resume 会话有历史）。首帧不清屏（假设屏幕干净）、static 提交一次、
    /// dynamic 画在其下，committed_static_count 推进到全部 static。
    #[test]
    fn first_frame_with_static_commits_without_clear() {
        let mut tui = Tui::new(BufferTerminal::new(40, 10));
        tui.render_frame(frame(&["resumed-1", "resumed-2"], &["prompt>", "footer"]));
        let out = tui.terminal.take_output();
        assert!(
            !out.contains("\x1b[3J"),
            "first frame must not clear scrollback"
        );
        assert!(
            out.contains("resumed-1") && out.contains("resumed-2"),
            "static committed"
        );
        assert!(
            out.contains("prompt>") && out.contains("footer"),
            "dynamic drawn below static"
        );
        assert_eq!(tui.committed_static_count, 2);
        // 下一帧仅 spinner 推进 → 已提交 static 不重写。
        tui.render_frame(frame(&["resumed-1", "resumed-2"], &["thinking…", "footer"]));
        let out2 = tui.terminal.take_output();
        assert!(!out2.contains("resumed-1") && !out2.contains("resumed-2"));
    }

    /// 双区：graduation 帧上旧 dynamic 比「新 static 尾段 ++ 新 dynamic」长时，必须清掉多出的尾行
    /// （否则旧 footer/editor 残留 → 重影）。验证 \x1b[2K 清行数覆盖到原 dynamic 全长。
    #[test]
    fn graduation_clears_shrunken_dynamic_tail() {
        let mut tui = Tui::new(BufferTerminal::new(40, 10));
        // 帧1：纯 dynamic，5 行（一个大 overlay + footer）。
        tui.render_frame(frame(&[], &["a", "b", "c", "d", "footer"]));
        let _ = tui.terminal.take_output();
        // 帧2：定稿出 1 行 static，dynamic 收缩到 2 行 → 旧 5 行 dynamic 多出的尾行必须被清。
        tui.render_frame(frame(&["item-done"], &["prompt>", "footer"]));
        let out = tui.terminal.take_output();
        assert!(out.contains("item-done"), "graduated static drawn");
        // 新内容 = 1 static + 2 dynamic = 3 行；旧 dynamic 5 行 → 需额外清 2 行（\r\n\x1b[2K ×2）。
        assert!(
            out.contains("\r\n\x1b[2K"),
            "extra stale dynamic tail lines cleared"
        );
        // 末尾把光标移回新内容末行（多走的 extra 行回退）。
        assert!(
            out.contains("\x1b[2A"),
            "cursor moved back up over cleared extra lines"
        );
    }

    /// 双区：dynamic-only 帧（无 static）→ 首次出现 static 前缀的「过渡帧」：新 static 必须**就地覆盖**
    /// 上一帧 dynamic（移上去重画），而非追加在旧 dynamic 之下。否则旧 editor/footer 会残留、内容错位。
    #[test]
    fn first_static_overwrites_prior_dynamic_in_place() {
        let mut tui = Tui::new(BufferTerminal::new(40, 10));
        // 帧1：纯 dynamic（welcome/editor/footer，无定稿前缀）。
        tui.render_frame(frame(&[], &["welcome", "prompt>", "footer"]));
        let _ = tui.terminal.take_output();
        // 帧2：用户消息定稿 → 首个 static 行出现；dynamic 收缩为 prompt+footer。
        tui.render_frame(frame(&["welcome", "> hello"], &["prompt>", "footer"]));
        let out = tui.terminal.take_output();
        // 过渡帧必须先把光标移回上一帧 dynamic 顶部（\x1b[{n}A）再就地覆盖，而不是只往下追加。
        assert!(
            out.contains("\x1b["),
            "must reposition cursor for in-place overwrite"
        );
        assert!(out.contains("> hello"), "new static line drawn");
        // 不应出现“在旧 footer 之下又写一遍 welcome”的重复。welcome 在帧1已画，帧2作为 static 提交一次。
        assert_eq!(out.matches("> hello").count(), 1);
    }

    /// 双区：static 定稿后 dynamic 区仍能最小重写（只改动行），不重写未变 dynamic 行。
    #[test]
    fn dynamic_region_minimal_rewrite_after_static() {
        let mut tui = Tui::new(BufferTerminal::new(40, 10));
        // 帧1：1 static + 2 dynamic（spinner + footer）。
        tui.render_frame(frame(&["history"], &["thinking… 0s", "footer"]));
        let _ = tui.terminal.take_output();
        // 帧2：static 不变；spinner 推进一相位，footer 不变。应只重写 spinner 行。
        tui.render_frame(frame(&["history"], &["thinking… 1s", "footer"]));
        let out = tui.terminal.take_output();
        assert!(
            !out.contains("history"),
            "committed static must not be touched"
        );
        assert!(out.contains("thinking… 1s"), "spinner line rewritten");
        // 未变的 footer 不应被重写（diff 只覆盖到首末改动行；footer 在 spinner 之后未变 → 不重写）。
        assert_eq!(
            out.matches("\x1b[2K").count(),
            1,
            "only the spinner line rewritten"
        );
    }

    /// 双区：CURSOR_MARKER 在 dynamic 区内被正确提取/剥离，硬件光标据此相对定位（IME 候选窗）。
    #[test]
    fn cursor_positioned_in_dynamic_after_static() {
        let mut tui = Tui::new(BufferTerminal::new(40, 10));
        tui.set_show_hardware_cursor(true);
        // dynamic 第二行带光标标记（列 5）。
        let editor = format!("edit{CURSOR_MARKER}line");
        tui.render_frame(Frame {
            static_lines: lines(&["history"]),
            dynamic_lines: vec!["prompt>".to_string(), editor, "footer".to_string()],
        });
        let out = tui.terminal.take_output();
        // 标记已剥离（不出现在输出里），且写出了去标记后的 editor 行。
        assert!(
            !out.contains(CURSOR_MARKER),
            "cursor marker must be stripped"
        );
        assert!(out.contains("editline"));
        // 出现绝对列定位（\x1b[{col}C），col = "edit" 的可见宽度 4 → \x1b[4C。
        assert!(
            out.contains("\x1b[4C"),
            "hardware cursor positioned at marker column"
        );
    }

    /// **核心 repro 回归测试**（修「generating 期间多行冻结 spinner 堆叠」）：
    /// height < 帧高，generating 期间同时存在「会增长的 static/历史输出」「spinner 连续多帧动画」
    /// 「追加触发滚动」三种情形。断言：spinner 行每帧只就地重写一次，**绝不被提交进 scrollback**，
    /// 历史行的提交写出永不夹带 spinner —— 即不会有冻结残留堆叠。
    #[test]
    fn spinner_never_frozen_in_scrollback_on_overflow() {
        // height = 6：远小于不断增长的历史帧高，制造溢出滚动。
        let mut tui = Tui::new(BufferTerminal::new(40, 6));
        let spinner_glyph = "⠋ thinking…";

        // 历史逐条定稿（模拟工具吐大段输出逐行进 scrollback），spinner 每帧重画，footer 恒定。
        let history: Vec<String> = (0..12).map(|i| format!("tool-output-line-{i}")).collect();
        let frame_count = history.len();
        let mut total_spinner_writes = 0usize;

        for f in 0..frame_count {
            // 已定稿前缀随帧增长（超过一屏 → 触发 scrollback 滚动）。
            let static_lines: Vec<String> = history[..=f].to_vec();
            let dynamic_lines = vec![spinner_glyph.to_string(), "footer".to_string()];
            tui.render_frame(Frame {
                static_lines,
                dynamic_lines,
            });
            let out = tui.terminal.take_output();
            // 不变式 A：本帧 spinner 至多写一次（就地重写）。滚动溢出绝不再多打一份 spinner 快照。
            let n = out.matches(spinner_glyph).count();
            assert!(
                n <= 1,
                "frame {f}: spinner written {n} times (frozen duplicate from scroll?)"
            );
            total_spinner_writes += n;
            // 不变式 B（物理根因，强断言）：本帧新定稿的历史行 tool-output-line-{f} 必须被写出（提交进
            // scrollback），且在字节流里**严格出现在 spinner 之前**——历史在 static 区（上方、随后滚入
            // scrollback），spinner 在其下的 dynamic 区。若有回归把 spinner 误判为 static，它会被写进
            // new_static_tail（在 footer 之后、且会被 committed 永久冻结），本断言即可捕获。
            let new_hist = format!("tool-output-line-{f}");
            let hist_pos = out
                .find(&new_hist)
                .expect("newly graduated history line must be committed");
            if let Some(sp_pos) = out.find(spinner_glyph) {
                assert!(
                    hist_pos < sp_pos,
                    "frame {f}: graduated history must be committed ABOVE the spinner (static→dynamic order)"
                );
            }
        }
        // 不变式 C（最强）：committed_static_count 单调走完全部历史，证明每条历史都只走 static 提交路径
        // 一次；spinner 从未被计入 static（否则计数会越过 history.len()）。
        assert_eq!(
            tui.committed_static_count, frame_count,
            "every history line committed exactly once via static path; spinner never counted as static"
        );

        // generating 全程：spinner 每帧恰好重写一次（无任何因滚动产生的冻结副本）。
        assert_eq!(
            total_spinner_writes, frame_count,
            "spinner must be written exactly once per generating frame (no frozen duplicates): \
             {total_spinner_writes} writes across {frame_count} frames"
        );

        // 收尾帧：generating 结束，spinner 从 dynamic 区消失。spinner 不再出现在任何输出里。
        tui.render_frame(Frame {
            static_lines: history.clone(),
            dynamic_lines: vec!["prompt>".to_string(), "footer".to_string()],
        });
        let final_out = tui.terminal.take_output();
        assert!(
            !final_out.contains(spinner_glyph),
            "after generating ends, spinner must be gone (not frozen anywhere): {final_out:?}"
        );
    }

    /// 双区：width 变化 → 全量重绘（清屏 + 清 scrollback），重新提交全部 static + 重画 dynamic。
    #[test]
    fn frame_width_change_full_redraw_recommits_static() {
        let mut tui = Tui::new(BufferTerminal::new(40, 10));
        tui.render_frame(frame(&["history-1", "history-2"], &["footer"]));
        let _ = tui.terminal.take_output();
        let redraws_before = tui.full_redraws();
        tui.terminal.set_size(30, 10);
        tui.render_frame(frame(&["history-1", "history-2"], &["footer"]));
        let out = tui.terminal.take_output();
        assert!(
            out.contains("\x1b[2J\x1b[H\x1b[3J"),
            "width change clears screen + scrollback"
        );
        assert!(
            out.contains("history-1") && out.contains("history-2"),
            "static re-committed"
        );
        assert!(out.contains("footer"), "dynamic re-drawn");
        assert!(tui.full_redraws() > redraws_before);
    }
}
