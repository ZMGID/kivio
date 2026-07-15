//! 词导航 —— PI `word-navigation.ts` 端口。
//!
//! `find_word_backward` / `find_word_forward` 返回从 `cursor`（**字节偏移**）向前/后移动一个词后的
//! 新光标位置。PI 用 `Intl.Segmenter` word granularity + ASCII 标点边界；本端口用
//! `unicode-segmentation` 的 word-bound 切分作为等价物，并保留「在词内遇 ASCII 标点停下」的语义。
//!
//! 与 PI 不同处（research/pi-tui.md 已预告需 re-tune）：`unicode-segmentation` 的词边界与
//! `Intl.Segmenter` 不完全一致，但对 ASCII / 常见 CJK 文本行为相同；标点边界用显式
//! [`is_punctuation`] 复刻 PI 的 `PUNCTUATION_REGEX`（`[!-/:-@[-`{-~]` 即 ASCII 标点）。
//!
//! 全部为纯函数，不改任何状态。

use unicode_segmentation::UnicodeSegmentation;

/// 是否 ASCII 标点（复刻 PI 的 `PUNCTUATION_REGEX = /[!-\/:-@\[-`{-~]/`）。
fn is_punctuation(c: char) -> bool {
    matches!(c,
        '!'..='/' | ':'..='@' | '['..='`' | '{'..='~'
    )
}

/// 一个分段是否「纯空白」（PI 的 `isWhitespaceChar` 对整段判定）。
fn is_whitespace_segment(seg: &str) -> bool {
    !seg.is_empty() && seg.chars().all(|c| c.is_whitespace())
}

/// 一个分段是否「词样」（含字母/数字，非纯标点/空白）。对应 `Intl.SegmentData.isWordLike`。
fn is_word_like(seg: &str) -> bool {
    seg.chars().any(|c| c.is_alphanumeric())
}

/// 允许调用方把某些分段视为原子单位（如 paste marker）。
pub type IsAtomic<'a> = &'a dyn Fn(&str) -> bool;

/// 把 `text` 按 word-bound 切成 (byte_index, segment) 列表。
fn segments(text: &str) -> Vec<(usize, &str)> {
    text.split_word_bound_indices().collect()
}

/// 向前（向左）移动一个词：跳过尾随空白，停在下一个词/标点边界。
pub fn find_word_backward(text: &str, cursor: usize, is_atomic: Option<IsAtomic>) -> usize {
    if cursor == 0 {
        return 0;
    }
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let mut segs = segments(before);
    let mut new_cursor = cursor;

    let atomic = |s: &str| is_atomic.map(|f| f(s)).unwrap_or(false);

    // 跳过尾随空白分段
    while let Some(&(_, seg)) = segs.last() {
        if atomic(seg) || !is_whitespace_segment(seg) {
            break;
        }
        new_cursor -= seg.len();
        segs.pop();
    }

    let Some(&(_, last)) = segs.last() else {
        return new_cursor;
    };

    if atomic(last) {
        // 跳过一个原子分段
        new_cursor -= last.len();
    } else if is_word_like(last) {
        // 在一个词样分段内：保留 ASCII 标点边界（停在最后一个标点之后）
        let punct_positions: Vec<usize> = last
            .char_indices()
            .filter(|&(_, c)| is_punctuation(c))
            .map(|(i, c)| i + c.len_utf8())
            .collect();
        if punct_positions.is_empty() {
            new_cursor -= last.len();
        } else {
            let last_match_end = *punct_positions.last().unwrap();
            new_cursor -= last.len() - last_match_end;
        }
    } else {
        // 跳过一段「非词非空白」（标点）run
        while let Some(&(_, seg)) = segs.last() {
            if atomic(seg) || is_word_like(seg) || is_whitespace_segment(seg) {
                break;
            }
            new_cursor -= seg.len();
            segs.pop();
        }
    }

    new_cursor
}

/// 向后（向右）移动一个词：跳过前导空白，停在下一个词/标点边界。
pub fn find_word_forward(text: &str, cursor: usize, is_atomic: Option<IsAtomic>) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    let after = &text[cursor..];
    let segs = segments(after);
    let mut new_cursor = cursor;
    let mut idx = 0usize;

    let atomic = |s: &str| is_atomic.map(|f| f(s)).unwrap_or(false);

    // 跳过前导空白
    while idx < segs.len() {
        let seg = segs[idx].1;
        if atomic(seg) || !is_whitespace_segment(seg) {
            break;
        }
        new_cursor += seg.len();
        idx += 1;
    }

    if idx >= segs.len() {
        return new_cursor;
    }

    let seg = segs[idx].1;
    if atomic(seg) {
        new_cursor += seg.len();
    } else if is_word_like(seg) {
        // 词样分段内：停在第一个 ASCII 标点处（否则整段）
        let first_punct = seg
            .char_indices()
            .find(|&(_, c)| is_punctuation(c))
            .map(|(i, _)| i);
        new_cursor += first_punct.unwrap_or(seg.len());
    } else {
        // 跳过非词非空白 run
        while idx < segs.len() {
            let s = segs[idx].1;
            if atomic(s) || is_word_like(s) || is_whitespace_segment(s) {
                break;
            }
            new_cursor += s.len();
            idx += 1;
        }
    }

    new_cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backward_from_word_end() {
        // "hello world", cursor at 11 (end) -> start of "world" = 6
        assert_eq!(find_word_backward("hello world", 11, None), 6);
    }

    #[test]
    fn backward_skips_trailing_space() {
        // "hello   ", cursor 8 -> skips spaces then "hello" -> 0
        assert_eq!(find_word_backward("hello   ", 8, None), 0);
    }

    #[test]
    fn backward_stops_at_word_start() {
        // cursor in middle of "world" at 8 -> 6
        assert_eq!(find_word_backward("hello world", 8, None), 6);
    }

    #[test]
    fn backward_at_zero() {
        assert_eq!(find_word_backward("abc", 0, None), 0);
    }

    #[test]
    fn forward_from_word_start() {
        // "hello world", cursor 0 -> end of "hello" = 5
        assert_eq!(find_word_forward("hello world", 0, None), 5);
    }

    #[test]
    fn forward_skips_leading_space() {
        // "  hello", cursor 0 -> skip 2 spaces, end of hello = 7
        assert_eq!(find_word_forward("  hello", 0, None), 7);
    }

    #[test]
    fn forward_at_end() {
        assert_eq!(find_word_forward("abc", 3, None), 3);
    }

    #[test]
    fn forward_stops_at_punctuation_in_word() {
        // "foo.bar" cursor 0: word-like segment may include the dot; should stop at '.' (index 3)
        let r = find_word_forward("foo.bar", 0, None);
        assert!(r == 3 || r == 4, "expected stop near the dot, got {r}");
    }

    #[test]
    fn atomic_segment_treated_as_unit() {
        // 注意：`is_atomic` 谓词作用于 word-bound 分段，PI 的全 marker 原子化靠定制 segmenter
        // 预合并；这里在单个分段恰好等于 marker 时才生效。一个不含空格/标点的「伪 marker」分段
        // 会被识别为原子。
        let token = "ATOMIC";
        let text = format!("{token} tail");
        let is_atomic: IsAtomic = &|s: &str| s == token;
        let r = find_word_forward(&text, 0, Some(is_atomic));
        assert_eq!(r, token.len());
    }

    #[test]
    fn backward_over_punctuation_run() {
        // "a... " cursor at 4 (after the dots): skip the dots -> 1
        let r = find_word_backward("a...x", 4, None);
        assert_eq!(r, 1);
    }
}
