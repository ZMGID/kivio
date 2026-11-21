use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

use serde::Deserialize;
use serde_json::{value::RawValue, Value};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::chat::{ChatMessage, ToolCallRecord, ToolCallStatus};

pub(super) type DeferredToolResults = BTreeMap<String, BTreeMap<String, String>>;

fn digest(preview: &str) -> String {
    format!("{:x}", Sha256::digest(preview.as_bytes()))
}

fn ordinary_large_result(tool: &ToolCallRecord) -> bool {
    let Some(preview) = tool.result_preview.as_deref() else {
        return false;
    };
    if preview.len() <= 4 * 1024
        || tool.status != ToolCallStatus::Success
        || tool.sensitive
        || tool.id.trim().is_empty()
        || !matches!(tool.source.as_str(), "native" | "external_cli")
        || tool.structured_content.is_some()
        || !tool.artifacts.is_empty()
        || tool.error.is_some()
        || preview.trim().is_empty()
    {
        return false;
    }
    let name = tool.name.to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "read" | "read_file" | "bash" | "shell" | "run_command"
    ) {
        return false;
    }
    let Ok(Value::Object(args)) = serde_json::from_str(&tool.arguments) else {
        return false;
    };
    if args.get("_kivioToolDraft").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if matches!(name.as_str(), "read" | "read_file") {
        // Multi-file and image reads have specialized cards. Defer plain file reads only.
        if args.contains_key("paths") {
            return false;
        }
        let path = ["path", "file_path", "filePath"]
            .iter()
            .find_map(|key| args.get(*key)?.as_str());
        let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
            return false;
        };
        if path.rsplit_once('.').is_some_and(|(_, ext)| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "heif"
            )
        }) {
            return false;
        }
    }
    true
}

fn duplicate_ids<'a>(ids: impl Iterator<Item = &'a str>) -> HashSet<String> {
    let mut seen = HashSet::new();
    ids.filter(|id| !seen.insert(*id))
        .map(str::to_owned)
        .collect()
}

/// Only called on a read-view copy, after selecting the original byte-bounded window.
/// The digest identifies the result independently of later turns and conversation revisions.
pub(super) fn defer_window_results(
    messages: &mut [ChatMessage],
    range: Range<usize>,
) -> DeferredToolResults {
    let duplicate_messages = duplicate_ids(messages.iter().map(|m| m.id.as_str()));
    let mut deferred = DeferredToolResults::new();
    for message in &mut messages[range] {
        if message.role != "assistant"
            || message.id.trim().is_empty()
            || duplicate_messages.contains(&message.id)
            || !matches!(message.stream_outcome.as_deref(), None | Some("completed"))
            || !message.tool_calls.iter().any(|tool| {
                tool.result_preview
                    .as_ref()
                    .is_some_and(|p| p.len() > 4 * 1024)
            })
        {
            continue;
        }
        let duplicate_tools = duplicate_ids(message.tool_calls.iter().map(|t| t.id.as_str()));
        for tool in &mut message.tool_calls {
            if !duplicate_tools.contains(&tool.id) && ordinary_large_result(tool) {
                if let Some(preview) = tool.result_preview.take() {
                    deferred
                        .entry(message.id.clone())
                        .or_default()
                        .insert(tool.id.clone(), digest(&preview));
                }
            }
        }
    }
    deferred
}

/// Borrow raw nested JSON so opening one disclosure never materializes other tool
/// results, attachments, segments, or either replay transcript.
fn read_result(
    source: &str,
    message_id: &str,
    tool_call_id: &str,
    expected_hash: &str,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Snapshot<'a> {
        #[serde(borrow)]
        messages: Vec<Message<'a>>,
    }
    #[derive(Deserialize)]
    struct Message<'a> {
        #[serde(borrow)]
        id: Cow<'a, str>,
        #[serde(default, borrow)]
        tool_calls: Option<&'a RawValue>,
    }
    #[derive(Deserialize)]
    struct Tool<'a> {
        #[serde(borrow)]
        id: Cow<'a, str>,
        #[serde(default, borrow)]
        result_preview: Option<&'a RawValue>,
    }
    let corrupt = |err: serde_json::Error| format!("读取工具结果失败：{err}");
    let snapshot: Snapshot = serde_json::from_str(source).map_err(corrupt)?;
    let mut messages = snapshot.messages.into_iter().filter(|m| m.id == message_id);
    let calls = messages
        .next()
        .and_then(|m| m.tool_calls)
        .ok_or("工具结果已不存在")?;
    if messages.next().is_some() {
        return Err("消息标识重复，无法定位工具结果".into());
    }
    let tools: Vec<Tool> = serde_json::from_str(calls.get()).map_err(corrupt)?;
    let mut matching = tools.into_iter().filter(|t| t.id == tool_call_id);
    let raw = matching
        .next()
        .and_then(|t| t.result_preview)
        .ok_or("工具结果已不存在")?;
    if matching.next().is_some() {
        return Err("工具标识重复，无法定位工具结果".into());
    }
    let result: String = serde_json::from_str(raw.get()).map_err(corrupt)?;
    if digest(&result) != expected_hash {
        return Err("工具结果已变更，请重新打开对话".into());
    }
    Ok(result)
}

