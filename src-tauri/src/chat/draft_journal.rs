//! 中断草稿的 append-only 日志(对齐 Claude Code / Codex 的会话 JSONL 模式)。
//!
//! 工具轮之间的 crash-safety 草稿原来走「整会话 load → 重写 → fsync → 全局索引重写」,
//! 多对话工具密集并发时是最大的磁盘热点(长对话数 MB × 每轮)。现在每轮只把草稿消息
//! **追加**成一行 JSONL(`conversations/{id}.draft.jsonl`),主会话文件与索引在 run 期间
//! 完全不动:
//!
//! - **追加**:每行包含 SDK 完整树、来源分支和 UI 草稿，兼容旧 `ChatMessage` 行。
//!   不 fsync——追加进页缓存后进程崩溃也不丢,只有掉电才丢,和成熟 CLI 的取舍一致。
//! - **清除**(`clear_draft`):终态写入主文件(`push_assistant_message` 的 mutate 成功)后
//!   日志立即失效,尽力删;删不掉也无害,恢复端对「主文件已有同 id 消息」会跳过。
//! - **恢复**(`recover_orphan_drafts`):启动时扫残留日志,按**每个 message_id 的最后一行**
//!   合并回会话文件(一次日志可能叠着多条 run 的草稿——panic 后同会话又跑了新 run)。
//!   在 setup 返回前完成恢复，UI run 不会与日志读取/删除并发。
//!
//! 扩展名刻意是 `.jsonl` 而非 `.json`:`storage::conversation_file_ids_in_dir` 的目录
//! 扫描按扩展名过滤,不会把日志当成会话文件。

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use kivio_agent_sdk::agent::{EntryId, SessionHistory, SessionSnapshot};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::storage;
use super::types::ChatMessage;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CanonicalDraft {
    Session {
        snapshot: SessionSnapshot,
        source_anchor: Option<EntryId>,
        source_branch_id: String,
        draft: ChatMessage,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum DraftEntry {
    Canonical(CanonicalDraft),
    Legacy(ChatMessage),
}

impl DraftEntry {
    fn draft(&self) -> &ChatMessage {
        match self {
            Self::Canonical(CanonicalDraft::Session { draft, .. }) | Self::Legacy(draft) => draft,
        }
    }
}

pub(crate) fn interrupted_snapshot(history: &SessionHistory) -> Result<SessionSnapshot, String> {
    let mut history = history.clone();
    let ancestors: Vec<_> = history
        .active_path()
        .iter()
        .rev()
        .map(|entry| entry.id)
        .collect();
    for leaf in ancestors.into_iter().map(Some).chain(std::iter::once(None)) {
        if history.switch_branch(leaf).is_ok() {
            return history.snapshot();
        }
    }
    Err("Cannot select a complete interrupted history".into())
}

pub(crate) fn append_session_draft(
    app: &AppHandle,
    conversation_id: &str,
    mut draft: ChatMessage,
    mut history: SessionHistory,
    source_anchor: Option<EntryId>,
    source_branch_id: String,
) -> Result<(), String> {
    super::attachments::externalize_message_artifacts(app, conversation_id, &mut draft);
    // A pending call remains an inactive fact; recovery never invents its result.
    let snapshot = interrupted_snapshot(&history)?;
    history = snapshot.into_history();
    super::attachments::externalize_history_media(app, conversation_id, &mut history)?;
    draft.model_messages.clear();
    draft.api_messages.clear();
    let entry = CanonicalDraft::Session {
        snapshot: history.snapshot()?,
        source_anchor,
        source_branch_id,
        draft,
    };
    let mut line = serde_json::to_string(&entry).map_err(|error| error.to_string())?;
    line.push('\n');
    append_line(&storage::draft_journal_path(app, conversation_id)?, &line)
}

/// 超过上限时，只保留每个消息的最后一份完整快照。
const DRAFT_JOURNAL_MAX_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) fn append_draft_at_path(path: &Path, draft: &ChatMessage) -> Result<(), String> {
    let mut line =
        serde_json::to_string(draft).map_err(|e| format!("serialize draft message: {e}"))?;
    line.push('\n');
    append_line(path, &line)
}

fn append_line(path: &Path, line: &str) -> Result<(), String> {
    let oversized = fs::metadata(&path)
        .map(|meta| meta.len() > DRAFT_JOURNAL_MAX_BYTES)
        .unwrap_or(false);
    if oversized {
        let mut content = fs::read_to_string(path)
            .map_err(|error| format!("read capped draft journal: {error}"))?;
        content.push('\n');
        content.push_str(line);
        let mut compacted = String::new();
        for entry in latest_entries_per_message(&content) {
            compacted.push_str(&serde_json::to_string(&entry).map_err(|error| error.to_string())?);
            compacted.push('\n');
        }
        storage::atomic_write(path, &compacted, "draft journal")
    } else {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("open draft journal: {e}"))?;
        file.write_all(line.as_bytes())
            .map_err(|e| format!("append draft journal: {e}"))
    }
}

