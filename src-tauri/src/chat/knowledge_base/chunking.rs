//! Text chunking for the knowledge base.
//!
//! Recursive, heading-aware chunking with a token budget. Token counting is a
//! cheap heuristic that **counts each CJK char as ~1 token and ~4 Latin chars
//! as 1 token** — using the English 4-chars/token rule on Chinese badly
//! under-counts and produces oversized chunks (see PRD §4.4). The estimate is
//! only used for sizing, so being approximate is fine; it just must not wildly
//! undershoot on CJK.

pub const TARGET_TOKENS: usize = 480;
pub const OVERLAP_TOKENS: usize = 64;
pub const MIN_TOKENS: usize = 80;

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkPiece {
    pub text: String,
    pub char_start: usize,
    pub char_end: usize,
    pub heading_path: Option<String>,
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x3040..=0x30FF // Hiragana + Katakana
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK compat ideographs
        | 0x3000..=0x303F // CJK punctuation
        | 0xFF00..=0xFFEF // full-width forms
    )
}

/// Cheap token estimate. CJK chars ≈ 1 token each; other chars ≈ 1/4 token.
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        if is_cjk(ch) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + other.div_ceil(4)
}

/// A markdown ATX heading line like `## Title` → (level, title). Returns None
/// for non-heading lines.
fn parse_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = trimmed[hashes..].trim();
    if rest.is_empty() || !trimmed[hashes..].starts_with(char::is_whitespace) {
        return None;
    }
    Some((hashes, rest.to_string()))
}

fn heading_path(stack: &[(usize, String)]) -> Option<String> {
    if stack.is_empty() {
        None
    } else {
        Some(
            stack
                .iter()
                .map(|(_, t)| t.as_str())
                .collect::<Vec<_>>()
                .join(" > "),
        )
    }
}

/// Split one over-long line into char-window sub-pieces, preferring to break
/// after sentence-ending punctuation. Offsets are char indices into the full
/// document (the caller passes `base` = the line's char start).
fn split_long_line(line: &str, base: usize, target: usize) -> Vec<(String, usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let piece_len = i - start + 1;
        let at_breakpoint = matches!(
            chars[i],
            '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';' | '\n'
        );
        let piece_tokens = estimate_tokens(&chars[start..=i].iter().collect::<String>());
        if (at_breakpoint && piece_tokens >= target / 2) || piece_tokens >= target {
            let text: String = chars[start..=i].iter().collect();
            out.push((text, base + start, base + i + 1));
            start = i + 1;
        }
        i += 1;
        let _ = piece_len;
    }
    if start < chars.len() {
        let text: String = chars[start..].iter().collect();
        out.push((text, base + start, base + chars.len()));
    }
    out
}

/// Chunk a document. `markdown` enables ATX-heading awareness (heading lines
/// start a new section and are recorded in `heading_path`).
pub fn chunk_document(text: &str, markdown: bool) -> Vec<ChunkPiece> {
    chunk_with(text, markdown, TARGET_TOKENS, OVERLAP_TOKENS, MIN_TOKENS)
}