#[tauri::command]
pub(crate) async fn chat_get_tool_result(
    app: AppHandle,
    conversation_id: String,
    message_id: String,
    tool_call_id: String,
    expected_hash: String,
) -> Result<String, String> {
    crate::chat::repository::repository(&app)
        .with_existing_conversation_storage(&app, &conversation_id, move |app, id| {
            let path = crate::chat::storage::conversation_file_path(app, id)?;
            let source =
                std::fs::read_to_string(path).map_err(|err| format!("读取工具结果失败：{err}"))?;
            read_result(&source, &message_id, &tool_call_id, &expected_hash)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn window_result_round_trips_after_an_append_but_rejects_replacement_or_removal() {
        let output = "文件内容\nquoted \"value\"\n".repeat(300);
        let tool = json!({
            "id": "read-id", "name": "Read", "source": "native", "status": "success",
            "arguments": r#"{"path":"data.csv"}"#, "result_preview": output
        });
        let message = json!({"id":"answer", "role":"assistant", "content":"answer", "timestamp":1,
                             "tool_calls":[tool]});
        let mut store = json!({"messages":[message]});
        let mut rows: Vec<ChatMessage> = serde_json::from_value(store["messages"].clone()).unwrap();
        let deferred = defer_window_results(&mut rows, 0..1);
        assert!(rows[0].tool_calls[0].result_preview.is_none());
        let hash = &deferred["answer"]["read-id"];
        store["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"next"}));
        let read = |store: &Value| read_result(&store.to_string(), "answer", "read-id", hash);
        assert_eq!(read(&store).unwrap(), output);
        store["messages"][0]["tool_calls"][0]["result_preview"] = json!("changed");
        assert!(read(&store).is_err());
        store["messages"][0]["tool_calls"] = json!([]);
        assert!(read(&store).is_err());
    }

    #[test]
    fn off_page_ambiguous_and_special_results_remain_inline() {
        let output = "x".repeat(5000);
        let tool = json!({
            "id": "read-id", "name": "Read", "source": "external_cli", "status": "success",
            "arguments": r#"{"path":"data.csv"}"#, "result_preview": output
        });
        let make_message = |id: &str, tools: Vec<Value>| {
            json!({
                "id":id, "role":"assistant", "content":"answer", "timestamp":1, "tool_calls":tools
            })
        };

        let mut tools = vec![tool.clone(), tool.clone()];
        for (id, field, value) in [
            ("image", "arguments", json!(r#"{"path":"image.png"}"#)),
            (
                "draft",
                "arguments",
                json!(r#"{"command":"ls","_kivioToolDraft":true}"#),
            ),
            ("failed", "status", json!("error")),
            ("structured", "structured_content", json!({"type":"patch"})),
            ("sensitive", "sensitive", json!(true)),
        ] {
            let mut special = tool.clone();
            special["id"] = json!(id);
            special[field] = value;
            tools.push(special);
        }
        let mut rows: Vec<ChatMessage> = serde_json::from_value(json!([
            make_message("duplicate", vec![tool.clone()]),
            make_message("older", vec![tool.clone()]),
            make_message("duplicate", vec![tool]),
            make_message("visible", tools),
        ]))
        .unwrap();
        assert!(defer_window_results(&mut rows, 2..4).is_empty());
        assert!(rows
            .iter()
            .flat_map(|row| &row.tool_calls)
            .all(|tool| tool.result_preview.as_deref() == Some(output.as_str())));
    }
}
