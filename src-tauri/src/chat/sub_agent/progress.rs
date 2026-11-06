use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::chat::Conversation;

#[derive(Serialize, Deserialize)]
pub(crate) struct TerminalProgress {
    #[serde(default)]
    pub generation: Option<u64>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub progress: Option<Value>,
    pub message_id: Option<String>,
    pub tool_call_id: String,
    pub task_id: String,
    pub name: String,
    pub model: Option<String>,
    pub depth: u8,
    pub status: String,
    pub preview: Option<String>,
    pub steps: Vec<String>,
}

fn directory(app: &AppHandle, conversation_id: &str) -> Result<PathBuf, String> {
    Ok(super::registry::artifacts_dir(app, conversation_id)?.join("task-progress"))
}

pub(crate) fn append(app: &AppHandle, conversation_id: &str, progress: &TerminalProgress) -> Result<(), String> {
    append_in(&directory(app, conversation_id)?, progress)
}

fn append_in(dir: &Path, progress: &TerminalProgress) -> Result<(), String> {
    let data = serde_json::to_string(progress).map_err(|error| format!("serialize task progress: {error}"))?;
    let path = dir.join(format!("{}.json", uuid::Uuid::new_v4()));
    crate::chat::storage::atomic_write(&path, &data, "task progress")
}

pub(crate) fn pending(app: &AppHandle, conversation_id: &str) -> Result<Vec<(PathBuf, TerminalProgress)>, String> {
    pending_in(&directory(app, conversation_id)?)
}

fn pending_in(dir: &Path) -> Result<Vec<(PathBuf, TerminalProgress)>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read task progress directory: {error}")),
    };
    let mut pending = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("read task progress entry: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") { continue; }
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("read task progress: {error}")),
        };
        match serde_json::from_slice(&data) {
            Ok(progress) => pending.push((path, progress)),
            Err(error) => eprintln!("Ignoring invalid task progress {}: {error}", path.display()),
        }
    }
    Ok(pending)
}

fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "aborted" | "cancelled")
}

fn update_async_state(structured: &mut serde_json::Map<String, Value>) {
    let Some(agents) = structured.get("agents").and_then(Value::as_array).filter(|agents| !agents.is_empty()) else { return; };
    let progress = structured.get("subagentProgressByTask").and_then(Value::as_object);
    let mut all_terminal = true;
    let mut failed = false;
    for agent in agents {
        let status = agent.get("agentId").and_then(Value::as_str)
            .and_then(|id| progress.and_then(|entries| entries.get(id)))
            .and_then(|entry| entry.get("status")).and_then(Value::as_str);
        match status {
            Some("completed") => {}
            Some("failed" | "aborted" | "cancelled") => failed = true,
            _ => all_terminal = false,
        }
    }
    let state = if !all_terminal { "running" } else if failed { "failed" } else { "completed" };
    if let Some(aggregate) = structured.get_mut("async").and_then(Value::as_object_mut) {
        aggregate.insert("state".into(), Value::String(state.into()));
    }
}

