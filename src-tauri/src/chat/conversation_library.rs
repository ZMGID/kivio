//! Host-independent conversation library query, shared by Tauri and GPUI.
use super::conversation_data::{ConversationListItem, ConversationSearchHit};
use serde::Serialize;
use std::cmp::Ordering;

/// 对话库查询参数（扩展 → 对话库）。
#[derive(Debug, Clone, Default)]
pub struct ConversationLibraryQuery {
    pub offset: usize,
    pub limit: usize,
    /// updated | created | title | messages
    pub sort: String,
    /// asc | desc（默认 desc）
    pub order: String,
    pub q: Option<String>,
    /// 有 q 时是否扫正文（默认 true）
    pub full_text: bool,
    /// all | starred | uncategorized | recent7d | archived
    pub shelf: String,
    pub project_id: Option<String>,
    pub set_id: Option<String>,
    pub assistant_id: Option<String>,
    pub provider_id: Option<String>,
    /// builtin | external | 空
    pub runtime_kind: Option<String>,
}

/// 对话库分页结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationLibraryPage {
    /// 有搜索词时条目带 match_* 高亮/跳转字段；无搜索词时 match 字段为空。
    pub items: Vec<ConversationSearchHit>,
    pub total: usize,
}

pub fn query_items(
    mut items: Vec<ConversationListItem>,
    query: ConversationLibraryQuery,
    mut content_matches: impl FnMut(&str, &str) -> bool,
    mut match_item: impl FnMut(ConversationListItem, &str) -> Option<ConversationSearchHit>,
) -> Result<ConversationLibraryPage, String> {
    let limit = query.limit.clamp(1, 200);
    let offset = query.offset;

    let shelf = query.shelf.trim().to_ascii_lowercase();
    // Conversation timestamps are unix seconds (chrono::Local::now().timestamp()).
    let now_sec = chrono::Local::now().timestamp();
    let week_ago = now_sec.saturating_sub(7 * 24 * 60 * 60);

    items.retain(|c| match shelf.as_str() {
        "starred" => c.pinned && !c.archived,
        "uncategorized" => {
            !c.archived
                && c.set_id.as_deref().map(str::is_empty).unwrap_or(true)
                && c.project_id.as_deref().map(str::is_empty).unwrap_or(true)
                && c.folder.as_deref().map(str::is_empty).unwrap_or(true)
        }
        "recent7d" => !c.archived && c.updated_at >= week_ago,
        "archived" => c.archived,
        // all（默认）：未归档
        _ => !c.archived,
    });

    if let Some(set_id) = query
        .set_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        items.retain(|c| c.set_id.as_deref() == Some(set_id));
    } else if let Some(project_id) = query
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        items.retain(|c| c.project_id.as_deref() == Some(project_id));
    }

    if let Some(assistant_id) = query
        .assistant_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        items.retain(|c| c.assistant_id.as_deref() == Some(assistant_id));
    }
    if let Some(provider_id) = query
        .provider_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        items.retain(|c| c.provider_id == provider_id);
    }
    if let Some(kind) = query
        .runtime_kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let want_external = kind.eq_ignore_ascii_case("external");
        items.retain(|c| c.agent_runtime.is_external() == want_external);
    }

    let needle = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase());

    if let Some(needle) = needle.as_deref() {
        let full_text = query.full_text;
        items.retain(|c| {
            let meta_hit = c.title.to_lowercase().contains(needle)
                || c.preview.to_lowercase().contains(needle)
                || c.folder
                    .as_deref()
                    .map(|f| f.to_lowercase().contains(needle))
                    .unwrap_or(false)
                || c.assistant_name
                    .as_deref()
                    .map(|n| n.to_lowercase().contains(needle))
                    .unwrap_or(false)
                || c.model.to_lowercase().contains(needle);
            if meta_hit {
                return true;
            }
            full_text && content_matches(&c.id, needle)
        });
    }

    let sort = query.sort.trim().to_ascii_lowercase();
    let asc = query.order.trim().eq_ignore_ascii_case("asc");
    items.sort_by(|a, b| compare_library_items(a, b, &sort, asc));

    let total = items.len();
    if offset >= total {
        return Ok(ConversationLibraryPage {
            items: vec![],
            total,
        });
    }
    let end = (offset + limit).min(total);
    // 只给当前页补匹配片段（最多 limit 条），避免对全量命中重复读盘。
    let page = items[offset..end]
        .iter()
        .map(|item| {
            if let Some(needle) = needle.as_deref() {
                match_item(item.clone(), needle).unwrap_or_else(|| ConversationSearchHit {
                    item: item.clone(),
                    match_field: "meta".into(),
                    match_message_id: None,
                    match_snippet: None,
                })
            } else {
                ConversationSearchHit {
                    item: item.clone(),
                    match_field: String::new(),
                    match_message_id: None,
                    match_snippet: None,
                }
            }
        })
        .collect();
    Ok(ConversationLibraryPage { items: page, total })
}

/// 对话库排序键：收藏置顶，同组内按 sort/order。纯函数，便于单测锁契约。
pub(crate) fn compare_library_items(
    a: &ConversationListItem,
    b: &ConversationListItem,
    sort: &str,
    asc: bool,
) -> Ordering {
    let pin_ord = b.pinned.cmp(&a.pinned);
    if pin_ord != Ordering::Equal {
        return pin_ord;
    }
    let primary = match sort {
        "created" => a.created_at.cmp(&b.created_at),
        "title" => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        "messages" => a.message_count.cmp(&b.message_count),
        _ => a.updated_at.cmp(&b.updated_at),
    };
    if asc { primary } else { primary.reverse() }
}
