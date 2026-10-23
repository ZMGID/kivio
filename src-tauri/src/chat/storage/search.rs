use std::fs;

use tauri::AppHandle;

use crate::chat::{Conversation, ConversationListItem, ConversationSearchHit};

use super::{conversation_file_path, load_index_or_scan};

#[cfg(test)]
pub(super) use super::super::conversation_library::compare_library_items;
pub use super::super::conversation_library::{ConversationLibraryPage, ConversationLibraryQuery};

/// 对话库统一查询：在 index 上筛选/排序/分页；`q` 非空时可全文扫正文。
/// 与 `search_conversations` 一样**不拿写锁**——成本与命中正文的会话数成正比。
pub fn query_conversations(
    app: &AppHandle,
    query: ConversationLibraryQuery,
) -> Result<ConversationLibraryPage, String> {
    super::super::conversation_library::query_items(
        load_index_or_scan(app)?.conversations,
        query,
        |id, needle| conversation_content_matches(app, id, needle),
        |item, needle| match_conversation_for_search(app, item, needle),
    )
}

/// 全量索引搜索：在所有对话（不止侧栏默认加载的前 N 个）的标题/预览/文件夹里做大小写
/// 不敏感子串匹配，按更新时间倒序返回前 limit 个。元数据未命中的条目才读正文；
/// 损坏或不可读的单个会话按未命中处理，不阻断其他搜索结果。
pub fn search_conversations(
    app: &AppHandle,
    query: &str,
    limit: usize,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<ConversationSearchHit>, String> {
    // Keep empty/disabled searches side-effect free: the legacy entry point returned before
    // touching the repository, so a missing/corrupt index must not turn an empty query into an
    // error.
    if query.trim().is_empty() || limit == 0 || cancelled() {
        return Ok(vec![]);
    }
    let index = load_index_or_scan(app)?;
    Ok(search_ranked_items(
        index.conversations,
        query,
        limit,
        |item, needle| match_conversation_for_search(app, item, needle),
        cancelled,
    ))
}

fn search_ranked_items<F>(
    items: Vec<ConversationListItem>,
    query: &str,
    limit: usize,
    mut matcher: F,
    cancelled: impl Fn() -> bool,
) -> Vec<ConversationSearchHit>
where
    F: FnMut(ConversationListItem, &str) -> Option<ConversationSearchHit>,
{
    let needle = query.trim().to_lowercase();
    if needle.is_empty() || limit == 0 {
        return vec![];
    }

    let mut hits = Vec::new();
    for item in rank_conversations_for_search(items) {
        if cancelled() {
            return Vec::new();
        }
        if let Some(hit) = matcher(item, &needle) {
            hits.push(hit);
            if hits.len() >= limit {
                break;
            }
        }
    }
    hits
}

pub(super) fn rank_conversations_for_search(
    mut items: Vec<ConversationListItem>,
) -> Vec<ConversationListItem> {
    items.retain(|item| !item.archived);
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    items
}

fn match_conversation_for_search(
    app: &AppHandle,
    item: ConversationListItem,
    needle: &str,
) -> Option<ConversationSearchHit> {
    match_conversation_with_loader(item, needle, |id, needle| {
        load_conversation_for_search(app, id, needle)
    })
}

fn match_conversation_with_loader<F>(
    item: ConversationListItem,
    needle: &str,
    mut load: F,
) -> Option<ConversationSearchHit>
where
    F: FnMut(&str, &str) -> Option<Conversation>,
{
    if item.title.to_lowercase().contains(needle)
        && !crate::chat::commands::title::is_placeholder_title(&item.title)
    {
        return Some(ConversationSearchHit {
            match_field: "title".into(),
            match_message_id: None,
            match_snippet: Some(item.title.clone()),
            item,
        });
    }
    if item.preview.to_lowercase().contains(needle) {
        return Some(ConversationSearchHit {
            match_snippet: Some(make_search_snippet(&item.preview, needle)),
            match_field: "preview".into(),
            match_message_id: None,
            item,
        });
    }
    if item
        .folder
        .as_deref()
        .map(|f| f.to_lowercase().contains(needle))
        .unwrap_or(false)
    {
        return Some(ConversationSearchHit {
            match_snippet: item.folder.clone(),
            match_field: "folder".into(),
            match_message_id: None,
            item,
        });
    }
    if item
        .assistant_name
        .as_deref()
        .map(|n| n.to_lowercase().contains(needle))
        .unwrap_or(false)
    {
        return Some(ConversationSearchHit {
            match_snippet: item.assistant_name.clone(),
            match_field: "assistant".into(),
            match_message_id: None,
            item,
        });
    }
    if item.model.to_lowercase().contains(needle) {
        return Some(ConversationSearchHit {
            match_snippet: Some(item.model.clone()),
            match_field: "model".into(),
            match_message_id: None,
            item,
        });
    }

    let conv = load(&item.id, needle)?;
    first_message_match(&conv, needle).map(|(field, message_id, snippet)| ConversationSearchHit {
        item,
        match_field: field,
        match_message_id: Some(message_id),
        match_snippet: Some(snippet),
    })
}

fn conversation_content_matches(app: &AppHandle, id: &str, needle: &str) -> bool {
    load_conversation_for_search(app, id, needle).is_some_and(|conv| messages_match(&conv, needle))
}

fn load_conversation_for_search(
    app: &AppHandle,
    id: &str,
    needle_lower: &str,
) -> Option<Conversation> {
    let path = conversation_file_path(app, id).ok()?;
    load_conversation_file_for_search(&path, needle_lower)
}

/// 全文匹配：原文里连关键词都没有就跳过 serde；读/解析失败按不匹配处理。
fn load_conversation_file_for_search(
    path: &std::path::Path,
    needle_lower: &str,
) -> Option<Conversation> {
    let raw = fs::read_to_string(path).ok()?;
    if !conversation_raw_might_contain(&raw, needle_lower) {
        return None;
    }
    serde_json::from_str(&raw).ok()
}

/// 保守预筛：只有确定解码后的正文也不可能匹配时，才跳过反序列化。
pub(super) fn conversation_raw_might_contain(raw: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return false;
    }
    if raw.contains('\\')
        && (raw.contains(r"\u")
            || needle_lower.chars().any(|c| {
                matches!(c, '"' | '\\' | '/')
                    || c.is_control()
                    || (!c.is_ascii() && (c.is_lowercase() || c.is_uppercase()))
            }))
    {
        return true;
    }
    raw.contains(needle_lower) || raw.to_lowercase().contains(needle_lower)
}