pub(crate) fn apply(conversation: &mut Conversation, pending: &[(PathBuf, TerminalProgress)]) -> Vec<PathBuf> {
    let mut applied = Vec::new();
    for (path, progress) in pending {
        if !is_terminal(&progress.status) { continue; }
        for message in &mut conversation.messages {
            if progress.message_id.as_ref().is_some_and(|id| id != &message.id) { continue; }
            for tool in &mut message.tool_calls {
                if tool.id != progress.tool_call_id || tool.source != "native"
                    || !matches!(tool.name.as_str(), "task" | "native__task") { continue; }
                let structured = tool.structured_content.get_or_insert_with(|| json!({}));
                let Some(structured) = structured.as_object_mut() else { continue; };
                let agents = structured.get("agents").and_then(Value::as_array);
                let has_agent = agents.is_some_and(|agents| agents.iter().any(|agent| {
                    agent.get("agentId").and_then(Value::as_str) == Some(progress.task_id.as_str())
                }));
                if !has_agent && (progress.message_id.is_none() || agents.is_some_and(|agents| !agents.is_empty())) { continue; }
                let map = structured.entry("subagentProgressByTask").or_insert_with(|| json!({}));
                let Some(map) = map.as_object_mut() else { continue; };
                let existing = map.get(&progress.task_id);
                if !existing.and_then(|value| value.get("status")).and_then(Value::as_str).is_some_and(is_terminal) {
                    map.insert(progress.task_id.clone(), json!({
                        "taskId": progress.task_id, "name": progress.name, "model": progress.model,
                        "generation": progress.generation, "runId": progress.run_id, "progress": progress.progress,
                        "depth": progress.depth, "status": progress.status,
                        "preview": progress.preview, "steps": progress.steps,
                    }));
                } else if let Some(value) = map.get_mut(&progress.task_id).and_then(Value::as_object_mut) {
                    if value.get("status").and_then(Value::as_str) == Some(progress.status.as_str())
                        && value.get("preview").and_then(Value::as_str).is_none_or(str::is_empty)
                    {
                        if let Some(preview) = &progress.preview {
                            value.insert("preview".into(), json!(preview));
                        }
                    }
                }
                update_async_state(structured);

                applied.push(path.clone());
                break;
            }
        }
    }
    applied
}
pub(crate) fn merge_message(previous: &crate::chat::ChatMessage, next: &mut crate::chat::ChatMessage) {
    for tool in &mut next.tool_calls {
        if tool.source != "native" || !matches!(tool.name.as_str(), "task" | "native__task") { continue; }
        let Some(old) = previous.tool_calls.iter().find(|old| {
            old.id == tool.id && old.source == "native" && matches!(old.name.as_str(), "task" | "native__task")
        }) else { continue; };
        let Some(old_map) = old.structured_content.as_ref()
            .and_then(|value| value.get("subagentProgressByTask"))
            .and_then(Value::as_object) else { continue; };
        for (id, progress) in old_map {
            if !progress.get("status").and_then(Value::as_str).is_some_and(is_terminal) { continue; }
            let agents = tool.structured_content.as_ref().and_then(|value| value.get("agents")).and_then(Value::as_array);
            if agents.is_some_and(|agents| !agents.is_empty() && !agents.iter().any(|agent| {
                agent.get("agentId").and_then(Value::as_str) == Some(id.as_str())
            })) { continue; }
            let structured = tool.structured_content.get_or_insert_with(|| json!({}));
            let Some(structured) = structured.as_object_mut() else { continue; };
            let map = structured.entry("subagentProgressByTask").or_insert_with(|| json!({}));
            let Some(map) = map.as_object_mut() else { continue; };
            map.insert(id.clone(), progress.clone());
        }
        if let Some(structured) = tool.structured_content.as_mut().and_then(Value::as_object_mut) {
            update_async_state(structured);
        }
    }
}

