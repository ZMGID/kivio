//! StdinBuffer —— 把分块到达的原始 stdin 字节累积、切成「完整转义序列」。PI `stdin-buffer.ts` 端口。
//!
//! 终端的 stdin data 事件可能把一个转义序列（尤其鼠标 / CSI-u）切成多块到达。若不缓冲，半截
//! 序列会被误判成普通按键。本缓冲累积字节，仅在序列完整时产出；bracketed paste 整段作为一次
//! paste 产出。10ms 超时由驱动方在空闲时调 [`StdinBuffer::flush`] 模拟（这里不持有定时器）。

const ESC: char = '\x1b';
const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";

#[derive(Debug, PartialEq, Eq)]
enum Completeness {
    Complete,
    Incomplete,
    NotEscape,
}

fn is_complete_csi(data: &str) -> Completeness {
    if !data.starts_with("\x1b[") {
        return Completeness::Complete;
    }
    let chars: Vec<char> = data.chars().collect();
    if chars.len() < 3 {
        return Completeness::Incomplete;
    }
    let payload: String = chars[2..].iter().collect();
    let last = *chars.last().unwrap();
    let last_code = last as u32;
    if (0x40..=0x7e).contains(&last_code) {
        // SGR 鼠标序列特例：ESC[<B;X;Ym / ...M
        if payload.starts_with('<') {
            let inner = &payload[1..payload.len() - 1];
            let parts: Vec<&str> = inner.split(';').collect();
            if (last == 'M' || last == 'm')
                && parts.len() == 3
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            {
                return Completeness::Complete;
            }
            return Completeness::Incomplete;
        }
        return Completeness::Complete;
    }
    Completeness::Incomplete
}

fn is_complete_st_terminated(data: &str, prefix: &str) -> Completeness {
    if !data.starts_with(prefix) {
        return Completeness::Complete;
    }
    if data.ends_with("\x1b\\") || (prefix == "\x1b]" && data.ends_with('\x07')) {
        return Completeness::Complete;
    }
    Completeness::Incomplete
}

fn is_complete_sequence(data: &str) -> Completeness {
    if !data.starts_with(ESC) {
        return Completeness::NotEscape;
    }
    let chars: Vec<char> = data.chars().collect();
    if chars.len() == 1 {
        return Completeness::Incomplete;
    }
    match chars[1] {
        '[' => {
            // 老式鼠标：ESC[M + 3 字节 = 6 总
            if chars.len() >= 2 && chars[1] == '[' && chars.get(2) == Some(&'M') {
                return if chars.len() >= 6 {
                    Completeness::Complete
                } else {
                    Completeness::Incomplete
                };
            }
            is_complete_csi(data)
        }
        ']' => is_complete_st_terminated(data, "\x1b]"),
        'P' => is_complete_st_terminated(data, "\x1bP"),
        '_' => is_complete_st_terminated(data, "\x1b_"),
        'O' => {
            // SS3：ESC O 后一个字符
            if chars.len() >= 3 {
                Completeness::Complete
            } else {
                Completeness::Incomplete
            }
        }
        _ => {
            // meta 键：ESC + 单字符
            Completeness::Complete
        }
    }
}

