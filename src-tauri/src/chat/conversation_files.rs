//! Shared durable conversation file and index rules. Blocking I/O: invoke on a worker.

use super::{
    conversation_data::{Conversation, ConversationIndex, ConversationListItem},
    file_io::atomic_write,
};
use std::{
    collections::HashSet,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRepositoryError {
    Conflict {
        id: String,
        expected: u64,
        actual: u64,
    },
    Storage(String),
}

impl fmt::Display for ConversationRepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict {
                id,
                expected,
                actual,
            } => write!(
                f,
                "conversation revision conflict ({id}): expected {expected}, actual {actual}"
            ),
            Self::Storage(message) => f.write_str(message),
        }
    }
}

impl From<String> for ConversationRepositoryError {
    fn from(value: String) -> Self {
        Self::Storage(value)
    }
}

pub type RepositoryResult<T> = Result<T, ConversationRepositoryError>;

pub(crate) fn validate_expected_revision(
    id: &str,
    actual: u64,
    expected: Option<u64>,
) -> RepositoryResult<()> {
    if let Some(expected) = expected.filter(|expected| actual != *expected) {
        return Err(ConversationRepositoryError::Conflict {
            id: id.to_string(),
            expected,
            actual,
        });
    }
    Ok(())
}

pub(crate) fn increment_revision(conversation: &mut Conversation) -> RepositoryResult<()> {
    conversation.revision = conversation.revision.checked_add(1).ok_or_else(|| {
        ConversationRepositoryError::Storage("conversation revision overflow".into())
    })?;
    Ok(())
}

pub(crate) fn validate_conversation_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("conv_")
        && id.len() > "conv_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid conversation id: {id}"))
    }
}

pub(crate) fn read_conversation_file(path: &Path, id: &str) -> Result<Conversation, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("读取对话文件失败（{id}）：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("对话文件已损坏，无法加载（{id}）：{e}"))
}

