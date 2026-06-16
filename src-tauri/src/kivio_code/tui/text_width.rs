//! ANSI-aware 列宽 / wrap / truncate / slice 工具 —— PI `utils.ts` 的 Rust 端口。
//!
//! 差分渲染器的正确性完全建立在 [`visible_width`] 的精确之上：它跳过 ANSI/OSC/APC 转义
//! 序列，按 grapheme cluster 计宽（CJK / 全角 = 2，combining mark / 控制字符 = 0），tab = 3。
//! 所有 wrap / truncate / slice 都在「可见列」坐标系里工作，并尽量保留跨行的 SGR 状态。
//!
//! 实现上用 `unicode-segmentation` 取 grapheme cluster，用 `unicode-width` 取列宽，
//! 对应 PI 里的 `Intl.Segmenter` + `get-east-asian-width`。

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// 一个已识别的 ANSI/OSC/APC 转义序列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsiCode {
    /// 转义序列原文（含起始 `\x1b`）。
    pub code: String,
    /// 字节长度（== `code.len()`），调用方据此推进游标。
    pub length: usize,
}

/// 计算单个 grapheme cluster 的终端列宽。
///
/// - tab → 3
/// - 控制字符 / 零宽（combining marks、default-ignorable）→ 0
/// - CJK / 全角 / emoji → 2
/// - 其余 → `unicode-width` 计算的列宽
fn grapheme_width(segment: &str) -> usize {
    if segment == "\t" {
        return 3;
    }
    // `unicode-width` 已正确处理零宽（combining marks 计 0）与全角（CJK/emoji 计 2）。
    // 对纯控制字符（如 \x1b 单独出现，理论上调用方已剥离）兜底为 0。
    UnicodeWidthStr::width(segment)
}

#[inline]
fn is_printable_ascii(s: &str) -> bool {
    s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// 从 `bytes[pos..]`（按字符索引）提取一个 ANSI/OSC/APC 转义序列。
///
/// 支持：
/// - CSI：`ESC [ ... <final in m/G/K/H/J>`
/// - OSC：`ESC ] ... (BEL | ESC \)`（超链接、窗口标题等）
/// - APC：`ESC _ ... (BEL | ESC \)`（CURSOR_MARKER 等）
///
/// `chars` 必须是 `str` 的 `char` 切片视图；返回的 `length` 是 **char 数**。
fn extract_ansi_code_chars(chars: &[char], pos: usize) -> Option<AnsiCode> {
    if pos >= chars.len() || chars[pos] != '\x1b' {
        return None;
    }
    let next = chars.get(pos + 1).copied();

    match next {
        // CSI sequence: ESC [ ... m/G/K/H/J
        Some('[') => {
            let mut j = pos + 2;
            while j < chars.len() && !matches!(chars[j], 'm' | 'G' | 'K' | 'H' | 'J') {
                j += 1;
            }
            if j < chars.len() {
                let code: String = chars[pos..=j].iter().collect();
                Some(AnsiCode { code, length: j + 1 - pos })
            } else {
                None
            }
        }
        // OSC sequence: ESC ] ... BEL or ESC ] ... ST (ESC \)
        Some(']') | Some('_') => {
            let mut j = pos + 2;
            while j < chars.len() {
                if chars[j] == '\x07' {
                    let code: String = chars[pos..=j].iter().collect();
                    return Some(AnsiCode { code, length: j + 1 - pos });
                }
                if chars[j] == '\x1b' && chars.get(j + 1) == Some(&'\\') {
                    let code: String = chars[pos..=j + 1].iter().collect();
                    return Some(AnsiCode { code, length: j + 2 - pos });
                }
                j += 1;
            }
            None
        }
        _ => None,
    }
}

/// 从字符串 `s` 的字节位置 `byte_pos` 提取一个 ANSI 序列（公开 API，按字节定位）。
///
/// 注意：`length` 字段返回的是 **字符数**，与 `code.chars().count()` 一致；若需字节长度用
/// `code.len()`。内部渲染逻辑统一用 char 切片，避免多字节边界问题。
pub fn extract_ansi_code(s: &str, byte_pos: usize) -> Option<AnsiCode> {
    let chars: Vec<char> = s.chars().collect();
    // 把 byte_pos 映射到 char 索引
    let mut char_pos = 0usize;
    let mut acc = 0usize;
    for (i, c) in s.chars().enumerate() {
        if acc >= byte_pos {
            char_pos = i;
            break;
        }
        acc += c.len_utf8();
        char_pos = i + 1;
    }
    extract_ansi_code_chars(&chars, char_pos)
}

/// 计算字符串的可见终端列宽（跳过 ANSI/OSC/APC，tab=3）。
pub fn visible_width(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    if is_printable_ascii(s) {
        return s.len();
    }

    // 规整：tab → 3 空格，剥离转义序列
    let chars: Vec<char> = s.chars().collect();
    let mut clean = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' {
            if let Some(ansi) = extract_ansi_code_chars(&chars, i) {
                i += ansi.length;
                continue;
            }
        }
        if chars[i] == '\t' {
            clean.push_str("   ");
            i += 1;
            continue;
        }
        clean.push(chars[i]);
        i += 1;
    }

    let mut width = 0;
    for g in clean.graphemes(true) {
        width += grapheme_width(g);
    }
    width
}