pub(super) fn messages_match(conv: &Conversation, needle: &str) -> bool {
    first_message_match(conv, needle).is_some()
}

pub(super) use crate::chat::conversation_search::{first_message_match, make_search_snippet};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn search_item(id: &str, updated_at: i64, archived: bool) -> ConversationListItem {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "revision": 1,
            "title": id,
            "preview": "",
            "provider_id": "provider",
            "model": "model",
            "message_count": 0,
            "created_at": 1,
            "updated_at": updated_at,
            "archived": archived
        }))
        .unwrap()
    }

    fn conversation(id: &str, content: &str) -> Conversation {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": "conversation",
            "provider_id": "provider",
            "model": "model",
            "created_at": 1,
            "updated_at": 1,
            "messages": [{
                "id": format!("msg_{id}"),
                "role": "user",
                "content": content,
                "timestamp": 1
            }]
        }))
        .unwrap()
    }

    #[test]
    fn superseded_search_stops_before_reading_remaining_conversations() {
        let reads = std::cell::Cell::new(0);
        let hits = search_ranked_items(
            vec![
                search_item("a", 3, false),
                search_item("b", 2, false),
                search_item("c", 1, false),
            ],
            "needle",
            10,
            |_, _| {
                reads.set(reads.get() + 1);
                None
            },
            || reads.get() >= 1,
        );
        assert!(hits.is_empty());
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn search_orders_newest_first_and_applies_limit() {
        let dir = std::env::temp_dir().join(format!("kivio-search-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        for id in ["conv_old", "conv_new", "conv_middle"] {
            fs::write(
                dir.join(format!("{id}.json")),
                serde_json::to_string(&conversation(id, "shared needle")).unwrap(),
            )
            .unwrap();
        }

        let hits = search_ranked_items(
            vec![
                search_item("conv_old", 1, false),
                search_item("conv_new", 9, false),
                search_item("conv_middle", 5, false),
            ],
            "needle",
            2,
            |item, needle| {
                match_conversation_with_loader(item, needle, |id, needle| {
                    load_conversation_file_for_search(&dir.join(format!("{id}.json")), needle)
                })
            },
            || false,
        );

        assert_eq!(
            hits.iter()
                .map(|hit| hit.item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["conv_new", "conv_middle"]
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_conversation_is_a_miss_and_does_not_block_later_hits() {
        let dir = std::env::temp_dir().join(format!("kivio-search-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("conv_corrupt.json"), r#"{"content":"needle""#).unwrap();
        fs::write(
            dir.join("conv_good.json"),
            serde_json::to_string(&conversation("conv_good", "needle survives")).unwrap(),
        )
        .unwrap();

        let hits = search_ranked_items(
            vec![
                search_item("conv_corrupt", 10, false),
                search_item("conv_good", 1, false),
            ],
            "needle",
            10,
            |item, needle| {
                match_conversation_with_loader(item, needle, |id, needle| {
                    load_conversation_file_for_search(&dir.join(format!("{id}.json")), needle)
                })
            },
            || false,
        );

        assert_eq!(
            hits.iter()
                .map(|hit| hit.item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["conv_good"]
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn make_search_snippet_centers_on_match_and_stays_char_safe() {
        let text = format!("{}关键命中词在这里{}", "前".repeat(80), "后".repeat(80));
        let snippet = make_search_snippet(&text, "命中词");
        assert!(snippet.starts_with('…'), "snippet={snippet}");
        assert!(snippet.ends_with('…'), "snippet={snippet}");
        assert!(snippet.contains("命中词"), "snippet={snippet}");
        assert!(snippet.is_char_boundary(0));
        assert!(snippet.is_char_boundary(snippet.len()));
    }

    #[test]
    fn make_search_snippet_collapses_whitespace_and_truncates_misses() {
        let hit = make_search_snippet("line1\n\n  line2\t\tline3", "line2");
        assert!(!hit.contains('\n'), "snippet={hit}");
        assert!(!hit.contains('\t'), "snippet={hit}");
        assert!(hit.contains("line2"), "snippet={hit}");

        let miss = make_search_snippet(&"甲".repeat(200), "不存在");
        assert!(miss.chars().count() <= 143);
        assert!(miss.ends_with("..."), "miss={miss}");
    }

    #[test]
    fn first_message_match_returns_field_message_id_and_snippet() {
        let conv: Conversation = serde_json::from_value(serde_json::json!({
            "id": "conv_s", "title": "t", "provider_id": "p", "model": "m",
            "created_at": 1, "updated_at": 1,
            "messages": [{
                "id": "m2", "role": "assistant",
                "content": "出现关键词知识库配置。",
                "reasoning": "需要检查 WASM 加载", "timestamp": 2
            }]
        }))
        .unwrap();

        let (field, message_id, snippet) = first_message_match(&conv, "知识库").unwrap();
        assert_eq!((field.as_str(), message_id.as_str()), ("content", "m2"));
        assert!(snippet.contains("知识库"));
        let (field, message_id, _) = first_message_match(&conv, "wasm 加载").unwrap();
        assert_eq!((field.as_str(), message_id.as_str()), ("reasoning", "m2"));
    }

    #[test]
    fn search_raw_prefilter_skips_serde_when_needle_absent() {
        let raw = r#"{"id":"conv_a","messages":[{"content":"你好世界"}]}"#;
        assert!(conversation_raw_might_contain(raw, "你好"));
        assert!(!conversation_raw_might_contain(raw, "不存在的词"));
        assert!(conversation_raw_might_contain(
            r#"{"content":"HELLO"}"#,
            "hello"
        ));
        assert!(!conversation_raw_might_contain(raw, ""));
    }

    fn assert_search_prefilter_preserves_match(encoded_text: &str, needle: &str) {
        for field in ["content", "reasoning"] {
            let mut value = serde_json::json!({
                "id": "conv_search", "title": "Search regression",
                "provider_id": "provider", "model": "model",
                "created_at": 1, "updated_at": 1,
                "messages": [{"id": "msg_search", "role": "assistant", "content": "", "timestamp": 1}]
            });
            value["messages"][0][field] = serde_json::json!("__search_text__");
            let raw = value
                .to_string()
                .replace(r#""__search_text__""#, encoded_text);
            let parsed: Conversation = serde_json::from_str(&raw).unwrap();
            let needle_lower = needle.to_lowercase();
            assert!(messages_match(&parsed, &needle_lower));
            assert!(conversation_raw_might_contain(&raw, &needle_lower));
        }
    }

    #[test]
    fn search_raw_prefilter_preserves_json_escaped_and_unicode_matches() {
        for (encoded, needle) in [
            (r#""C:\\Users\\alice""#, r"c:\users\alice"),
            (r#""say \"hello\"""#, "\"hello\""),
            (r#""foo\/bar""#, "foo/bar"),
            (r#""\u4f60\u597d""#, "你好"),
            (r#""ПРИВЕТ""#, "привет"),
            (r#""\u00c9""#, "é"),
        ] {
            assert_search_prefilter_preserves_match(encoded, needle);
        }
    }

    #[test]
    fn file_loader_rejects_missing_and_corrupt_files() {
        assert!(load_conversation_file_for_search(Path::new("missing"), "needle").is_none());
    }
}
