//! Token boundaries shared by native skill activation and external CLI dispatch.
use std::ops::Range;

fn boundary(c: char) -> bool {
    c.is_whitespace()
        || matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'
            | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}'
            | '，' | '。' | '！' | '？' | '；' | '：' | '（' | '）' | '【' | '】')
}

pub(crate) fn command_ranges(content: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let bytes = content.as_bytes();
    let mut fence = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let count = i - start;
            if fence == 0 {
                fence = count;
            } else if fence == count {
                fence = 0;
            }
            continue;
        }
        let c = content[i..].chars().next().unwrap();
        let previous = content[..i].chars().next_back();
        if fence != 0 || c != '/' || previous.is_some_and(|c| !boundary(c)) {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b':' | b'-'))
        {
            i += 1;
        }
        if i == start + 1
            || bytes
                .get(i)
                .is_some_and(|b| matches!(b, b'/' | b'.' | b'\\'))
        {
            continue;
        }
        if content[..start]
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .trim_start()
            .starts_with('>')
        {
            continue;
        }
        ranges.push(start..i);
    }
    ranges
}

pub(crate) fn goal_objective(content: &str) -> Option<String> {
    let range = command_ranges(content)
        .into_iter()
        .find(|range| content[range.clone()].eq_ignore_ascii_case("/goal"))?;
    let objective = format!("{}{}", &content[..range.start], &content[range.end..]);
    let objective = objective.trim();
    (!objective.is_empty()).then(|| objective.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_commands_preserve_utf8_offsets_and_ignore_literals() {
        let text = "请用/review 检查 /plugin:foo\n/plan args";
        assert_eq!(
            command_ranges(text)
                .iter()
                .map(|r| &text[r.clone()])
                .collect::<Vec<_>>(),
            ["/review", "/plugin:foo", "/plan"]
        );
        for text in [
            "https://host/review",
            "C:/review",
            "/review/file",
            "`/review`",
            "```\n/review\n```",
            "> /review",
        ] {
            assert!(command_ranges(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn goal_can_be_inserted_in_the_middle_without_losing_surrounding_task() {
        assert_eq!(
            goal_objective("请完成/goal 这个任务").as_deref(),
            Some("请完成 这个任务")
        );
        assert_eq!(
            goal_objective("/goal existing task").as_deref(),
            Some("existing task")
        );
        assert!(goal_objective("/goal").is_none());
        assert!(goal_objective("`/goal` examples").is_none());
    }
}