/// PI `normalizeTerminalOutput` 的端口：把预组合的泰文 / 老挝文 AM 元音替换为兼容分解形式，
/// 避免部分终端在差分重绘时留下脏 cell。列宽不变，仅替换字符。
pub fn normalize_terminal_output(s: &str) -> String {
    if !s.contains('\u{0e33}') && !s.contains('\u{0eb3}') {
        return s.to_string();
    }
    s.chars()
        .flat_map(|c| match c {
            '\u{0e33}' => vec!['\u{0e4d}', '\u{0e32}'],
            '\u{0eb3}' => vec!['\u{0ecd}', '\u{0eb2}'],
            other => vec![other],
        })
        .collect()
}

// =============================================================================
// ANSI SGR 状态跟踪（跨行保留样式）
// =============================================================================

#[derive(Clone)]
struct ActiveHyperlink {
    params: String,
    url: String,
    /// 终止符：BEL(`\x07`) 或 ST(`\x1b\\`)。
    st_terminator: bool,
}

/// 跟踪当前活跃的 SGR 属性，用于 wrap 时在新行行首重新施加样式、行尾关闭下划线 / 超链接。
/// 对应 PI 的 `AnsiCodeTracker`。
#[derive(Default)]
pub struct AnsiCodeTracker {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strikethrough: bool,
    fg_color: Option<String>,
    bg_color: Option<String>,
    hyperlink: Option<ActiveHyperlink>,
}

