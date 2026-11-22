use std::collections::{BTreeMap, HashSet};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::chat::{ChatMessage, ToolCallRecord, ToolCallStatus};

pub(crate) type DeferredToolResults = BTreeMap<String, BTreeMap<String, String>>;

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

/// Move eligible text into addressed disclosures while projecting a display row.
/// Its digest survives later conversation revisions with the same result.
pub(crate) fn extract_results(message: &mut ChatMessage) -> Vec<(String, String, String)> {
    if message.role != "assistant"
        || message.id.trim().is_empty()
        || !matches!(message.stream_outcome.as_deref(), None | Some("completed"))
        || !message.tool_calls.iter().any(|tool| {
            tool.result_preview
                .as_ref()
                .is_some_and(|p| p.len() > 4 * 1024)
        })
    {
        return Vec::new();
    }
    let ambiguous = duplicate_ids(message.tool_calls.iter().map(|t| t.id.as_str()));
    message
        .tool_calls
        .iter_mut()
        .filter_map(|tool| {
            if ambiguous.contains(&tool.id) || !ordinary_large_result(tool) {
                return None;
            }
            let result = tool.result_preview.take()?;
            Some((tool.id.clone(), digest(&result), result))
        })
        .collect()
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
            crate::chat::history_projection::read(app, id, |view| {
                view.tool_result(&message_id, &tool_call_id, &expected_hash)
            })
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ambiguous_tool_ids_and_special_results_remain_inline() {
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
        let mut row: ChatMessage = serde_json::from_value(make_message("visible", tools)).unwrap();
        assert!(extract_results(&mut row).is_empty());
        assert!(row
            .tool_calls
            .iter()
            .all(|tool| tool.result_preview.as_deref() == Some(output.as_str())));
    }
}
