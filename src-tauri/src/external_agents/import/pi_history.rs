use super::{
    resolve_tool_result, tool_result_text, truncate_tool_result, ImportedImage, ImportedMessage,
    Pending,
};
use crate::external_agents::session::pi_rpc::active_pi_branch;
use serde_json::Value;

struct PiTranscript {
    entries: Vec<Value>,
    linear: bool,
}

impl PiTranscript {
    fn parse(raw: &str) -> Option<Self> {
        let mut lines = raw.lines().filter_map(|line| {
            serde_json::from_str::<Value>(line.trim_start_matches('\u{feff}')).ok()
        });
        let header = lines.next().filter(|value| value["type"] == "session")?;
        Some(Self {
            entries: lines.collect(),
            linear: header["version"].as_u64().unwrap_or(1) < 2,
        })
    }

    fn active_branch(&self) -> Option<Vec<&Value>> {
        if self.linear {
            return Some(self.entries.iter().collect());
        }
        let leaf = self.entries.last()?["id"].as_str()?;
        active_pi_branch(&self.entries, leaf).ok()
    }

    fn name(&self) -> Option<String> {
        let entry = self
            .entries
            .iter()
            .rev()
            .find(|entry| entry["type"] == "session_info")?;
        let name = entry.get("name")?.as_str()?.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }
}

pub(in crate::external_agents) struct PiHistoryPreview {
    pub name: Option<String>,
    pub first_user_text: Option<String>,
}

pub(in crate::external_agents) fn pi_history_preview(raw: &str) -> Option<PiHistoryPreview> {
    let transcript = PiTranscript::parse(raw)?;
    let mut has_content = false;
    let mut first_user_text = None;
    for entry in transcript.active_branch()? {
        let message = &entry["message"];
        if entry["type"] != "message"
            || !matches!(message["role"].as_str(), Some("user" | "assistant"))
        {
            continue;
        }
        if has_content && message["role"] == "assistant" {
            continue;
        }
        let mut pending = Pending::default();
        append_content(&mut pending, &message["content"]);
        has_content |= !pending.is_empty();
        if message["role"] == "user" && !pending.text.is_empty() {
            first_user_text = Some(pending.text);
            break;
        }
    }
    has_content.then(|| PiHistoryPreview {
        name: transcript.name(),
        first_user_text,
    })
}

pub(in crate::external_agents) fn parse_pi_history(raw: &str) -> Vec<ImportedMessage> {
    let Some(transcript) = PiTranscript::parse(raw) else {
        return Vec::new();
    };
    let Some(branch) = transcript.active_branch() else {
        return Vec::new();
    };

    let mut messages = Vec::new();
    let mut pending = Pending::default();
    for entry in branch {
        if entry["type"] != "message" {
            continue;
        }
        let message = &entry["message"];
        match message["role"].as_str() {
            Some(role @ ("user" | "assistant")) => {
                if role == "user" || pending.role != role {
                    messages.extend(std::mem::take(&mut pending).finish());
                    pending.role = role.to_owned();
                    pending.timestamp = message_timestamp(entry, message);
                }
                append_content(&mut pending, &message["content"]);
            }
            Some("toolResult") => apply_tool_result(&mut pending, &mut messages, message),
            _ => {}
        }
    }
    messages.extend(pending.finish());
    messages
}

fn message_timestamp(entry: &Value, message: &Value) -> i64 {
    message["timestamp"].as_i64().unwrap_or_else(|| {
        entry["timestamp"]
            .as_str()
            .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
            .map_or(0, |timestamp| timestamp.timestamp_millis())
    })
}

fn apply_tool_result(pending: &mut Pending, messages: &mut [ImportedMessage], message: &Value) {
    let Some(id) = message["toolCallId"].as_str() else {
        return;
    };
    let result = truncate_tool_result(&tool_result_text(message));
    let is_error = message["isError"].as_bool().unwrap_or(false);
    resolve_tool_result(pending, messages, id, result, is_error);

    let Some(content) = message["content"].as_array() else {
        return;
    };
    if !content.iter().any(|block| block["type"] == "image") {
        return;
    }
    let images = content.iter().filter_map(imported_image);
    if pending.tool_calls.iter().any(|call| call.id == id) {
        pending.images.extend(images);
    } else if let Some(assistant) = messages
        .iter_mut()
        .rev()
        .find(|item| item.message.tool_calls.iter().any(|call| call.id == id))
    {
        assistant.images.extend(images);
    }
}

fn imported_image(block: &Value) -> Option<ImportedImage> {
    if block["type"] != "image" {
        return None;
    }
    let data = block["data"].as_str().filter(|data| !data.is_empty())?;
    Some(ImportedImage {
        media_type: block["mimeType"].as_str().unwrap_or("image/png").to_owned(),
        data_base64: data.to_owned(),
    })
}

