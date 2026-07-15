//! 模糊匹配 —— PI `fuzzy.ts` 端口。
//!
//! 若 query 的所有字符按序（不必连续）出现于 text 即匹配。**分数越低越好。**
//! 奖励连续匹配 / 词边界 / 全等，惩罚 gap 与靠后位置。`fuzzy_filter` 按空白/斜杠切 token，
//! 全部 token 命中才保留，升序排序。

/// 模糊匹配结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

fn is_word_boundary_char(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '-' | '_' | '.' | '/' | ':')
}

fn match_query(normalized_query: &[char], text_lower: &[char]) -> FuzzyMatch {
    if normalized_query.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }
    if normalized_query.len() > text_lower.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    let mut query_index = 0usize;
    let mut score = 0.0f64;
    let mut last_match_index: i64 = -1;
    let mut consecutive_matches = 0i64;

    let mut i = 0usize;
    while i < text_lower.len() && query_index < normalized_query.len() {
        if text_lower[i] == normalized_query[query_index] {
            let is_word_boundary = i == 0 || is_word_boundary_char(text_lower[i - 1]);

            if last_match_index == i as i64 - 1 {
                consecutive_matches += 1;
                score -= consecutive_matches as f64 * 5.0;
            } else {
                consecutive_matches = 0;
                if last_match_index >= 0 {
                    score += (i as i64 - last_match_index - 1) as f64 * 2.0;
                }
            }

            if is_word_boundary {
                score -= 10.0;
            }

            score += i as f64 * 0.1;

            last_match_index = i as i64;
            query_index += 1;
        }
        i += 1;
    }

    if query_index < normalized_query.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    if normalized_query == text_lower {
        score -= 100.0;
    }

    FuzzyMatch {
        matches: true,
        score,
    }
}

/// 把 ASCII 字母段 + 数字段（或反之）互换，用于「输入顺序颠倒」的兜底匹配。
fn swapped_query(q: &str) -> Option<String> {
    let chars: Vec<char> = q.chars().collect();
    if chars.is_empty() {
        return None;
    }
    // letters+digits
    let letters_end = chars.iter().take_while(|c| c.is_ascii_lowercase()).count();
    if letters_end > 0
        && letters_end < chars.len()
        && chars[letters_end..].iter().all(|c| c.is_ascii_digit())
    {
        let letters: String = chars[..letters_end].iter().collect();
        let digits: String = chars[letters_end..].iter().collect();
        return Some(format!("{digits}{letters}"));
    }
    // digits+letters
    let digits_end = chars.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits_end > 0
        && digits_end < chars.len()
        && chars[digits_end..].iter().all(|c| c.is_ascii_lowercase())
    {
        let digits: String = chars[..digits_end].iter().collect();
        let letters: String = chars[digits_end..].iter().collect();
        return Some(format!("{letters}{digits}"));
    }
    None
}

/// 模糊匹配 `query` 与 `text`。
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower = query.to_lowercase();
    let text_lower_str = text.to_lowercase();
    let query_chars: Vec<char> = query_lower.chars().collect();
    let text_chars: Vec<char> = text_lower_str.chars().collect();

    let primary = match_query(&query_chars, &text_chars);
    if primary.matches {
        return primary;
    }

    let Some(sw) = swapped_query(&query_lower) else {
        return primary;
    };
    let sw_chars: Vec<char> = sw.chars().collect();
    let swapped = match_query(&sw_chars, &text_chars);
    if !swapped.matches {
        return primary;
    }
    FuzzyMatch {
        matches: true,
        score: swapped.score + 5.0,
    }
}

/// 按模糊匹配质量过滤并排序 `items`（最佳在前）。空白/斜杠切 token，全部命中才保留。
pub fn fuzzy_filter<T, F>(items: Vec<T>, query: &str, get_text: F) -> Vec<T>
where
    F: Fn(&T) -> String,
{
    if query.trim().is_empty() {
        return items;
    }
    let tokens: Vec<&str> = query
        .trim()
        .split(|c: char| c.is_whitespace() || c == '/')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return items;
    }

    let mut scored: Vec<(T, f64)> = Vec::new();
    for item in items {
        let text = get_text(&item);
        let mut total = 0.0f64;
        let mut all_match = true;
        for token in &tokens {
            let m = fuzzy_match(token, &text);
            if m.matches {
                total += m.score;
            } else {
                all_match = false;
                break;
            }
        }
        if all_match {
            scored.push((item, total));
        }
    }
    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(item, _)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches() {
        assert!(fuzzy_match("", "anything").matches);
    }

    #[test]
    fn subsequence_match() {
        assert!(fuzzy_match("abc", "axbxc").matches);
        assert!(!fuzzy_match("abc", "acb").matches);
    }

    #[test]
    fn query_longer_than_text_fails() {
        assert!(!fuzzy_match("abcdef", "abc").matches);
    }

    #[test]
    fn consecutive_better_than_gapped() {
        let consec = fuzzy_match("abc", "abcxyz").score;
        let gapped = fuzzy_match("abc", "axbxcx").score;
        assert!(
            consec < gapped,
            "consecutive {consec} should beat gapped {gapped}"
        );
    }

    #[test]
    fn exact_match_best() {
        let exact = fuzzy_match("hello", "hello").score;
        let prefix = fuzzy_match("hello", "helloworld").score;
        assert!(exact < prefix);
    }

    #[test]
    fn word_boundary_rewarded() {
        // 'c' after a boundary '-' should score better than mid-word
        let boundary = fuzzy_match("c", "ab-c").score;
        let midword = fuzzy_match("c", "abxc").score;
        assert!(boundary < midword);
    }

    #[test]
    fn filter_orders_best_first() {
        let items = vec![
            "model".to_string(),
            "compact".to_string(),
            "commit".to_string(),
        ];
        let out = fuzzy_filter(items, "co", |s| s.clone());
        // "compact"/"commit" both start with co; "model" doesn't contain c then o in order? m-o-d-e-l: no 'c'
        assert!(!out.contains(&"model".to_string()));
        assert!(out.contains(&"compact".to_string()));
        assert!(out.contains(&"commit".to_string()));
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let items = vec!["a".to_string(), "b".to_string()];
        let out = fuzzy_filter(items.clone(), "  ", |s| s.clone());
        assert_eq!(out, items);
    }

    #[test]
    fn filter_multi_token_all_must_match() {
        let items = vec!["foo bar".to_string(), "foo".to_string(), "bar".to_string()];
        let out = fuzzy_filter(items, "foo bar", |s| s.clone());
        assert_eq!(out, vec!["foo bar".to_string()]);
    }

    #[test]
    fn swapped_query_fallback() {
        // "1a" should fuzzy-match "a1" via swap
        let m = fuzzy_match("1a", "a1");
        assert!(m.matches);
    }
}