/// 从缓冲里抽出所有完整序列，返回 (sequences, remainder)。
fn extract_complete_sequences(buffer: &str) -> (Vec<String>, String) {
    let chars: Vec<char> = buffer.chars().collect();
    let mut sequences: Vec<String> = Vec::new();
    let mut pos = 0usize;

    while pos < chars.len() {
        if chars[pos] == ESC {
            let mut seq_end = 1usize;
            let mut consumed = false;
            while pos + seq_end <= chars.len() {
                let candidate: String = chars[pos..pos + seq_end].iter().collect();
                match is_complete_sequence(&candidate) {
                    Completeness::Complete => {
                        // WezTerm 双 ESC 特例：'\x1b\x1b' 后跟一个会开启新序列的字符时，只产出第一个 ESC
                        if candidate == "\x1b\x1b" {
                            if let Some(next) = chars.get(pos + seq_end) {
                                if matches!(next, '[' | ']' | 'O' | 'P' | '_') {
                                    sequences.push(ESC.to_string());
                                    pos += 1;
                                    consumed = true;
                                    break;
                                }
                            }
                        }
                        sequences.push(candidate);
                        pos += seq_end;
                        consumed = true;
                        break;
                    }
                    Completeness::Incomplete => {
                        seq_end += 1;
                    }
                    Completeness::NotEscape => {
                        sequences.push(candidate);
                        pos += seq_end;
                        consumed = true;
                        break;
                    }
                }
            }
            if !consumed {
                // 走完仍不完整 —— 余下留作 remainder
                let remainder: String = chars[pos..].iter().collect();
                return (sequences, remainder);
            }
        } else {
            sequences.push(chars[pos].to_string());
            pos += 1;
        }
    }
    (sequences, String::new())
}

/// 单次 `process` 的产出。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StdinEvents {
    /// 完整的输入序列（每项一个按键 / 转义序列）。
    pub sequences: Vec<String>,
    /// bracketed paste 的整段内容（每项一次粘贴）。
    pub pastes: Vec<String>,
}

/// 累积 stdin 字节并切成完整序列；处理 bracketed paste。
#[derive(Default)]
pub struct StdinBuffer {
    buffer: String,
    paste_mode: bool,
    paste_buffer: String,
}

