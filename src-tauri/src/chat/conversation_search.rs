//! Full-history matching shared by desktop hosts. Never searches hidden model payloads.
use super::conversation_data::{ChatMessage, Conversation};

pub fn first_message_match(conv: &Conversation, needle: &str) -> Option<(String, String, String)> {
    first_messages_match(&conv.messages, needle)
}

pub fn first_messages_match(
    messages: &[ChatMessage],
    needle: &str,
) -> Option<(String, String, String)> {
    for message in messages {
        if message.content.to_lowercase().contains(needle) {
            return Some((
                "content".into(),
                message.id.clone(),
                make_search_snippet(&message.content, needle),
            ));
        }
        if let Some(reasoning) = message
            .reasoning
            .as_deref()
            .filter(|text| text.to_lowercase().contains(needle))
        {
            return Some((
                "reasoning".into(),
                message.id.clone(),
                make_search_snippet(reasoning, needle),
            ));
        }
    }
    None
}

/// 围绕关键词截取一段可读上下文；字符边界安全，折叠换行。
pub fn make_search_snippet(text: &str, needle_lower: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let lower = flat.to_lowercase();
    let Some(pos) = lower.find(needle_lower) else {
        return super::conversation_data::truncate_chars(&flat, 140);
    };
    // Case folding can expand a character (e.g. İ -> i + combining dot).
    // Translate the match offsets back to the original text before slicing it.
    let mut folded_offset = 0;
    let mut match_start = 0;
    let mut match_end = 0;
    for (offset, character) in flat.char_indices() {
        let next = folded_offset + character.to_lowercase().map(char::len_utf8).sum::<usize>();
        if folded_offset <= pos && pos < next {
            match_start = offset;
        }
        if folded_offset < pos + needle_lower.len() && pos + needle_lower.len() <= next {
            match_end = offset + character.len_utf8();
            break;
        }
        folded_offset = next;
    }
    let start = flat.floor_char_boundary(match_start.saturating_sub(48));
    let end = flat.ceil_char_boundary((match_end + 72).min(flat.len()));
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&flat[start..end]);
    if end < flat.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn snippet_keeps_match_after_case_expanding_unicode_prefix() {
        let text = format!("{} TERMINAL final context", "İ".repeat(200));
        let snippet = super::make_search_snippet(&text, "terminal");
        assert!(
            snippet.contains("TERMINAL"),
            "snippet must include its match: {snippet}"
        );
    }
}