/// 终态已落主文件,草稿日志随即失效。删除失败不致命(下次启动恢复会识别出
/// 「主文件已有同 id 消息」并清理)。
pub(crate) fn clear_draft(app: &AppHandle, conversation_id: &str) {
    if let Ok(path) = storage::draft_journal_path(app, conversation_id) {
        let _ = fs::remove_file(path);
    }
}

pub(crate) fn stop_fence_path(app: &AppHandle, conversation_id: &str) -> Result<PathBuf, String> {
    Ok(storage::draft_journal_path(app, conversation_id)?.with_extension("stopped"))
}

pub(super) fn stop_fences_user_turn(
    app: &AppHandle,
    conversation_id: &str,
    user_message_id: &str,
) -> Result<bool, String> {
    match fs::read_to_string(stop_fence_path(app, conversation_id)?) {
        Ok(stopped_user) => Ok(stopped_user.trim() == user_message_id),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot read conversation stop marker: {error}")),
    }
}

/// 启动恢复:把崩溃残留的草稿日志合并回会话文件,然后删除日志。
pub(crate) async fn recover_orphan_drafts(app: &AppHandle) {
    let Ok(dir) = storage::conversations_dir(app) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(conversation_id) = name.strip_suffix(".draft.jsonl") else {
            continue;
        };
        recover_one(app, conversation_id, &path).await;
    }
}

async fn recover_one(app: &AppHandle, conversation_id: &str, path: &Path) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let drafts = latest_entries_per_message(&content);
    let result = crate::chat::repository::repository(app)
        .mutate(app, conversation_id, |conversation| {
            merge_drafts(conversation, drafts)
        })
        .await;
    if let Err(error) = result {
        if let Err(error) = crate::chat::repository::recover_committed(app, error).await {
            eprintln!("recover drafts for {conversation_id} failed: {error}");
            return;
        }
    }
    let _ = fs::remove_file(path);
}

fn merge_drafts(
    conversation: &mut super::Conversation,
    drafts: Vec<DraftEntry>,
) -> Result<(), String> {
    let missing = |entry: &DraftEntry| {
        !conversation
            .messages
            .iter()
            .any(|message| message.id == entry.draft().id)
    };
    let has_canonical = conversation.builtin_history.is_some()
        || drafts
            .iter()
            .any(|entry| missing(entry) && matches!(entry, DraftEntry::Canonical(_)));
    if has_canonical
        && drafts
            .iter()
            .any(|entry| missing(entry) && matches!(entry, DraftEntry::Legacy(_)))
    {
        return Err(
            "Legacy draft has no binding in the canonical session; preserving its journal".into(),
        );
    }
    for entry in drafts {
        if conversation
            .messages
            .iter()
            .any(|message| message.id == entry.draft().id)
        {
            continue;
        }
        match entry {
            DraftEntry::Canonical(CanonicalDraft::Session {
                snapshot,
                source_anchor,
                source_branch_id,
                draft,
            }) => {
                super::session_history::commit_interrupted_run(
                    conversation,
                    &draft.id,
                    snapshot.history(),
                    source_anchor,
                    &source_branch_id,
                )?;
                conversation.messages.push(draft);
            }
            DraftEntry::Legacy(draft) => conversation.messages.push(draft),
        }
    }
    Ok(())
}

/// 读当前会话的草稿日志（每个 message_id 最后一行）。日志不存在或读失败时返回空。
/// 给同轮工具（例如 mixer 改图）查找主 JSON 里还没有的 artifact。
pub(crate) fn latest_drafts_for(app: &AppHandle, conversation_id: &str) -> Vec<ChatMessage> {
    let Ok(path) = storage::draft_journal_path(app, conversation_id) else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    latest_drafts_per_message(&content)
}