impl AnsiCodeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset_sgr(&mut self) {
        self.bold = false;
        self.dim = false;
        self.italic = false;
        self.underline = false;
        self.blink = false;
        self.inverse = false;
        self.hidden = false;
        self.strikethrough = false;
        self.fg_color = None;
        self.bg_color = None;
        // SGR reset 不影响 OSC 8 超链接状态
    }

    /// 清空全部状态（含超链接）以复用。
    pub fn clear(&mut self) {
        self.reset_sgr();
        self.hyperlink = None;
    }

    fn parse_osc8(&mut self, code: &str) -> bool {
        if !code.starts_with("\x1b]8;") {
            return false;
        }
        let st = code.ends_with('\x07');
        let body = if st {
            &code[4..code.len() - 1]
        } else {
            // ends with ESC \
            &code[4..code.len() - 2]
        };
        if let Some(sep) = body.find(';') {
            let params = body[..sep].to_string();
            let url = body[sep + 1..].to_string();
            if url.is_empty() {
                // close
                self.hyperlink = None;
            } else {
                self.hyperlink = Some(ActiveHyperlink { params, url, st_terminator: st });
            }
        }
        true
    }

    /// 处理一个转义序列，更新内部状态。
    pub fn process(&mut self, code: &str) {
        if self.parse_osc8(code) {
            return;
        }
        if !code.ends_with('m') {
            return;
        }
        // 提取 ESC[ 与 m 之间的参数
        let inner = &code[2..code.len() - 1];
        if inner.is_empty() || inner == "0" {
            self.reset_sgr();
            return;
        }
        let parts: Vec<&str> = inner.split(';').collect();
        let mut i = 0;
        while i < parts.len() {
            let n: i64 = parts[i].parse().unwrap_or(-1);
            // 256-color / RGB：38/48 消耗多个参数
            if (n == 38 || n == 48) && i + 1 < parts.len() {
                if parts[i + 1] == "5" && i + 2 < parts.len() {
                    let color = format!("{};{};{}", parts[i], parts[i + 1], parts[i + 2]);
                    if n == 38 {
                        self.fg_color = Some(color);
                    } else {
                        self.bg_color = Some(color);
                    }
                    i += 3;
                    continue;
                } else if parts[i + 1] == "2" && i + 4 < parts.len() {
                    let color = format!(
                        "{};{};{};{};{}",
                        parts[i],
                        parts[i + 1],
                        parts[i + 2],
                        parts[i + 3],
                        parts[i + 4]
                    );
                    if n == 38 {
                        self.fg_color = Some(color);
                    } else {
                        self.bg_color = Some(color);
                    }
                    i += 5;
                    continue;
                }
            }
            match n {
                0 => self.reset_sgr(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                5 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strikethrough = true,
                21 => self.bold = false,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strikethrough = false,
                39 => self.fg_color = None,
                49 => self.bg_color = None,
                30..=37 | 90..=97 => self.fg_color = Some(n.to_string()),
                40..=47 | 100..=107 => self.bg_color = Some(n.to_string()),
                _ => {}
            }
            i += 1;
        }
    }

    /// 返回重新施加当前活跃样式的转义序列（用于 wrap 新行行首）。
    pub fn active_codes(&self) -> String {
        let mut codes: Vec<String> = Vec::new();
        if self.bold {
            codes.push("1".into());
        }
        if self.dim {
            codes.push("2".into());
        }
        if self.italic {
            codes.push("3".into());
        }
        if self.underline {
            codes.push("4".into());
        }
        if self.blink {
            codes.push("5".into());
        }
        if self.inverse {
            codes.push("7".into());
        }
        if self.hidden {
            codes.push("8".into());
        }
        if self.strikethrough {
            codes.push("9".into());
        }
        if let Some(fg) = &self.fg_color {
            codes.push(fg.clone());
        }
        if let Some(bg) = &self.bg_color {
            codes.push(bg.clone());
        }
        let mut result = if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        };
        if let Some(h) = &self.hyperlink {
            let term = if h.st_terminator { "\x07" } else { "\x1b\\" };
            result.push_str(&format!("\x1b]8;{};{}{}", h.params, h.url, term));
        }
        result
    }

    /// 行尾需要关闭的属性（下划线必须关，避免渗入 padding；活跃超链接需关闭再于下一行重开）。
    pub fn line_end_reset(&self) -> String {
        let mut result = String::new();
        if self.underline {
            result.push_str("\x1b[24m");
        }
        if let Some(h) = &self.hyperlink {
            let term = if h.st_terminator { "\x07" } else { "\x1b\\" };
            result.push_str(&format!("\x1b]8;;{term}"));
        }
        result
    }
}

fn update_tracker_from_text(text: &str, tracker: &mut AnsiCodeTracker) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if let Some(ansi) = extract_ansi_code_chars(&chars, i) {
            tracker.process(&ansi.code);
            i += ansi.length;
        } else {
            i += 1;
        }
    }
}

// =============================================================================
// Word wrap（保留 ANSI）
// =============================================================================

/// 判断一个 grapheme 是否属于 CJK 系（可在相邻 CJK 之间断行）。
fn is_cjk(segment: &str) -> bool {
    segment.chars().next().is_some_and(|c| {
        let cp = c as u32;
        // Han, Hiragana, Katakana, Hangul syllables/jamo, Bopomofo, CJK 标点
        (0x3000..=0x303f).contains(&cp)   // CJK 符号标点
            || (0x3040..=0x30ff).contains(&cp) // 平假名 + 片假名
            || (0x3100..=0x312f).contains(&cp) // 注音
            || (0x3400..=0x4dbf).contains(&cp) // CJK 扩展 A
            || (0x4e00..=0x9fff).contains(&cp) // CJK 统一表意
            || (0xac00..=0xd7af).contains(&cp) // 谚文音节
            || (0xf900..=0xfaff).contains(&cp) // CJK 兼容表意
            || (0xff00..=0xffef).contains(&cp) // 全角形式
    })
}