fn append_content(pending: &mut Pending, content: &Value) {
    if let Some(text) = content.as_str() {
        pending.push_text(text);
    }
    if let Some(blocks) = content.as_array() {
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => pending.push_text(block["text"].as_str().unwrap_or_default()),
                Some("thinking") => {
                    pending.push_reasoning(block["thinking"].as_str().unwrap_or_default());
                }
                Some("image") => pending.images.extend(imported_image(block)),
                Some("toolCall") => {
                    if let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) {
                        let arguments = block.get("arguments").map_or_else(
                            || "{}".to_owned(),
                            |value| {
                                value
                                    .as_str()
                                    .map_or_else(|| value.to_string(), str::to_owned)
                            },
                        );
                        pending.push_tool(id.to_owned(), name.to_owned(), arguments);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::ToolCallStatus;

    #[test]
    fn imports_only_active_branch_with_reasoning_tools_and_attachments() {
        let raw = r#"
            {"type":"session","version":3,"id":"pi-id","cwd":"/project"}
            {"type":"message","id":"u1","parentId":null,"message":{"role":"user","timestamp":1733234401000,"content":[{"type":"text","text":"question"},{"type":"image","mimeType":"image/png","data":"AA=="}]}}
            {"type":"message","id":"old","parentId":"u1","message":{"role":"assistant","content":"Abandoned answer"}}
            {"type":"message","id":"old-u","parentId":"old","message":{"role":"user","content":"Abandoned question"}}
            {"type":"message","id":"tool","parentId":"u1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Consider this"},{"type":"toolCall","id":"run","name":"bash","arguments":{"command":"echo hello"}}]}}
            {"type":"message","id":"result","parentId":"tool","message":{"role":"toolResult","toolCallId":"run","isError":true,"content":[{"type":"text","text":"RESULT"}]}}
            {"type":"compaction","id":"compact","parentId":"result","summary":"Private summary"}
            {"type":"message","id":"answer","parentId":"compact","message":{"role":"assistant","content":[{"type":"text","text":"Active answer"}]}}
            {"type":"context_edit","id":"edit","parentId":"answer","targetId":"u1","replacement":"Private replacement"}
            {"type":"session_info","id":"name","parentId":"edit","name":"Named session"}
            {"type":"message"
        "#
        .replace("RESULT", &"汉".repeat(900));
        let messages = parse_pi_history(&raw);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message.timestamp, 1733234401);
        assert_eq!(messages[0].message.content, "question");
        assert_eq!(messages[0].images[0].data_base64, "AA==");
        let answer = &messages[1].message;
        assert_eq!(answer.content, "Active answer");
        assert_eq!(answer.reasoning.as_deref(), Some("Consider this"));
        assert_eq!(answer.tool_calls[0].name, "bash");
        assert_eq!(answer.tool_calls[0].status, ToolCallStatus::Error);
        assert!(answer.tool_calls[0]
            .error
            .as_deref()
            .unwrap()
            .contains("已截断"));
        assert!(answer.model_messages.is_empty());
    }

    #[test]
    fn legacy_sessions_keep_distinct_consecutive_user_turns() {
        let messages = parse_pi_history(
            r#"
            {"type":"session","id":"legacy","cwd":"/project"}
            {"type":"message","message":{"role":"user","content":"first"}}
            {"type":"message","timestamp":"2024-12-03T14:00:01Z","message":{"role":"user","content":"second"}}
            {"type":"message","message":{"role":"hookMessage","content":"secret"}}
            {"type":"message","message":{"role":"assistant","content":"answer"}}
        "#,
        );
        let contents: Vec<_> = messages
            .iter()
            .map(|message| message.message.content.as_str())
            .collect();
        assert_eq!(contents, ["first", "second", "answer"]);
        assert_eq!(messages[1].message.timestamp, 1733234401);
    }

    #[test]
    fn image_only_tool_result_attaches_to_the_matching_assistant() {
        let messages = parse_pi_history(
            r#"
            {"type":"session","version":3,"id":"pi-id","cwd":"/project"}
            {"type":"message","id":"first","parentId":null,"message":{"role":"user","content":"Read the image"}}
            {"type":"message","id":"call","parentId":"first","message":{"role":"assistant","content":[{"type":"toolCall","id":"read-image","name":"read","arguments":{"path":"example.png"}}]}}
            {"type":"message","id":"next","parentId":"call","message":{"role":"user","content":"And describe it"}}
            {"type":"message","id":"result","parentId":"next","message":{"role":"toolResult","toolCallId":"read-image","content":[{"type":"image","mimeType":"image/png","data":"AA=="}]}}
            {"type":"message","id":"orphan","parentId":"result","message":{"role":"toolResult","toolCallId":"unmatched","content":[{"type":"image","mimeType":"image/png","data":"BB=="}]}}
        "#,
        );
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].message.role, "assistant");
        assert_eq!(
            messages[1].images,
            [ImportedImage {
                media_type: "image/png".into(),
                data_base64: "AA==".into()
            }]
        );
        assert!(messages[0].images.is_empty());
        assert!(messages[2].images.is_empty());
    }

    #[test]
    fn rejects_incomplete_or_cyclic_active_branches() {
        let header = r#"{"type":"session","version":2,"id":"pi-id","cwd":"/project"}"#;
        for parent in ["missing", "last"] {
            let last = format!(
                r#"{{"type":"message","id":"last","parentId":"{parent}","message":{{"role":"user","content":"cannot verify"}}}}"#,
            );
            assert!(parse_pi_history(&format!("{header}\n{last}")).is_empty());
        }
    }
}
