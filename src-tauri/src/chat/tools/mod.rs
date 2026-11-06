pub use kivio_agent_sdk::tools::*;
use serde_json::Value;

pub(crate) fn summarize_tool_text(content: &str) -> String {
    let trimmed = content.trim();
    let json_str = trimmed
        .strip_prefix("stdout:")
        .map(str::trim)
        .unwrap_or(trimmed);
    let Ok(value) = serde_json::from_str::<Value>(json_str) else {
        return content.to_string();
    };

    if let Some(answer) = value
        .get("answer")
        .and_then(|answer| answer.as_str())
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
    {
        return format!("答: {answer}");
    }

    let Some(results) = value.get("results").and_then(|results| results.as_array()) else {
        return content.to_string();
    };

    if results.is_empty() {
        return "无搜索结果".to_string();
    }

    let query = value
        .get("query")
        .and_then(|query| query.as_str())
        .unwrap_or_default();
    let query_label = if query.is_empty() {
        String::new()
    } else {
        format!("「{query}」")
    };
    let first = &results[0];
    let title = first
        .get("title")
        .or_else(|| first.get("url"))
        .and_then(|title| title.as_str())
        .unwrap_or_default();
    let snippet = first
        .get("content")
        .or_else(|| first.get("raw_content"))
        .and_then(|content| content.as_str())
        .unwrap_or_default();
    let snippet: String = snippet.chars().take(80).collect();
    let head = format!("{} 条结果{query_label}", results.len());
    if title.is_empty() && snippet.is_empty() {
        return head;
    }
    if snippet.is_empty() {
        return format!("{head}: {title}");
    }
    format!("{head}: {title} - {snippet}")
}