/// 把一行（无内嵌换行）按词切成 token，ANSI 序列附着到其后第一个可见字符；
/// CJK 字符各自成 token（允许相邻断行）。
fn split_into_tokens_with_ansi(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut pending_ansi = String::new();
    // None / Some(true)=space / Some(false)=word
    let mut current_kind: Option<bool> = None;
    let mut i = 0;

    let flush = |tokens: &mut Vec<String>, current: &mut String, kind: &mut Option<bool>| {
        if !current.is_empty() {
            tokens.push(std::mem::take(current));
            *kind = None;
        }
    };

    while i < chars.len() {
        if let Some(ansi) = extract_ansi_code_chars(&chars, i) {
            pending_ansi.push_str(&ansi.code);
            i += ansi.length;
            continue;
        }
        // 找到下一段非 ANSI 文本
        let mut end = i;
        while end < chars.len() && extract_ansi_code_chars(&chars, end).is_none() {
            end += 1;
        }
        let chunk: String = chars[i..end].iter().collect();
        for g in chunk.graphemes(true) {
            let is_space = g == " ";
            if !is_space && is_cjk(g) {
                flush(&mut tokens, &mut current, &mut current_kind);
                let mut token = std::mem::take(&mut pending_ansi);
                token.push_str(g);
                tokens.push(token);
                continue;
            }
            let kind = !is_space; // word=true, space=false
            if !current.is_empty() && current_kind != Some(kind) {
                flush(&mut tokens, &mut current, &mut current_kind);
            }
            if !pending_ansi.is_empty() {
                current.push_str(&pending_ansi);
                pending_ansi.clear();
            }
            current_kind = Some(kind);
            current.push_str(g);
        }
        i = end;
    }

    if !pending_ansi.is_empty() {
        if !current.is_empty() {
            current.push_str(&pending_ansi);
        } else if let Some(last) = tokens.last_mut() {
            last.push_str(&pending_ansi);
        } else {
            current = pending_ansi.clone();
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn break_long_word(word: &str, width: usize, tracker: &mut AnsiCodeTracker) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = tracker.active_codes();
    let mut current_width = 0usize;

    // 把 word 拆成 (ansi | grapheme) 段
    enum Seg {
        Ansi(String),
        Grapheme(String),
    }
    let mut segments: Vec<Seg> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(ansi) = extract_ansi_code_chars(&chars, i) {
            segments.push(Seg::Ansi(ansi.code.clone()));
            i += ansi.length;
        } else {
            let mut end = i;
            while end < chars.len() && extract_ansi_code_chars(&chars, end).is_none() {
                end += 1;
            }
            let portion: String = chars[i..end].iter().collect();
            for g in portion.graphemes(true) {
                segments.push(Seg::Grapheme(g.to_string()));
            }
            i = end;
        }
    }

    for seg in segments {
        match seg {
            Seg::Ansi(code) => {
                current_line.push_str(&code);
                tracker.process(&code);
            }
            Seg::Grapheme(g) => {
                if g.is_empty() {
                    continue;
                }
                let gw = visible_width(&g);
                if current_width + gw > width {
                    let reset = tracker.line_end_reset();
                    if !reset.is_empty() {
                        current_line.push_str(&reset);
                    }
                    lines.push(std::mem::take(&mut current_line));
                    current_line = tracker.active_codes();
                    current_width = 0;
                }
                current_line.push_str(&g);
                current_width += gw;
            }
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if visible_width(line) <= width {
        return vec![line.to_string()];
    }

    let mut wrapped: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::new();
    let tokens = split_into_tokens_with_ansi(line);

    let mut current_line = String::new();
    let mut current_visible = 0usize;

    for token in &tokens {
        let token_w = visible_width(token);
        let is_ws = token.trim().is_empty();

        // token 自身超宽 —— 逐字符断行
        if token_w > width && !is_ws {
            if !current_line.is_empty() {
                let reset = tracker.line_end_reset();
                if !reset.is_empty() {
                    current_line.push_str(&reset);
                }
                wrapped.push(std::mem::take(&mut current_line));
            }
            let broken = break_long_word(token, width, &mut tracker);
            for b in &broken[..broken.len() - 1] {
                wrapped.push(b.clone());
            }
            current_line = broken[broken.len() - 1].clone();
            current_visible = visible_width(&current_line);
            continue;
        }

        let total = current_visible + token_w;
        if total > width && current_visible > 0 {
            let mut line_to_wrap = current_line.trim_end().to_string();
            let reset = tracker.line_end_reset();
            if !reset.is_empty() {
                line_to_wrap.push_str(&reset);
            }
            wrapped.push(line_to_wrap);
            if is_ws {
                current_line = tracker.active_codes();
                current_visible = 0;
            } else {
                current_line = tracker.active_codes();
                current_line.push_str(token);
                current_visible = token_w;
            }
        } else {
            current_line.push_str(token);
            current_visible += token_w;
        }
        update_tracker_from_text(token, &mut tracker);
    }

    if !current_line.is_empty() {
        wrapped.push(current_line);
    }

    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped.iter().map(|l| l.trim_end().to_string()).collect()
    }
}

/// 把文本按 `width` 个可见列 word-wrap，跨行保留 ANSI 状态。**不** padding、**不** 加背景色。
/// 返回每行均 ≤ width 可见列（除非单个 grapheme 本身超宽）。
pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let input_lines: Vec<&str> = text.split('\n').collect();
    let mut result: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::new();

    for input_line in &input_lines {
        let prefix = if result.is_empty() { String::new() } else { tracker.active_codes() };
        let combined = format!("{prefix}{input_line}");
        for wl in wrap_single_line(&combined, width) {
            result.push(wl);
        }
        update_tracker_from_text(input_line, &mut tracker);
    }
    if result.is_empty() {
        vec![String::new()]
    } else {
        result
    }
}

/// 给一行加背景色并 padding 到 `width` 列。`bg_fn` 接收「内容+padding」返回带背景的字符串。
pub fn apply_background_to_line(line: &str, width: usize, bg_fn: &dyn Fn(&str) -> String) -> String {
    let vis = visible_width(line);
    let pad = width.saturating_sub(vis);
    let with_padding = format!("{}{}", line, " ".repeat(pad));
    bg_fn(&with_padding)
}

// =============================================================================
// Truncate
// =============================================================================

fn finalize_truncated(
    prefix: &str,
    prefix_width: usize,
    ellipsis: &str,
    ellipsis_width: usize,
    max_width: usize,
    pad: bool,
) -> String {
    let reset = "\x1b[0m";
    let visible = prefix_width + ellipsis_width;
    let mut result = if !ellipsis.is_empty() {
        format!("{prefix}{reset}{ellipsis}{reset}")
    } else {
        format!("{prefix}{reset}")
    };
    if pad {
        result.push_str(&" ".repeat(max_width.saturating_sub(visible)));
    }
    result
}

fn truncate_fragment(text: &str, max_width: usize) -> (String, usize) {
    if max_width == 0 || text.is_empty() {
        return (String::new(), 0);
    }
    let mut result = String::new();
    let mut width = 0;
    for g in text.graphemes(true) {
        let w = grapheme_width(g);
        if width + w > max_width {
            break;
        }
        result.push_str(g);
        width += w;
    }
    (result, width)
}

/// 把文本截断到 ≤ `max_width` 可见列；超出时追加 `ellipsis`；`pad=true` 时空格补齐到 max_width。
/// ANSI 序列不计入宽度。
pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str, pad: bool) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.is_empty() {
        return if pad { " ".repeat(max_width) } else { String::new() };
    }

    let ellipsis_width = visible_width(ellipsis);
    if ellipsis_width >= max_width {
        let text_width = visible_width(text);
        if text_width <= max_width {
            return if pad {
                format!("{}{}", text, " ".repeat(max_width - text_width))
            } else {
                text.to_string()
            };
        }
        let (clipped, w) = truncate_fragment(ellipsis, max_width);
        if w == 0 {
            return if pad { " ".repeat(max_width) } else { String::new() };
        }
        return finalize_truncated("", 0, &clipped, w, max_width, pad);
    }

    let target_width = max_width - ellipsis_width;
    let chars: Vec<char> = text.chars().collect();

    let mut result = String::new();
    let mut pending_ansi = String::new();
    let mut visible_so_far = 0usize;
    let mut kept_width = 0usize;
    let mut keep_prefix = true;
    let mut overflowed = false;
    let mut i = 0;
    while i < chars.len() {
        if let Some(ansi) = extract_ansi_code_chars(&chars, i) {
            pending_ansi.push_str(&ansi.code);
            i += ansi.length;
            continue;
        }
        if chars[i] == '\t' {
            if keep_prefix && kept_width + 3 <= target_width {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push('\t');
                kept_width += 3;
            } else {
                keep_prefix = false;
                pending_ansi.clear();
            }
            visible_so_far += 3;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && chars[end] != '\t' && extract_ansi_code_chars(&chars, end).is_none() {
            end += 1;
        }
        let chunk: String = chars[i..end].iter().collect();
        for g in chunk.graphemes(true) {
            let w = grapheme_width(g);
            if keep_prefix && kept_width + w <= target_width {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push_str(g);
                kept_width += w;
            } else {
                keep_prefix = false;
                pending_ansi.clear();
            }
            visible_so_far += w;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }
        }
        if overflowed {
            break;
        }
        i = end;
    }
    let exhausted = !overflowed && i >= chars.len();

    if !overflowed && exhausted {
        return if pad {
            format!("{}{}", text, " ".repeat(max_width.saturating_sub(visible_so_far)))
        } else {
            text.to_string()
        };
    }
    finalize_truncated(&result, kept_width, ellipsis, ellipsis_width, max_width, pad)
}