pub fn chunk_with(
    text: &str,
    markdown: bool,
    target: usize,
    overlap: usize,
    min_tokens: usize,
) -> Vec<ChunkPiece> {
    // Work in char offsets so citation positions are stable across scripts.
    let mut pieces: Vec<ChunkPiece> = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();

    // Current accumulating buffer: list of (text, start, end, tokens).
    let mut buf: Vec<(String, usize, usize, usize)> = Vec::new();
    let mut buf_tokens = 0usize;
    let mut cur_heading: Option<String> = None;

    let flush = |buf: &mut Vec<(String, usize, usize, usize)>,
                 buf_tokens: &mut usize,
                 heading: &Option<String>,
                 pieces: &mut Vec<ChunkPiece>| {
        if buf.is_empty() {
            return;
        }
        let start = buf.first().unwrap().1;
        let end = buf.last().unwrap().2;
        let body: String = buf
            .iter()
            .map(|(t, _, _, _)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let text = match heading {
            Some(h) => format!("{h}\n{body}"),
            None => body,
        };
        pieces.push(ChunkPiece {
            text,
            char_start: start,
            char_end: end,
            heading_path: heading.clone(),
        });
        buf.clear();
        *buf_tokens = 0;
    };

    // Iterate lines, tracking char offsets (newline counts as 1 char).
    let mut offset = 0usize;
    for line in split_lines_keep(text) {
        let line_chars = line.chars().count();
        let content = line.trim_end_matches(['\r']);
        // char_end points at the end of the stored content (CRLF '\r' excluded),
        // while `offset` still advances over the full raw line + the '\n' so
        // later line offsets stay aligned with the original text.
        let line_start = offset;
        let line_end = offset + content.chars().count();
        offset += line_chars + 1;

        // Heading boundary (markdown only).
        if markdown {
            if let Some((level, title)) = parse_heading(content) {
                flush(&mut buf, &mut buf_tokens, &cur_heading, &mut pieces);
                stack.retain(|(l, _)| *l < level);
                stack.push((level, title));
                cur_heading = heading_path(&stack);
                continue;
            }
        }

        if content.trim().is_empty() {
            continue;
        }

        let line_tokens = estimate_tokens(content);

        // A single over-long line: flush, then emit sentence-window sub-pieces.
        if line_tokens > target {
            flush(&mut buf, &mut buf_tokens, &cur_heading, &mut pieces);
            for (sub, s, e) in split_long_line(content, line_start, target) {
                let t = match &cur_heading {
                    Some(h) => format!("{h}\n{sub}"),
                    None => sub.clone(),
                };
                pieces.push(ChunkPiece {
                    text: t,
                    char_start: s,
                    char_end: e,
                    heading_path: cur_heading.clone(),
                });
            }
            continue;
        }

        // Would overflow → flush, then seed overlap from the tail of the buffer.
        if buf_tokens + line_tokens > target && !buf.is_empty() {
            let carry = take_overlap(&buf, overlap);
            flush(&mut buf, &mut buf_tokens, &cur_heading, &mut pieces);
            for item in carry {
                buf_tokens += item.3;
                buf.push(item);
            }
        }

        buf_tokens += line_tokens;
        buf.push((content.to_string(), line_start, line_end, line_tokens));
    }
    flush(&mut buf, &mut buf_tokens, &cur_heading, &mut pieces);

    merge_tiny_tail(&mut pieces, min_tokens);
    pieces
}

/// Trailing buffer items summing to ~`overlap` tokens (kept as the next chunk's
/// lead-in). Order preserved.
fn take_overlap(
    buf: &[(String, usize, usize, usize)],
    overlap: usize,
) -> Vec<(String, usize, usize, usize)> {
    if overlap == 0 {
        return Vec::new();
    }
    let mut acc = 0usize;
    let mut start_idx = buf.len();
    for i in (0..buf.len()).rev() {
        if acc >= overlap {
            break;
        }
        acc += buf[i].3;
        start_idx = i;
    }
    // Don't carry the whole buffer (that would never make progress).
    if start_idx == 0 && buf.len() > 1 {
        start_idx = 1;
    }
    buf[start_idx..].to_vec()
}

/// If the last chunk is tiny and shares the previous chunk's heading, fold it
/// back in so we don't emit a near-empty fragment.
fn merge_tiny_tail(pieces: &mut Vec<ChunkPiece>, min_tokens: usize) {
    if pieces.len() < 2 {
        return;
    }
    let last = pieces.last().unwrap();
    if estimate_tokens(&last.text) >= min_tokens {
        return;
    }
    let last = pieces.pop().unwrap();
    let prev = pieces.last_mut().unwrap();
    if prev.heading_path == last.heading_path {
        prev.text.push('\n');
        prev.text.push_str(&last.text);
        prev.char_end = last.char_end;
    } else {
        pieces.push(last); // different section — keep as-is
    }
}

/// Split on '\n' without dropping a trailing empty segment difference; mirrors
/// the offset math in `chunk_with` (each '\n' is one char).
fn split_lines_keep(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_tokens_counted_higher_than_latin_rule() {
        // 10 Chinese chars → ~10 tokens, NOT 10/4≈3.
        let zh = "一二三四五六七八九十";
        assert!(estimate_tokens(zh) >= 10);
        // 40 latin chars → ~10 tokens.
        let en = "abcdefghij abcdefghij abcdefghij abcdefg";
        assert!(estimate_tokens(en) <= 14 && estimate_tokens(en) >= 8);
    }

    #[test]
    fn heading_parsing() {
        assert_eq!(parse_heading("# Title"), Some((1, "Title".to_string())));
        assert_eq!(parse_heading("### A B"), Some((3, "A B".to_string())));
        assert_eq!(parse_heading("#NoSpace"), None);
        assert_eq!(parse_heading("plain"), None);
        assert_eq!(parse_heading("####### too many"), None);
    }

    #[test]
    fn markdown_chunks_carry_heading_path() {
        let md = "# Guide\n\n## Setup\nInstall the thing.\nRun it.\n\n## Usage\nUse the thing daily.";
        let chunks = chunk_document(md, true);
        assert!(!chunks.is_empty());
        let setup = chunks.iter().find(|c| c.text.contains("Install")).unwrap();
        assert_eq!(setup.heading_path.as_deref(), Some("Guide > Setup"));
        let usage = chunks.iter().find(|c| c.text.contains("daily")).unwrap();
        assert_eq!(usage.heading_path.as_deref(), Some("Guide > Usage"));
    }

    #[test]
    fn long_chinese_text_chunks_within_budget() {
        // ~2000 Chinese chars, no separators → must be split, none oversized.
        let zh = "数据".repeat(1000); // 2000 chars
        let chunks = chunk_with(&zh, false, 480, 64, 80);
        assert!(chunks.len() > 1, "expected multiple chunks");
        for c in &chunks {
            assert!(
                estimate_tokens(&c.text) <= 480 + 64,
                "chunk too large: {} tokens",
                estimate_tokens(&c.text)
            );
        }
    }

    #[test]
    fn short_doc_is_one_chunk() {
        let chunks = chunk_document("Just a short note.", false);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].char_start, 0);
    }

    #[test]
    fn overlap_makes_progress_not_infinite() {
        // Many medium lines → multiple chunks, strictly increasing offsets.
        let text = (0..50)
            .map(|i| format!("line number {i} with some filler words here"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_with(&text, false, 60, 16, 10);
        assert!(chunks.len() > 1);
        for w in chunks.windows(2) {
            assert!(w[1].char_start >= w[0].char_start);
        }
    }
}
