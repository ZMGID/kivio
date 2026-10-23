use super::*;
pub(super) use crate::chat::conversation_files::{
    conversation_file_ids_in_dir, index_covers_files, load_index_in_dir, load_index_or_scan_in_dir,
};

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

/// 加载对话索引
pub fn load_index(app: &AppHandle) -> Result<ConversationIndex, String> {
    load_index_in_dir(&conversations_dir(app)?)
}

/// 保存对话索引
pub(crate) fn save_index(app: &AppHandle, index: &ConversationIndex) -> Result<(), String> {
    let path = index_file_path(app)?;
    crate::chat::conversation_files::save_index_in_dir(&conversations_dir(app)?, index)?;
    if let Some(dir) = path.parent() {
        remember_conversation_index_cache(dir.to_path_buf(), index.clone(), false);
    }
    Ok(())
}
