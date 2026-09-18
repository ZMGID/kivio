use super::conversations::read_conversation_file;
use super::*;

struct ConversationIndexCacheEntry {
    index: ConversationIndex,
    needs_persist: bool,
}

fn conversation_index_cache() -> &'static Mutex<HashMap<PathBuf, ConversationIndexCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, ConversationIndexCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember_conversation_index_cache(dir: PathBuf, index: ConversationIndex, needs_persist: bool) {
    conversation_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            dir,
            ConversationIndexCacheEntry {
                index,
                needs_persist,
            },
        );
}

pub(crate) fn conversation_index_needs_persist(app: &AppHandle) -> bool {
    let Ok(dir) = conversations_dir(app) else {
        return false;
    };
    conversation_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&dir)
        .is_some_and(|entry| entry.needs_persist)
}

/// 把内存里补齐的索引写回 `index.json`。调用方必须持有 `index_lock`。
pub(crate) fn persist_healed_conversation_index(app: &AppHandle) -> Result<(), String> {
    if !conversation_index_needs_persist(app) {
        return Ok(());
    }
    let index = load_index_or_scan(app)?;
    save_index(app, &index)
}

/// 索引缺的会话才读正文；已在 index 里的条目原样保留，绝不整目录重扫。
fn merge_missing_conversations(
    dir: &Path,
    mut index: ConversationIndex,
    file_ids: &[String],
) -> ConversationIndex {
    let indexed: HashSet<String> = index
        .conversations
        .iter()
        .map(|item| item.id.clone())
        .collect();
    for id in file_ids {
        if indexed.contains(id) {
            continue;
        }
        let path = dir.join(format!("{id}.json"));
        match read_conversation_file(&path, id) {
            Ok(conversation) => index
                .conversations
                .push(ConversationListItem::from(&conversation)),
            Err(e) => eprintln!("skip corrupt conversation file {id}: {e}"),
        }
    }
    index
}

pub(crate) fn load_index_or_scan(app: &AppHandle) -> Result<ConversationIndex, String> {
    let dir = conversations_dir(app)?;
    let file_ids = conversation_file_ids_in_dir(&dir).unwrap_or_default();
    let mut cache = conversation_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(entry) = cache.get(&dir) {
        if index_covers_files(&entry.index, &file_ids) {
            return Ok(entry.index.clone());
        }
    }
    let (index, healed) = load_index_or_scan_in_dir(&dir)?;
    let needs_persist = healed || cache.get(&dir).is_some_and(|entry| entry.needs_persist);
    cache.insert(
        dir,
        ConversationIndexCacheEntry {
            index: index.clone(),
            needs_persist,
        },
    );
    Ok(index)
}

/// index.json 只是缓存；conv_<id>.json 才是真相源。
///
/// 对账口径必须**廉价**：一次 readdir 只比文件名，不读也不反序列化任何对话正文。
/// 索引缺文件时只读缺的那几份，补进现有条目——不要把已索引的会话整本再 parse 一遍。
/// 多余的幽灵条目无害，按 updated_at 排序时会被过滤掉。
///
/// **只读不写**：自愈落盘统一交给持有 `index_lock` 的写路径（`persist_healed_conversation_index`
/// / `repository::persist_locked` / `bulk_mutate_loaded` / `delete_conversation`）。这里顺手
/// `save_index` 会绕开那把锁，和并发的持久化 lost update——刚存的会话会在侧栏短暂消失。
pub(super) fn load_index_or_scan_in_dir(dir: &Path) -> Result<(ConversationIndex, bool), String> {
    let file_ids = conversation_file_ids_in_dir(dir).unwrap_or_default();
    match load_index_in_dir(dir) {
        Ok(index) if index_covers_files(&index, &file_ids) => Ok((index, false)),
        Ok(index) => Ok((merge_missing_conversations(dir, index, &file_ids), true)),
        Err(e) => {
            eprintln!("conversation index unavailable, rebuilding list from files: {e}");
            Ok((
                merge_missing_conversations(dir, ConversationIndex::default(), &file_ids),
                true,
            ))
        }
    }
}

/// 索引是否覆盖了磁盘上每个对话文件。只比 id，不比内容/revision。
pub(super) fn index_covers_files(index: &ConversationIndex, file_ids: &[String]) -> bool {
    let indexed: std::collections::HashSet<&str> =
        index.conversations.iter().map(|c| c.id.as_str()).collect();
    file_ids.iter().all(|id| indexed.contains(id.as_str()))
}

/// 纯逻辑:扫描给定目录,收集有效对话 id(只看文件名,不读内容)。
/// `validate_conversation_id` 要求 `conv_` 前缀 → 天然排除 index/projects/assistants.json。
pub(super) fn conversation_file_ids_in_dir(dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read conversations dir: {e}"))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if validate_conversation_id(stem).is_ok() {
                ids.push(stem.to_string());
            }
        }
    }
    Ok(ids)
}

/// 加载对话索引
pub fn load_index(app: &AppHandle) -> Result<ConversationIndex, String> {
    load_index_in_dir(&conversations_dir(app)?)
}

pub(super) fn load_index_in_dir(dir: &Path) -> Result<ConversationIndex, String> {
    let path = dir.join("index.json");
    if !path.exists() {
        return Ok(ConversationIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read index file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse index file: {e}"))
}

/// 保存对话索引
pub(crate) fn save_index(app: &AppHandle, index: &ConversationIndex) -> Result<(), String> {
    let path = index_file_path(app)?;
    let content = serde_json::to_string(index).map_err(|e| format!("serialize index: {e}"))?;
    atomic_write(&path, &content, "index")?;
    if let Some(dir) = path.parent() {
        remember_conversation_index_cache(dir.to_path_buf(), index.clone(), false);
    }
    Ok(())
}
