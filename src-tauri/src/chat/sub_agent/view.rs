use std::io::SeekFrom;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use ts_rs::TS;

use super::registry::{AgentRecord, AgentStatus};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SubagentView {
    pub id: String,
    pub parent_id: String,
    pub generation: u64,
    pub display_name: String,
    pub agent_type: String,
    pub status: AgentStatus,
    pub depth: u8,
    pub last_activity: i64,
    pub activity: Option<String>,
    pub model: Option<String>,
    pub progress: Option<Value>,
    pub revivable: bool,
    pub revive_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SubagentRoster {
    pub agents: Vec<SubagentView>,
    pub running_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SubagentTranscriptCursor {
    pub file_id: String,
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SubagentTranscriptKind { Message, Compaction, Init, Notice }

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SubagentTranscriptEntry {
    pub id: String,
    pub kind: SubagentTranscriptKind,
    pub role: String,
    pub content: String,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SubagentTranscriptPage {
    pub id: String,
    pub generation: u64,
    pub cursor: SubagentTranscriptCursor,
    pub reset: bool,
    pub entries: Vec<SubagentTranscriptEntry>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SubagentSendReceipt {
    pub id: String,
    pub recipient: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SubagentIrcMessage {
    pub id: String,
    pub conversation_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub progress: bool,
    pub broadcast: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub wake_relay: Option<bool>,
    pub observation: bool,
    pub timestamp: i64,
}

pub async fn persist_incoming(app: &AppHandle, incoming: &SubagentIrcMessage) -> Result<(), String> {
    if incoming.observation { return Err("Relay observations cannot enter conversation history".into()); }
    crate::chat::storage::validate_conversation_id(&incoming.conversation_id)?;
    let id = format!("subagent-irc-{}", incoming.id);
    let message: crate::chat::ChatMessage = serde_json::from_value(json!({
        "id": id, "role": "assistant", "content": incoming.content,
        "timestamp": incoming.timestamp, "irc_message": incoming,
    })).map_err(|error| error.to_string())?;
    crate::chat::repository::repository(app).mutate(app, &incoming.conversation_id, |conversation| {
        if !conversation.messages.iter().any(|item| item.id == id) {
            conversation.messages.push(message);
        }
        Ok(())
    }).await.map_err(crate::chat::repository::repository_error)?;
    app.emit("subagent-irc", incoming).map_err(|error| error.to_string())
}

pub fn model_input(incoming: &SubagentIrcMessage) -> Value {
    let content = super::prompts::render(include_str!("prompts/irc-incoming.md"), &json!({
        "from": super::delivery::escape_harness_tags(&incoming.from_agent),
        "replyTo": incoming.reply_to, "progress": incoming.progress,
        "message": super::delivery::escape_harness_tags(&incoming.content),
        "interrupting": false, "relayOnStop": !incoming.broadcast && !incoming.progress && !incoming.wake_relay.unwrap_or(false) && incoming.from_agent != "@user",
    }));
    let mut input = super::delivery::report_input(&content);
    input[super::delivery::REPORT_MESSAGE_ID_KEY] = json!(format!("subagent-irc-{}", incoming.id));
    input
}

pub fn project(app: &AppHandle, record: AgentRecord) -> SubagentView {
    let state = app.state::<AppState>();
    let latest = state.sub_agents.jobs.running(&record.root, &record.parent_id).into_iter()
        .filter(|job| job.agent_id == record.id).max_by_key(|job| job.started_at);
    let progress = latest.and_then(|job| job.progress);
    let model = progress.as_ref().and_then(|value| value["model"].as_str()).map(str::to_string);
    SubagentView {
        id: record.id, parent_id: record.parent_id, generation: record.generation,
        display_name: record.display_name, agent_type: record.agent_type, status: record.status,
        depth: record.depth, last_activity: record.last_activity, activity: record.activity,
        model, progress, revivable: record.session_file.is_file(), revive_error: None,
    }
}

fn message_text(value: &Value) -> String {
    if let Some(text) = value.as_str() { return text.to_string(); }
    if let Some(parts) = value.as_array() {
        return parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n");
    }
    String::new()
}

fn transcript_entry(value: Value, offset: u64) -> SubagentTranscriptEntry {
    let message = value.get("message").unwrap_or(&value);
    let kind = match value["type"].as_str() {
        Some("session_init" | "session") => SubagentTranscriptKind::Init,
        Some("compaction") => SubagentTranscriptKind::Compaction,
        _ if message["role"].is_string() => SubagentTranscriptKind::Message,
        _ => SubagentTranscriptKind::Notice,
    };
    let role = message["role"].as_str().unwrap_or_default().to_string();
    let mut content = message_text(&message["content"]);
    if let Some(calls) = message["tool_calls"].as_array() {
        for call in calls {
            let function = &call["function"];
            let name = function["name"].as_str().unwrap_or("tool");
            content.push_str(&format!("\n{name}\n{}", function["arguments"].as_str().unwrap_or_default()));
        }
    }
    if content.is_empty() {
        content = match kind {
            SubagentTranscriptKind::Init => format!("{}\n{}", value["agent"].as_str().unwrap_or("session"), value["task"].as_str().unwrap_or_default()),
            SubagentTranscriptKind::Compaction => "Context compacted. Earlier messages remain in this transcript.".into(),
            _ => serde_json::to_string_pretty(&value).unwrap_or_default(),
        };
    }
    SubagentTranscriptEntry { id: format!("line-{offset}"), kind, role, content, timestamp: value["timestamp"].as_i64() }
}

async fn read_page(path: &Path, id: &str, generation: u64, cursor: Option<SubagentTranscriptCursor>) -> Result<SubagentTranscriptPage, String> {
    const PAGE_BYTES: usize = 512 * 1024;
    const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;
    let mut file = tokio::fs::File::open(path).await.map_err(|error| format!("Cannot open agent history: {error}"))?;
    let metadata = file.metadata().await.map_err(|error| error.to_string())?;
    if !metadata.is_file() { return Err("Agent history is not a regular file".into()); }
    let mut header = vec![0; metadata.len().min(4096) as usize];
    file.read_exact(&mut header).await.map_err(|error| error.to_string())?;
    header.truncate(header.iter().position(|byte| *byte == b'\n').unwrap_or(header.len()));
    let creation = metadata.created().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|time| time.as_nanos());
    let file_id = format!("{creation:?}-{:x}", Sha256::digest(&header));
    let reset = cursor.as_ref().is_some_and(|cursor| cursor.file_id != file_id || cursor.offset > metadata.len());
    let start = if reset { 0 } else { cursor.as_ref().map_or(0, |cursor| cursor.offset) };
    if start > 0 {
        file.seek(SeekFrom::Start(start - 1)).await.map_err(|error| error.to_string())?;
        let mut previous = [0];
        file.read_exact(&mut previous).await.map_err(|error| error.to_string())?;
        if previous[0] != b'\n' { return Err("Invalid agent history cursor".into()); }
    }
    file.seek(SeekFrom::Start(start)).await.map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    (&mut file).take(MAX_LINE_BYTES + 1).read_to_end(&mut bytes).await.map_err(|error| error.to_string())?;
    let mut used = 0;
    let mut entries = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !line.ends_with(b"\n") { break; }
        if used > 0 && used + line.len() > PAGE_BYTES { break; }
        let value = match serde_json::from_slice(line) {
            Ok(value) => value,
            Err(error) => json!({"type":"notice", "content":format!("Unreadable transcript record at byte {}: {error}", start + used as u64)}),
        };
        entries.push(transcript_entry(value, start + used as u64));
        used += line.len();
    }
    if used == 0 && bytes.len() as u64 > MAX_LINE_BYTES { return Err("Agent history record exceeds 8 MiB".into()); }
    let offset = start + used as u64;
    let has_more = bytes[used..].contains(&b'\n');
    Ok(SubagentTranscriptPage { id: id.into(), generation, cursor: SubagentTranscriptCursor { file_id, offset }, reset, entries, has_more })
}

pub async fn transcript(app: &AppHandle, root: &str, id: &str, expected_generation: u64, cursor: Option<SubagentTranscriptCursor>) -> Result<SubagentTranscriptPage, String> {
    crate::chat::storage::validate_conversation_id(root)?;
    let state = app.state::<AppState>();
    let record = state.sub_agents.registry.resolve(root, id).ok_or("Agent not found")?;
    if !record.published || record.generation != expected_generation { return Err("Agent changed. Refresh its snapshot.".into()); }
    read_page(&record.session_file, &record.id, record.generation, cursor).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transcript_only_advances_complete_lines_and_resets_after_truncation() {
        let dir = std::env::temp_dir().join(format!("kivio-agent-view-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let path = dir.join("A.jsonl");
        let first = "{\"type\":\"session_init\",\"agent\":\"task\"}\n";
        tokio::fs::write(&path, format!("{first}{{\"role\":\"assistant\",\"content\":\"你好")).await.unwrap();
        let page = read_page(&path, "A", 1, None).await.unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.cursor.offset, first.len() as u64);
        assert!(!page.has_more);
        tokio::fs::write(&path, format!("{first}{{\"role\":\"assistant\",\"content\":\"你好\"}}\n")).await.unwrap();
        let second = read_page(&path, "A", 1, Some(page.cursor)).await.unwrap();
        assert!(!second.reset);
        assert_eq!(second.entries[0].content, "你好");
        tokio::fs::write(&path, first).await.unwrap();
        let reset = read_page(&path, "A", 1, Some(second.cursor)).await.unwrap();
        assert!(reset.reset);
        assert_eq!(reset.entries.len(), 1);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn transcript_displays_tools_without_executing_them() {
        let entry = transcript_entry(json!({"role":"assistant", "tool_calls":[{"function":{"name":"write","arguments":"{\"path\":\"example\"}"}}]}), 12);
        assert_eq!(entry.kind, SubagentTranscriptKind::Message);
        assert!(entry.content.contains("write"));
        assert!(entry.content.contains("example"));
    }

    #[test]
    fn persisted_automatic_replies_do_not_create_another_reply_obligation() {
        let mut incoming: SubagentIrcMessage = serde_json::from_value(json!({"id":"answer","conversationId":"root","fromAgent":"Peer","toAgent":"Main","content":"done","replyTo":"question","progress":false,"broadcast":false,"wakeRelay":true,"observation":false,"timestamp":1})).unwrap();
        let automatic = model_input(&incoming);
        assert!(!automatic["content"].as_str().unwrap().contains("delivered to"));
        incoming.wake_relay = Some(false);
        assert!(model_input(&incoming)["content"].as_str().unwrap().contains("delivered to"));
    }
}
