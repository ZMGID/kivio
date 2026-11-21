use super::conversations::{conversation_file_revision, read_conversation_file};
use super::*;

struct ConversationIndexCacheEntry {
    index: ConversationIndex,
    needs_persist: bool,
}

fn conversation_index_cache() -> &'static Mutex<HashMap<PathBuf, ConversationIndexCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, ConversationIndexCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn forget_index_cache(dir: &Path) {
    conversation_index_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(dir);
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

pub(crate) fn load_index_or_scan(app: &AppHandle) -> Result<ConversationIndex, String> {
    let dir = conversations_dir(app)?;
    let mut cache = conversation_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(entry) = cache.get(&dir) {
        return Ok(entry.index.clone());
    }
    let (index, needs_persist) = load_index_or_scan_in_dir(&dir)?;
    cache.insert(
        dir,
        ConversationIndexCacheEntry {
            index: index.clone(),
            needs_persist,
        },
    );
    Ok(index)
}

/// Reconcile only when the in-process index is cold. Existing rows need a
/// header check because a crash may have committed a body without its index.
/// Repository writers own index publication; this read cannot overwrite them.
pub(super) fn load_index_or_scan_in_dir(dir: &Path) -> Result<(ConversationIndex, bool), String> {
    let file_ids = conversation_file_ids_in_dir(dir)?;
    let (mut index, mut healed) = match load_index_in_dir(dir) {
        Ok(index) => (index, false),
        Err(e) => {
            eprintln!("conversation index unavailable, rebuilding list from files: {e}");
            (ConversationIndex::default(), true)
        }
    };
    let positions: HashMap<_, _> = index
        .conversations
        .iter()
        .enumerate()
        .map(|(position, item)| (item.id.clone(), position))
        .collect();
    for id in file_ids {
        let path = dir.join(format!("{id}.json"));
        let position = positions.get(&id).copied();
        if let Some(position) = position {
            match conversation_file_revision(&path) {
                Ok(Some(revision)) if index.conversations[position].revision != Some(revision) => {}
                _ => continue,
            }
        }
        healed = true;
        match read_conversation_file(&path, &id) {
            Ok(conversation) => {
                let item = ConversationListItem::from(&conversation);
                if let Some(position) = position {
                    index.conversations[position] = item;
                } else {
                    index.conversations.push(item);
                }
            }
            Err(error) => eprintln!("skip corrupt conversation file {id}: {error}"),
        }
    }
    Ok((index, healed))
}

/// 纯逻辑:扫描给定目录,收集有效对话 id(只看文件名,不读内容)。
/// `validate_conversation_id` 要求 `conv_` 前缀 → 天然排除 index/projects/assistants.json。
pub(crate) fn conversation_file_ids_in_dir(dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read conversations dir: {e}"))? {
        let entry = entry.map_err(|e| format!("read conversation directory entry: {e}"))?;
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
    let result = atomic_write(&path, &content, "index");
    if let Some(dir) = path.parent() {
        // The body is already durable. Keep its newest row for this process
        // and leave the index marked for retry if this write failed.
        remember_conversation_index_cache(dir.to_path_buf(), index.clone(), result.is_err());
    }
    result
}