// =============================================================================
// Slice by column
// =============================================================================

/// 从 `line` 提取 `[start_col, start_col+length)` 范围的可见列，ANSI-aware。
/// `strict=true` 时排除会越界的宽字符。返回 (text, visible_width)。
pub fn slice_with_width(line: &str, start_col: usize, length: usize, strict: bool) -> (String, usize) {
    if length == 0 {
        return (String::new(), 0);
    }
    let end_col = start_col + length;
    let chars: Vec<char> = line.chars().collect();
    let mut result = String::new();
    let mut result_width = 0usize;
    let mut current_col = 0usize;
    let mut i = 0;
    let mut pending_ansi = String::new();

    while i < chars.len() {
        if let Some(ansi) = extract_ansi_code_chars(&chars, i) {
            if current_col >= start_col && current_col < end_col {
                result.push_str(&ansi.code);
            } else if current_col < start_col {
                pending_ansi.push_str(&ansi.code);
            }
            i += ansi.length;
            continue;
        }
        let mut end = i;
        while end < chars.len() && extract_ansi_code_chars(&chars, end).is_none() {
            end += 1;
        }
        let chunk: String = chars[i..end].iter().collect();
        let mut broke = false;
        for g in chunk.graphemes(true) {
            let w = grapheme_width(g);
            let in_range = current_col >= start_col && current_col < end_col;
            let fits = !strict || current_col + w <= end_col;
            if in_range && fits {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push_str(g);
                result_width += w;
            }
            current_col += w;
            if current_col >= end_col {
                broke = true;
                break;
            }
        }
        i = end;
        if broke || current_col >= end_col {
            break;
        }
    }
    (result, result_width)
}