impl StdinBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段数据，返回本次能产出的完整序列与粘贴内容。未完成的尾部留在内部缓冲。
    pub fn process(&mut self, data: &str) -> StdinEvents {
        let mut events = StdinEvents::default();
        self.process_inner(data, &mut events);
        events
    }

    fn process_inner(&mut self, data: &str, events: &mut StdinEvents) {
        self.buffer.push_str(data);

        if self.paste_mode {
            self.paste_buffer.push_str(&self.buffer);
            self.buffer.clear();
            if let Some(idx) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let content = self.paste_buffer[..idx].to_string();
                let remaining = self.paste_buffer[idx + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                events.pastes.push(content);
                if !remaining.is_empty() {
                    self.process_inner(&remaining, events);
                }
            }
            return;
        }

        if let Some(start) = self.buffer.find(BRACKETED_PASTE_START) {
            if start > 0 {
                let before = self.buffer[..start].to_string();
                let (seqs, _) = extract_complete_sequences(&before);
                events.sequences.extend(seqs);
            }
            let rest = self.buffer[start + BRACKETED_PASTE_START.len()..].to_string();
            self.buffer.clear();
            self.paste_mode = true;
            self.paste_buffer = rest;

            if let Some(idx) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let content = self.paste_buffer[..idx].to_string();
                let remaining = self.paste_buffer[idx + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                events.pastes.push(content);
                if !remaining.is_empty() {
                    self.process_inner(&remaining, events);
                }
            }
            return;
        }

        let (seqs, remainder) = extract_complete_sequences(&self.buffer);
        self.buffer = remainder;
        events.sequences.extend(seqs);
    }

    /// 模拟 10ms 超时：把内部缓冲里残留的不完整序列强制作为一个序列产出。
    pub fn flush(&mut self) -> Vec<String> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        vec![std::mem::take(&mut self.buffer)]
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.paste_mode = false;
        self.paste_buffer.clear();
    }

    pub fn pending(&self) -> &str {
        &self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_chars_split_individually() {
        let mut b = StdinBuffer::new();
        let e = b.process("abc");
        assert_eq!(e.sequences, vec!["a", "b", "c"]);
        assert!(e.pastes.is_empty());
    }

    #[test]
    fn complete_csi_in_one_chunk() {
        let mut b = StdinBuffer::new();
        let e = b.process("\x1b[A");
        assert_eq!(e.sequences, vec!["\x1b[A"]);
    }

    #[test]
    fn csi_split_across_chunks() {
        let mut b = StdinBuffer::new();
        // SGR mouse arrives in pieces; should buffer until complete
        let e1 = b.process("\x1b");
        assert!(e1.sequences.is_empty());
        let e2 = b.process("[<35");
        assert!(e2.sequences.is_empty());
        let e3 = b.process(";20;5m");
        assert_eq!(e3.sequences, vec!["\x1b[<35;20;5m"]);
    }

    #[test]
    fn csi_u_complete() {
        let mut b = StdinBuffer::new();
        let e = b.process("\x1b[99;5u");
        assert_eq!(e.sequences, vec!["\x1b[99;5u"]);
    }

    #[test]
    fn bracketed_paste_single_chunk() {
        let mut b = StdinBuffer::new();
        let e = b.process("\x1b[200~hello world\x1b[201~");
        assert_eq!(e.pastes, vec!["hello world"]);
        assert!(e.sequences.is_empty());
    }

    #[test]
    fn bracketed_paste_with_surrounding_keys() {
        let mut b = StdinBuffer::new();
        let e = b.process("a\x1b[200~pasted\x1b[201~b");
        assert_eq!(e.sequences, vec!["a", "b"]);
        assert_eq!(e.pastes, vec!["pasted"]);
    }

    #[test]
    fn bracketed_paste_split_across_chunks() {
        let mut b = StdinBuffer::new();
        let e1 = b.process("\x1b[200~par");
        assert!(e1.pastes.is_empty());
        let e2 = b.process("tial\x1b[201~");
        assert_eq!(e2.pastes, vec!["partial"]);
    }

    #[test]
    fn incomplete_kept_until_flush() {
        let mut b = StdinBuffer::new();
        let e = b.process("\x1b[");
        assert!(e.sequences.is_empty());
        assert_eq!(b.pending(), "\x1b[");
        let flushed = b.flush();
        assert_eq!(flushed, vec!["\x1b["]);
        assert_eq!(b.pending(), "");
    }

    #[test]
    fn lone_escape_is_incomplete_then_flushes_as_escape() {
        // A bare ESC is the legal prefix of CSI/SS3/etc., so it is buffered as
        // Incomplete and NOT produced on `process`. The input thread's poll
        // timeout must `flush()` it so a standalone Esc key takes effect (close
        // overlay / cancel run). Regression guard for the swallowed-Esc bug.
        let mut b = StdinBuffer::new();
        let e = b.process("\x1b");
        assert!(e.sequences.is_empty(), "lone ESC must not emit immediately");
        assert_eq!(b.pending(), "\x1b");
        let flushed = b.flush();
        assert_eq!(flushed, vec!["\x1b"]);
        // And the flushed token must be recognized as the escape key.
        assert!(crate::kivio_code::tui::keys::matches_key(
            "\x1b", "escape", false
        ));
    }

    #[test]
    fn flush_empty_buffer_is_noop() {
        let mut b = StdinBuffer::new();
        assert!(b.flush().is_empty());
    }

    #[test]
    fn osc_terminated_by_bel() {
        let mut b = StdinBuffer::new();
        let e = b.process("\x1b]11;rgb:0000/0000/0000\x07");
        assert_eq!(e.sequences, vec!["\x1b]11;rgb:0000/0000/0000\x07"]);
    }

    #[test]
    fn old_mouse_six_bytes() {
        let mut b = StdinBuffer::new();
        let e = b.process("\x1b[M abc");
        assert_eq!(e.sequences[0], "\x1b[M ab");
    }

    #[test]
    fn wezterm_double_escape() {
        let mut b = StdinBuffer::new();
        // \x1b\x1b followed by [ should emit a lone ESC then parse the CSI
        let e = b.process("\x1b\x1b[27;5u");
        assert_eq!(e.sequences[0], "\x1b");
        assert_eq!(e.sequences[1], "\x1b[27;5u");
    }
}