pub(crate) fn clear_applied(paths: &[PathBuf]) {
    for path in paths {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Cannot remove embedded task progress {}: {error}", path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(id: &str, status: &str, preview: &str) -> TerminalProgress {
        TerminalProgress {
            generation: Some(1), run_id: Some("child-run".into()), progress: Some(json!({"requests":2,"cost":0.1})),
            message_id: Some("message".into()), tool_call_id: "call".into(), task_id: id.into(),
            name: id.into(), model: None, depth: 1, status: status.into(),
            preview: Some(preview.into()), steps: Vec::new(),
        }
    }

    fn conversation() -> Conversation {
        serde_json::from_value(json!({
            "id": "conv_progress", "title": "test", "provider_id": "p", "model": "m",
            "created_at": 1, "updated_at": 1, "messages": []
        })).unwrap()
    }

    fn receipt() -> crate::chat::ChatMessage {
        serde_json::from_value(json!({
            "id": "message", "role": "assistant", "content": "Finished", "timestamp": 2,
            "tool_calls": [{
                "id": "call", "name": "task", "source": "native", "status": "success",
                "structured_content": {
                    "async": {"state":"running"},
                    "agents": [
                        {"agentId": "inline", "name": "inline"},
                        {"agentId": "async", "name": "async", "jobId": "job-async"}
                    ],
                    "subagentProgressByTask": {
                        "inline": {"status": "pending"},
                        "async": {"status": "running"}
                    }
                }
            }]
        })).unwrap()
    }

    #[test]
    fn terminal_events_before_receipt_are_embedded_and_survive_a_fresh_disk_read() {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("task-progress");
        let conversation_file = dir.path().join("conversation.json");
        append_in(&journal, &progress("inline", "completed", "Inline result")).unwrap();
        append_in(&journal, &progress("async", "failed", "Async diagnostic")).unwrap();

        let mut saved = conversation();
        assert!(apply(&mut saved, &pending_in(&journal).unwrap()).is_empty());
        assert_eq!(pending_in(&journal).unwrap().len(), 2);
        saved.messages.push(receipt());
        let (inline, background): (Vec<_>, Vec<_>) = pending_in(&journal).unwrap()
            .into_iter().partition(|(_, entry)| entry.task_id == "inline");
        let mut applied = apply(&mut saved, &inline);
        assert_eq!(saved.messages[0].tool_calls[0].structured_content.as_ref().unwrap()["async"]["state"], "running");
        applied.extend(apply(&mut saved, &background));
        assert_eq!(applied.len(), 2);
        crate::chat::storage::atomic_write(&conversation_file, &serde_json::to_string(&saved).unwrap(), "test").unwrap();
        clear_applied(&applied);
        assert!(pending_in(&journal).unwrap().is_empty());

        let reloaded = crate::chat::storage::read_conversation_file(&conversation_file, "conv_progress").unwrap();
        let structured = reloaded.messages[0].tool_calls[0].structured_content.as_ref().unwrap();
        assert_eq!(structured["subagentProgressByTask"]["inline"]["status"], "completed");
        assert_eq!(structured["subagentProgressByTask"]["inline"]["preview"], "Inline result");
        assert_eq!(structured["subagentProgressByTask"]["async"]["status"], "failed");
        assert_eq!(structured["subagentProgressByTask"]["async"]["preview"], "Async diagnostic");
        assert_eq!(structured["async"]["state"], "failed");
        assert_eq!(structured["agents"][1]["jobId"], "job-async");
    }

    #[test]
    fn journal_does_not_attach_to_a_different_agent_or_replace_a_terminal_result() {
        let mut saved = conversation();
        saved.messages.push(receipt());
        saved.messages[0].tool_calls[0].structured_content.as_mut().unwrap()
            ["subagentProgressByTask"].as_object_mut().unwrap().remove("async");
        let mut other = progress("other", "completed", "wrong result");
        other.message_id = None;
        let pending = vec![(PathBuf::from("other.json"), other)];
        assert!(apply(&mut saved, &pending).is_empty());
        let applied = vec![(PathBuf::from("current.json"), progress("inline", "completed", "persisted result"))];
        assert_eq!(apply(&mut saved, &applied).len(), 1);
        assert_eq!(saved.messages[0].tool_calls[0].structured_content.as_ref().unwrap()["async"]["state"], "running");
        let done = vec![(PathBuf::from("done.json"), progress("async", "completed", "background result"))];
        assert_eq!(apply(&mut saved, &done).len(), 1);
        assert_eq!(saved.messages[0].tool_calls[0].structured_content.as_ref().unwrap()["async"]["state"], "completed");
        let stale = vec![(PathBuf::from("stale.json"), progress("inline", "failed", "stale result"))];
        assert_eq!(apply(&mut saved, &stale).len(), 1);
        let structured = saved.messages[0].tool_calls[0].structured_content.as_ref().unwrap();
        assert_eq!(structured["subagentProgressByTask"]["inline"]["status"], "completed");
        assert_eq!(structured["subagentProgressByTask"]["inline"]["preview"], "persisted result");
        assert!(structured["subagentProgressByTask"].get("other").is_none());
        let mut replacement = receipt();
        replacement.tool_calls[0].structured_content = Some(json!({
            "agents":[{"agentId":"replacement"}], "subagentProgressByTask":{}
        }));
        merge_message(&saved.messages[0], &mut replacement);
        assert!(replacement.tool_calls[0].structured_content.as_ref().unwrap()
            ["subagentProgressByTask"].get("inline").is_none());
    }
}