/// 同 [`slice_with_width`] 但只返回文本。
pub fn slice_by_column(line: &str, start_col: usize, length: usize, strict: bool) -> String {
    slice_with_width(line, start_col, length, strict).0
}

/// 一次扫描提取 before / after 两段（overlay 横向拼接用）。after 继承 overlay 之前的样式。
pub fn extract_segments(
    line: &str,
    before_end: usize,
    after_start: usize,
    after_len: usize,
    strict_after: bool,
) -> (String, usize, String, usize) {
    let chars: Vec<char> = line.chars().collect();
    let mut before = String::new();
    let mut before_width = 0usize;
    let mut after = String::new();
    let mut after_width = 0usize;
    let mut current_col = 0usize;
    let mut i = 0;
    let mut pending_before = String::new();
    let mut after_started = false;
    let after_end = after_start + after_len;
    let mut tracker = AnsiCodeTracker::new();

    while i < chars.len() {
        if let Some(ansi) = extract_ansi_code_chars(&chars, i) {
            tracker.process(&ansi.code);
            if current_col < before_end {
                pending_before.push_str(&ansi.code);
            } else if current_col >= after_start && current_col < after_end && after_started {
                after.push_str(&ansi.code);
            }
            i += ansi.length;
            continue;
        }
        let mut end = i;
        while end < chars.len() && extract_ansi_code_chars(&chars, end).is_none() {
            end += 1;
        }
        let chunk: String = chars[i..end].iter().collect();
        let mut broke = false;
        for g in chunk.graphemes(true) {
            let w = grapheme_width(g);
            if current_col < before_end && current_col + w <= before_end {
                if !pending_before.is_empty() {
                    before.push_str(&pending_before);
                    pending_before.clear();
                }
                before.push_str(g);
                before_width += w;
            } else if current_col >= after_start && current_col < after_end {
                let fits = !strict_after || current_col + w <= after_end;
                if fits {
                    if !after_started {
                        after.push_str(&tracker.active_codes());
                        after_started = true;
                    }
                    after.push_str(g);
                    after_width += w;
                }
            }
            current_col += w;
            let done = if after_len == 0 { current_col >= before_end } else { current_col >= after_end };
            if done {
                broke = true;
                break;
            }
        }
        i = end;
        let done = if after_len == 0 { current_col >= before_end } else { current_col >= after_end };
        if broke || done {
            break;
        }
    }
    (before, before_width, after, after_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_width_ascii() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width(""), 0);
        assert_eq!(visible_width("a b c"), 5);
    }

    #[test]
    fn visible_width_cjk_double() {
        // 每个 CJK 字符 2 列
        assert_eq!(visible_width("你好"), 4);
        assert_eq!(visible_width("a你b"), 4);
        // 全角形式
        assert_eq!(visible_width("Ａ"), 2);
    }

    #[test]
    fn visible_width_combining_marks() {
        // e + combining acute accent = 1 cell
        assert_eq!(visible_width("e\u{0301}"), 1);
        // 多个 combining
        assert_eq!(visible_width("a\u{0301}\u{0302}"), 1);
    }

    #[test]
    fn visible_width_strips_ansi() {
        assert_eq!(visible_width("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(visible_width("\x1b[1;32mhi\x1b[0m there"), 8);
        // 256-color
        assert_eq!(visible_width("\x1b[38;5;240mx\x1b[0m"), 1);
        // OSC 8 hyperlink
        assert_eq!(visible_width("\x1b]8;;https://x.com\x07link\x1b]8;;\x07"), 4);
    }

    #[test]
    fn visible_width_tab_is_three() {
        assert_eq!(visible_width("\tx"), 4);
    }

    #[test]
    fn visible_width_cursor_marker() {
        // APC marker is zero-width
        assert_eq!(visible_width("ab\x1b_pi:c\x07cd"), 4);
    }

    #[test]
    fn extract_ansi_csi() {
        let code = extract_ansi_code("\x1b[31mx", 0).unwrap();
        assert_eq!(code.code, "\x1b[31m");
        assert_eq!(code.length, 5);
    }

    #[test]
    fn extract_ansi_osc_bel() {
        let code = extract_ansi_code("\x1b]8;;url\x07rest", 0).unwrap();
        assert_eq!(code.code, "\x1b]8;;url\x07");
    }

    #[test]
    fn extract_ansi_apc_marker() {
        let code = extract_ansi_code("\x1b_pi:c\x07", 0).unwrap();
        assert_eq!(code.code, "\x1b_pi:c\x07");
    }

    #[test]
    fn extract_ansi_none_for_plain() {
        assert!(extract_ansi_code("hello", 0).is_none());
    }

    #[test]
    fn wrap_basic() {
        let lines = wrap_text_with_ansi("the quick brown fox", 9);
        for l in &lines {
            assert!(visible_width(l) <= 9, "line too wide: {l:?}");
        }
        assert_eq!(lines, vec!["the quick", "brown fox"]);
    }

    #[test]
    fn wrap_long_word_breaks() {
        let lines = wrap_text_with_ansi("abcdefghij", 4);
        assert_eq!(lines, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_preserves_newlines() {
        let lines = wrap_text_with_ansi("a\nb", 10);
        assert_eq!(lines, vec!["a", "b"]);
    }

    #[test]
    fn wrap_preserves_ansi_across_lines() {
        // bold should re-open on the second wrapped line
        let lines = wrap_text_with_ansi("\x1b[1mhello world foo\x1b[0m", 6);
        for l in &lines {
            assert!(visible_width(l) <= 6);
        }
        // second line should carry the bold code
        assert!(lines.len() >= 2);
        assert!(lines[1].contains("\x1b[1m"));
    }

    #[test]
    fn wrap_cjk_breaks_between() {
        let lines = wrap_text_with_ansi("你好世界", 4);
        // each CJK char is 2 cols, so 2 per line
        for l in &lines {
            assert!(visible_width(l) <= 4);
        }
        assert_eq!(lines, vec!["你好", "世界"]);
    }

    #[test]
    fn truncate_no_change_when_fits() {
        assert_eq!(truncate_to_width("hi", 10, "...", false), "hi");
    }

    #[test]
    fn truncate_adds_ellipsis() {
        let r = truncate_to_width("hello world", 8, "...", false);
        assert!(visible_width(&r) <= 8);
        assert!(r.contains("..."));
    }

    #[test]
    fn truncate_pads_to_width() {
        let r = truncate_to_width("hi", 5, "...", true);
        assert_eq!(visible_width(&r), 5);
    }

    #[test]
    fn truncate_cjk() {
        let r = truncate_to_width("你好世界", 5, "…", false);
        assert!(visible_width(&r) <= 5);
    }

    #[test]
    fn slice_basic() {
        assert_eq!(slice_by_column("hello", 1, 3, false), "ell");
        assert_eq!(slice_by_column("hello", 0, 5, false), "hello");
    }

    #[test]
    fn slice_preserves_ansi_before() {
        // color set before the slice window should carry into the slice
        let r = slice_by_column("\x1b[31mhello\x1b[0m", 1, 2, false);
        assert!(r.contains("\x1b[31m"));
        assert_eq!(visible_width(&r), 2);
    }

    #[test]
    fn slice_strict_excludes_wide_at_boundary() {
        // "a你b": cols a=0, 你=1..3, b=3. slice [0,2) strict should drop 你 (would end at col 3 > 2)
        let r = slice_with_width("a你b", 0, 2, true);
        assert_eq!(r.0, "a");
        assert_eq!(r.1, 1);
    }

    #[test]
    fn apply_background_pads() {
        let bg = |s: &str| format!("[{s}]");
        let r = apply_background_to_line("hi", 5, &bg);
        assert_eq!(r, "[hi   ]");
    }

    #[test]
    fn normalize_thai_am() {
        let r = normalize_terminal_output("\u{0e33}");
        assert_eq!(r, "\u{0e4d}\u{0e32}");
        // unaffected text untouched
        assert_eq!(normalize_terminal_output("abc"), "abc");
    }

    #[test]
    fn extract_segments_splices() {
        // before [0,2), after starts at 4
        let (before, bw, after, aw) = extract_segments("abcdefgh", 2, 4, 2, true);
        assert_eq!(before, "ab");
        assert_eq!(bw, 2);
        assert_eq!(after, "ef");
        assert_eq!(aw, 2);
    }
}