/// 解析日志:每个 message_id 取最后一行(自包含快照,后写覆盖先写),
/// 保持首次出现的顺序;坏行(崩溃时写了半行)跳过。
pub(crate) fn latest_drafts_per_message(content: &str) -> Vec<ChatMessage> {
    latest_entries_per_message(content)
        .into_iter()
        .map(|entry| entry.draft().clone())
        .collect()
}

fn latest_entries_per_message(content: &str) -> Vec<DraftEntry> {
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, DraftEntry> = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<DraftEntry>(line) else {
            continue;
        };
        let message = entry.draft();
        if !latest.contains_key(&message.id) {
            order.push(message.id.clone());
        }
        latest.insert(message.id.clone(), entry);
    }
    order
        .into_iter()
        .filter_map(|id| latest.remove(&id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn capped_journal_replaces_read_only_snapshot_without_truncating_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kivio-journal-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.draft.jsonl");
        let file = fs::File::create(&path).unwrap();
        file.set_len(DRAFT_JOURNAL_MAX_BYTES + 1).unwrap();
        drop(file);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        let line = format!(
            "{}\n",
            serde_json::to_string(&draft("new", "recoverable")).unwrap()
        );
        let result = append_line(&path, &line);
        assert!(
            result.is_ok(),
            "replace complete snapshot rather than opening old file for truncation: {result:?}"
        );
        let recovered = latest_drafts_per_message(&fs::read_to_string(&path).unwrap());
        assert_eq!(recovered[0].content, "recoverable");
        fs::remove_dir_all(dir).unwrap();
    }

    fn draft(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            irc_message: None,
            id: id.to_string(),
            role: "assistant".to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            reasoning: None,
            artifacts: Vec::new(),
            tool_calls: Vec::new(),
            segments: Vec::new(),
            agent_plan: None,
            api_messages: Vec::new(),
            model_messages: Vec::new(),
            active_skill_id: None,
            run_entry: None,
            stream_outcome: Some("interrupted".to_string()),
            usage: None,
            anchor_usage: None,
            cache_pair_input: None,
            cache_pair_read: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 0,
            degraded: None,
        }
    }

    #[test]
    fn latest_drafts_takes_last_line_per_id_and_skips_bad_lines() {
        let lines = [
            serde_json::to_string(&draft("msg_a", "round 1")).unwrap(),
            "{ broken json".to_string(),
            serde_json::to_string(&draft("msg_b", "other run")).unwrap(),
            serde_json::to_string(&draft("msg_a", "round 2")).unwrap(),
            String::new(),
        ];
        let content = lines.join("\n");
        let drafts = latest_drafts_per_message(&content);
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].id, "msg_a");
        assert_eq!(drafts[0].content, "round 2");
        assert_eq!(drafts[1].id, "msg_b");
    }

    #[test]
    fn latest_drafts_empty_content_yields_nothing() {
        assert!(latest_drafts_per_message("").is_empty());
        assert!(latest_drafts_per_message("\n\n").is_empty());
    }

    #[test]
    fn canonical_journal_recovers_tools_compaction_and_branches_without_replaying_pending_calls() {
        use crate::chat::model::ModelRole;
        use kivio_agent_sdk::agent::{session::CompactionPlan, RuntimeMessage};
        let mut conversation: super::super::Conversation =
            serde_json::from_value(serde_json::json!({
                "id":"conv", "title":"test", "provider_id":"provider", "model":"model",
                "created_at":1, "updated_at":1,
                "messages":[{"id":"user", "role":"user", "content":"question", "timestamp":1}]
            }))
            .unwrap();
        super::super::session_history::ensure_migrated(&mut conversation).unwrap();
        let state = conversation.builtin_history.as_ref().unwrap();
        let source_anchor = state.snapshot.active_leaf();
        let source_branch_id = state.active_branch_id.clone();
        let mut history = state.snapshot.clone().into_history();
        history.push(RuntimeMessage::text(
            ModelRole::Assistant,
            "inactive answer",
        ));
        let inactive = history.active_leaf();
        history.switch_branch(source_anchor).unwrap();
        for value in [
            serde_json::json!({"role":"assistant","tool_calls":[{"id":"completed","type":"function","function":{"name":"write","arguments":"{}"}}]}),
            serde_json::json!({"role":"tool","tool_call_id":"completed","content":"written once"}),
        ] {
            history.push(RuntimeMessage::from_legacy(value).unwrap());
        }
        history
            .compact(CompactionPlan {
                covered: 1..history.context().len(),
                replacement: vec![RuntimeMessage::text(
                    ModelRole::User,
                    "summary of completed write",
                )],
            })
            .unwrap();
        let complete = history.active_leaf();
        history.push(RuntimeMessage::from_legacy(serde_json::json!({
            "role":"assistant","tool_calls":[{"id":"pending","type":"function","function":{"name":"write","arguments":"{}"}}]
        })).unwrap());
        let expected_entries = history.entries().to_vec();
        assert!(history.snapshot().is_err());
        let snapshot = interrupted_snapshot(&history).unwrap();
        assert_eq!(snapshot.active_leaf(), complete);
        let mut ui = draft("reply", "unfinished reply");
        ui.tool_calls = serde_json::from_value(serde_json::json!([
            {"id":"completed","name":"write","source":"native","arguments":"{}","status":"success","result_preview":"written once"},
            {"id":"pending","name":"write","source":"native","arguments":"{}","status":"cancelled","error":"interrupted"}
        ])).unwrap();
        let entry = DraftEntry::Canonical(CanonicalDraft::Session {
            snapshot,
            source_anchor,
            source_branch_id,
            draft: ui,
        });
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("conv.draft.jsonl");
        append_line(&journal, &(serde_json::to_string(&entry).unwrap() + "\n")).unwrap();
        let content = fs::read_to_string(&journal).unwrap();
        assert_eq!(latest_drafts_per_message(&content)[0].tool_calls.len(), 2);
        merge_drafts(&mut conversation, latest_entries_per_message(&content)).unwrap();
        let file = dir.path().join("conv.json");
        fs::write(&file, serde_json::to_vec(&conversation).unwrap()).unwrap();
        let reopened: super::super::Conversation =
            serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
        let restored = reopened
            .builtin_history
            .as_ref()
            .unwrap()
            .snapshot
            .history();
        assert_eq!(restored.entries(), expected_entries);
        assert_eq!(restored.active_leaf(), complete);
        assert!(restored.leaves().contains(&inactive.unwrap()));
        assert!(!serde_json::to_string(restored.context())
            .unwrap()
            .contains("pending"));
        assert!(reopened.messages.last().unwrap().model_messages.is_empty());
        assert_eq!(
            reopened.messages.last().unwrap().tool_calls[1].status,
            super::super::ToolCallStatus::Cancelled
        );
        merge_drafts(&mut conversation, latest_entries_per_message(&content)).unwrap();
        assert_eq!(conversation.messages.len(), 2);
    }

    #[test]
    fn mixed_journal_rejects_unbound_legacy_transcript_without_changing_the_conversation() {
        let mut conversation: super::super::Conversation =
            serde_json::from_value(serde_json::json!({
                "id":"conv", "title":"test", "provider_id":"provider", "model":"model",
                "created_at":1, "updated_at":1, "messages":[]
            }))
            .unwrap();
        super::super::session_history::ensure_migrated(&mut conversation).unwrap();
        let state = conversation.builtin_history.as_ref().unwrap();
        let canonical = DraftEntry::Canonical(CanonicalDraft::Session {
            snapshot: state.snapshot.clone(),
            source_anchor: state.snapshot.active_leaf(),
            source_branch_id: state.active_branch_id.clone(),
            draft: draft("new", "canonical"),
        });
        let mut legacy = draft("old", "legacy");
        legacy
            .api_messages
            .push(serde_json::json!({"role":"assistant", "content":"original fact"}));
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&legacy).unwrap(),
            serde_json::to_string(&canonical).unwrap()
        );
        let before = serde_json::to_value(&conversation).unwrap();
        assert!(merge_drafts(&mut conversation, latest_entries_per_message(&content)).is_err());
        assert_eq!(serde_json::to_value(&conversation).unwrap(), before);
        assert_eq!(
            latest_drafts_per_message(&content)[0].api_messages,
            legacy.api_messages
        );
    }
}