pub(crate) fn write_conversation_file_at_path(
    path: &Path,
    conversation: Conversation,
) -> Result<Conversation, String> {
    // compact 而非 pretty：长对话数 MB 级,pretty 徒增 ~30-50% 体积与序列化时间,
    // 且每个工具轮都要整本重写。人读导出走 export.rs,不靠这份文件的排版。
    let content =
        serde_json::to_string(&conversation).map_err(|e| format!("serialize conversation: {e}"))?;
    atomic_write(path, &content, "conversation")?;
    Ok(conversation)
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

/// index.json 只是缓存；conv_<id>.json 才是真相源。
///
/// 对账口径必须**廉价**：一次 readdir 只比文件名，不读也不反序列化任何对话正文。
/// 索引缺文件时只读缺的那几份，补进现有条目——不要把已索引的会话整本再 parse 一遍。
/// 多余的幽灵条目无害，按 updated_at 排序时会被过滤掉。
///
/// **只读不写**：自愈落盘统一交给持有 `index_lock` 的写路径（`persist_healed_conversation_index`
/// / `repository::persist_locked` / `bulk_mutate_loaded` / `delete_conversation`）。这里顺手
/// `save_index` 会绕开那把锁，和并发的持久化 lost update——刚存的会话会在侧栏短暂消失。
pub(crate) fn load_index_or_scan_in_dir(dir: &Path) -> Result<(ConversationIndex, bool), String> {
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
pub(crate) fn index_covers_files(index: &ConversationIndex, file_ids: &[String]) -> bool {
    let indexed: std::collections::HashSet<&str> =
        index.conversations.iter().map(|c| c.id.as_str()).collect();
    file_ids.iter().all(|id| indexed.contains(id.as_str()))
}

/// 纯逻辑:扫描给定目录,收集有效对话 id(只看文件名,不读内容)。
/// `validate_conversation_id` 要求 `conv_` 前缀 → 天然排除 index/projects/assistants.json。
pub(crate) fn conversation_file_ids_in_dir(dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read conversations dir: {e}"))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|stem| validate_conversation_id(stem).is_ok())
        {
            ids.push(stem.to_string());
        }
    }
    Ok(ids)
}

pub(crate) fn load_index_in_dir(dir: &Path) -> Result<ConversationIndex, String> {
    let path = dir.join("index.json");
    if !path.exists() {
        return Ok(ConversationIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read index file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse index file: {e}"))
}

pub(crate) fn save_index_in_dir(dir: &Path, index: &ConversationIndex) -> Result<(), String> {
    let content = serde_json::to_string(index).map_err(|e| format!("serialize index: {e}"))?;
    atomic_write(&dir.join("index.json"), &content, "index")
}

/// Standalone host owner over the same files, canonical types and CAS rules used
/// by ConversationRepository. Clones and owners opened for the same directory
/// share a write lock. The host must use an isolated data identity while the old
/// Tauri application is active; this lock deliberately is not a cross-process lease.
#[derive(Clone)]
pub struct ConversationStore {
    directory: PathBuf,
    mutation: Arc<Mutex<()>>,
}

impl ConversationStore {
    pub fn new(directory: PathBuf) -> Self {
        type Locks = std::collections::HashMap<PathBuf, Weak<Mutex<()>>>;
        static LOCKS: OnceLock<Mutex<Locks>> = OnceLock::new();
        // Canonicalize an existing ancestor too: the first owner can be opened
        // before its data directory exists (including through macOS /var symlinks).
        // A later owner must resolve to the same lock after that first write.
        let directory = directory
            .ancestors()
            .find_map(|ancestor| {
                ancestor.canonicalize().ok().and_then(|base| {
                    directory
                        .strip_prefix(ancestor)
                        .ok()
                        .map(|suffix| base.join(suffix))
                })
            })
            .unwrap_or(directory);
        let mut locks = LOCKS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        locks.retain(|_, value| value.strong_count() > 0);
        let mutation = locks
            .get(&directory)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(directory.clone(), Arc::downgrade(&lock));
                lock
            });
        Self {
            directory,
            mutation,
        }
    }

    /// Run a multi-file catalog import/export while excluding conversation writes.
    /// The callback accesses files directly and must not re-enter this store.
    pub(crate) fn with_exclusive<T>(&self, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let _guard = self.mutation.lock().unwrap_or_else(|p| p.into_inner());
        operation()
    }

    fn prepare(&self) -> RepositoryResult<()> {
        fs::create_dir_all(&self.directory)
            .map_err(|e| format!("create conversations directory: {e}"))?;
        Ok(())
    }

    fn path(&self, id: &str) -> RepositoryResult<PathBuf> {
        validate_conversation_id(id)?;
        Ok(self.directory.join(format!("{id}.json")))
    }

    pub fn load(&self, id: &str) -> RepositoryResult<Conversation> {
        let conversation = read_conversation_file(&self.path(id)?, id)?;
        if conversation.id != id {
            return Err(format!("conversation file identity mismatch: {id}").into());
        }
        Ok(conversation)
    }

    /// Only an absent file means a new conversation; corrupt or unreadable data is an error.
    pub fn load_optional(&self, id: &str) -> RepositoryResult<Option<Conversation>> {
        let content = match fs::read_to_string(self.path(id)?) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("读取对话文件失败（{id}）：{error}").into()),
        };
        let conversation: Conversation = serde_json::from_str(&content).map_err(|error| format!("对话文件已损坏，无法加载（{id}）：{error}"))?;
        if conversation.id != id { return Err(format!("conversation file identity mismatch: {id}").into()); }
        Ok(Some(conversation))
    }

    pub fn load_all(&self) -> RepositoryResult<Vec<Conversation>> {
        let _guard = self.mutation.lock().unwrap_or_else(|p| p.into_inner());
        self.prepare()?;
        let mut records = conversation_file_ids_in_dir(&self.directory)?
            .into_iter()
            .map(|id| self.load(&id))
            .collect::<RepositoryResult<Vec<_>>>()?;
        records.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
        Ok(records)
    }

    fn persist(&self, conversation: Conversation) -> RepositoryResult<Conversation> {
        let persisted =
            write_conversation_file_at_path(&self.path(&conversation.id)?, conversation)?;
        // The conversation file is authoritative and has committed. Index failure
        // cannot turn that commit into an apparent failure with a stale revision.
        let update_index = || -> Result<(), String> {
            let (mut index, _) = load_index_or_scan_in_dir(&self.directory)?;
            let item = ConversationListItem::from(&persisted);
            if let Some(position) = index
                .conversations
                .iter()
                .position(|candidate| candidate.id == item.id)
            {
                index.conversations[position] = item;
            } else {
                index.conversations.insert(0, item);
            }
            save_index_in_dir(&self.directory, &index)
        };
        if let Err(error) = update_index() {
            eprintln!(
                "conversation {} committed; index repair pending: {error}",
                persisted.id
            );
        }
        Ok(persisted)
    }

    pub fn create(&self, mut conversation: Conversation) -> RepositoryResult<Conversation> {
        let _guard = self.mutation.lock().unwrap_or_else(|p| p.into_inner());
        self.prepare()?;
        if self.path(&conversation.id)?.exists() {
            return Err(format!("conversation already exists: {}", conversation.id).into());
        }
        conversation.revision = 1;
        self.persist(conversation)
    }

    /// Commit a full edited snapshot only when the caller's baseline is current.
    /// The returned snapshot owns the next revision; never publish success before it arrives.
    pub fn save(
        &self,
        mut conversation: Conversation,
        expected_revision: u64,
    ) -> RepositoryResult<Conversation> {
        let _guard = self.mutation.lock().unwrap_or_else(|p| p.into_inner());
        let latest = self.load(&conversation.id)?;
        validate_expected_revision(&conversation.id, latest.revision, Some(expected_revision))?;
        conversation.revision = latest.revision;
        increment_revision(&mut conversation)?;
        conversation.updated_at = chrono::Local::now().timestamp();
        self.persist(conversation)
    }

    /// Remove the durable record before disposable artifacts. Other domain owners
    /// must release running work and external-session bindings before calling this.
    pub fn delete(&self, id: &str, expected_revision: u64) -> RepositoryResult<Vec<String>> {
        let _guard = self.mutation.lock().unwrap_or_else(|p| p.into_inner());
        let latest = self.load(id)?;
        validate_expected_revision(id, latest.revision, Some(expected_revision))?;
        let (mut index, _) = load_index_or_scan_in_dir(&self.directory)?;
        fs::remove_file(self.path(id)?).map_err(|e| format!("delete conversation file: {e}"))?;
        index.conversations.retain(|item| item.id != id);
        if let Err(error) = save_index_in_dir(&self.directory, &index) {
            eprintln!("conversation {id} deleted; index repair pending: {error}");
        }
        let mut warnings = Vec::new();
        match fs::remove_file(self.directory.join(format!("{id}.draft.jsonl"))) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                warnings.push(format!("草稿日志未能清理：{error}"))
            }
            _ => {}
        }
        let attachments = self.directory.join(format!("{id}_attachments"));
        match fs::remove_dir_all(attachments) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                warnings.push(format!("附件目录未能清理：{error}"));
            }
            _ => {}
        }
        let cleanup_pins = || -> Result<(), String> {
            let mut pins = super::group_catalog::load_conversation_pins(&self.directory)?;
            pins.values_mut()
                .for_each(|pins| pins.retain(|pin| pin.id != id));
            pins.retain(|_, pins| !pins.is_empty());
            super::group_catalog::save_conversation_pins(&self.directory, &pins)
        };
        if let Err(error) = cleanup_pins() {
            warnings.push(format!("固定位置未能清理：{error}"));
        }
        Ok(warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::super::file_io::fail_atomic_replace_for_current_test;
    use super::*;

    #[test]
    fn independent_owners_preserve_rich_history_and_reject_stale_writes() {
        let directory = tempfile::tempdir().unwrap();
        let first = ConversationStore::new(directory.path().to_path_buf());
        let mut conversation: Conversation = serde_json::from_value(serde_json::json!({
            "id":"conv_native_contract", "title":"Imported", "provider_id":"provider", "model":"model",
            "created_at":1, "updated_at":2,
            "agent_runtime":{"kind":"external", "externalAgentId":"claude", "externalModel":"sonnet"},
            "project_id":"proj_work", "folder":"Work", "additional_directories":[{"path":"/extra"}],
            "assistant_snapshot":{"id":"asst_frozen", "name":"Writer", "system_prompt":"Keep this frozen."},
            "messages":[{"id":"msg_answer", "role":"assistant", "content":"Visible answer", "timestamp":2,
                "model_messages":[{"role":"assistant", "content":[{"type":"anthropic_thinking", "thinking":"plan", "signature":"opaque-signature"}]}],
                "api_messages":[{"role":"assistant", "content":"wire transcript"}],
                "artifacts":[{"name":"plot", "mime_type":"image/png", "data_url":"", "path":"plot.png"}],
                "usage":{"input_tokens":41,"output_tokens":9,"total_tokens":50},
                "group_id":"group_one"
            }], "group_selections":{"group_one":"msg_answer"}
        })).unwrap();
        conversation = first.create(conversation).unwrap();
        let second = ConversationStore::new(directory.path().to_path_buf());
        let mut stale = second.load(&conversation.id).unwrap();
        conversation.title = "Renamed".into();
        let saved = first.save(conversation, 1).unwrap();
        stale.title = "Would overwrite".into();
        assert!(matches!(
            second.save(stale, 1),
            Err(ConversationRepositoryError::Conflict {
                expected: 1,
                actual: 2,
                ..
            })
        ));
        let reopened = second.load_all().unwrap();
        assert_eq!(reopened.len(), 1);
        let record = serde_json::to_value(&reopened[0]).unwrap();
        assert_eq!(record["title"], "Renamed");
        assert_eq!(record["agent_runtime"]["externalAgentId"], "claude");
        assert_eq!(
            record["assistant_snapshot"]["system_prompt"],
            "Keep this frozen."
        );
        assert_eq!(
            record["messages"][0]["model_messages"][0]["content"][0]["signature"],
            "opaque-signature"
        );
        assert_eq!(record["messages"][0]["usage"]["total_tokens"], 50);
        assert_eq!(record["messages"][0]["artifacts"][0]["path"], "plot.png");
        assert_eq!(record["group_selections"]["group_one"], "msg_answer");
        let mut failed = saved.clone();
        failed.title = "Must not publish".into();
        fail_atomic_replace_for_current_test(true);
        let result = second.save(failed, 2);
        fail_atomic_replace_for_current_test(false);
        assert!(result.is_err());
        assert_eq!(first.load(&saved.id).unwrap().title, "Renamed");
        assert!(matches!(
            second.delete(&saved.id, 1),
            Err(ConversationRepositoryError::Conflict { .. })
        ));
        second.delete(&saved.id, 2).unwrap();
        assert!(first.load_all().unwrap().is_empty());
        assert!(
            load_index_in_dir(directory.path())
                .unwrap()
                .conversations
                .is_empty()
        );
    }

    #[test]
    fn broken_index_does_not_disguise_committed_save_or_delete_as_failure() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(directory.path().to_path_buf());
        fs::create_dir(directory.path().join("index.json")).unwrap();
        let initial = store
            .create(Conversation::new("provider".into(), "model".into()))
            .unwrap();
        assert_eq!(initial.revision, 1);
        let mut changed = initial;
        changed.title = "Committed even with broken cache".into();
        let saved = store.save(changed, 1).unwrap();
        assert_eq!(saved.revision, 2);
        let reopened = ConversationStore::new(directory.path().to_path_buf());
        assert_eq!(
            reopened.load_all().unwrap()[0].title,
            "Committed even with broken cache"
        );
        reopened.delete(&saved.id, 2).unwrap();
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn competing_owners_cannot_both_commit_the_same_revision() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(directory.path().to_path_buf());
        let initial = store
            .create(Conversation::new("p".into(), "m".into()))
            .unwrap();
        let rendezvous = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|title| {
                let store = ConversationStore::new(directory.path().to_path_buf());
                let mut record = initial.clone();
                let rendezvous = rendezvous.clone();
                std::thread::spawn(move || {
                    record.title = title.into();
                    rendezvous.wait();
                    store.save(record, 1)
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(
                    result,
                    Err(ConversationRepositoryError::Conflict { .. })
                ))
                .count(),
            1
        );
        assert_eq!(store.load(&initial.id).unwrap().revision, 2);
    }
}
