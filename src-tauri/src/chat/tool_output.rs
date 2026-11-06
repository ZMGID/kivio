use serde_json::Value;
use tauri::AppHandle;

use super::{
    agent::execute::AgentToolOutput,
    model::{model_messages_from_openai_messages, ModelRole},
    tools::ToolOutput,
};
use crate::mcp::types::{ChatToolArtifact, ChatToolDefinition, McpToolCallResult};

pub(crate) async fn prepare(
    app: &AppHandle,
    conversation_id: &str,
    message_id: &str,
    tool: &ChatToolDefinition,
    arguments: &Value,
    output: McpToolCallResult,
) -> Result<AgentToolOutput, String> {
    let output =
        super::artifacts::prepare_output(app, conversation_id, message_id, tool, arguments, output)
            .await?;
    from_mcp(tool, output)
}

pub(crate) fn from_mcp(
    tool: &ChatToolDefinition,
    mut output: McpToolCallResult,
) -> Result<AgentToolOutput, String> {
    if !output.is_error && tool.name == "present_artifacts" {
        complete_artifact_presentation(&mut output);
    }
    let content = tool_content_with_structured_output(&output, &tool.source);
    let mut follow_up_messages = Vec::new();
    for value in output
        .follow_up_user_messages
        .into_iter()
        .filter(|_| !output.is_error)
    {
        if value["role"] != "user" {
            return Err("Tool follow-up must be a user message".into());
        }
        let messages = model_messages_from_openai_messages(vec![value]);
        if messages.len() != 1
            || messages[0].role != ModelRole::User
            || messages[0].content.is_empty()
        {
            return Err("Tool follow-up contains no supported user content".into());
        }
        follow_up_messages.extend(messages);
    }
    Ok(ToolOutput {
        content,
        structured_content: output.structured_content,
        host_data: output.artifacts,
        follow_up_messages,
        is_error: output.is_error,
    })
}

fn complete_artifact_presentation(output: &mut McpToolCallResult) {
    let Some(structured) = output.structured_content.as_mut() else {
        return;
    };
    let Some(ids) = structured
        .get_mut("artifactIds")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for artifact in &output.artifacts {
        let Some(id) = artifact.id.as_deref() else {
            continue;
        };
        if !ids.iter().any(|value| value.as_str() == Some(id)) {
            ids.push(Value::String(id.to_string()));
        }
    }
    output.raw = structured.clone();
}

fn artifact_presentation_hint(artifacts: &[ChatToolArtifact]) -> Option<String> {
    if artifacts.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    for artifact in artifacts {
        let Some(id) = artifact.id.as_deref() else {
            continue;
        };
        items.push(format!(
            "- {id}: {} ({})",
            artifact.name, artifact.mime_type
        ));
        if let Some(path) = artifact.path.as_deref() {
            items.push(format!(
                "  Local file: {}",
                serde_json::to_string(path).unwrap_or_default()
            ));
        }
    }
    if items.is_empty() {
        return None;
    }
    Some(format!(
        "Available artifacts (not shown automatically):\n{}\nIn your final answer, reference only necessary deliverables at the relevant paragraph: [label](artifact:art_ID) for files, ![description](artifact:art_ID) for images. Replace art_ID with an exact ID listed above. Internal QA screenshots, extracted frames, drafts, and failed attempts normally stay in the work log. Do not pass file contents, base64, or data URLs. Use present_artifacts with paths to prepare selected existing local files; use mode preview only for an explicit user preview or choice.",
        format!("{}\nTo inspect an artifact listed here, call read with artifact_ids containing its exact ID. If a local path is listed, it can also be used by file-based tools. For a user-provided disk path, use read with path directly; no artifact registration is required. Avoid guessing or searching for a generated filename when its exact ID or path is already available. Showing an image does not mean you have visually inspected it.", items.join("\n")),
    ))
}

fn tool_content_with_structured_output(output: &McpToolCallResult, source: &str) -> String {
    let mut content = if source == "native" {
        output.content.clone()
    } else if let Some(structured_content) = output.structured_content.as_ref() {
        let structured_json = serde_json::to_string(structured_content).unwrap_or_default();
        if structured_json.is_empty() || output.content.contains(&structured_json) {
            output.content.clone()
        } else if output.content.trim().is_empty() {
            structured_json
        } else {
            format!(
                "{}\n\nstructuredContent: {}",
                output.content, structured_json
            )
        }
    } else {
        output.content.clone()
    };
    let is_presentation = output
        .structured_content
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        == Some("artifact_presentation");
    if !is_presentation {
        if let Some(hint) = artifact_presentation_hint(&output.artifacts) {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&hint);
        }
    } else if let Some(structured) = output.structured_content.as_ref() {
        // Old records have no mode. New calls need the IDs assigned to local
        // files after execution, including when native content omits raw JSON.
        if structured.get("mode").is_some() {
            if let Some(ids) = structured.get("artifactIds").and_then(Value::as_array) {
                let ids: Vec<&str> = ids.iter().filter_map(Value::as_str).collect();
                content.push_str(&format!(
                    "\n\nRegistered artifact IDs: {}. Use only needed IDs in your final answer: [label](artifact:art_ID) for files or ![description](artifact:art_ID) for images. Do not repeat a separate attachment gallery.",
                    ids.join(", ")
                ));
            }
        }
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_error_keeps_structured_feedback_and_discards_follow_ups() {
        let mut tool = crate::mcp::types::native_read_file_tool();
        tool.source = "mcp".into();
        let output = from_mcp(
            &tool,
            McpToolCallResult {
                content: "failed".into(),
                is_error: true,
                raw: serde_json::json!({"privateTransport":"not model content"}),
                structured_content: Some(serde_json::json!({"reason":"denied"})),
                follow_up_user_messages: vec![
                    serde_json::json!({"role":"assistant","content":"ignored"}),
                ],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(output.is_error);
        assert!(output
            .content
            .contains("structuredContent: {\"reason\":\"denied\"}"));
        assert!(!output.content.contains("privateTransport"));
        assert_eq!(
            output.structured_content,
            Some(serde_json::json!({"reason":"denied"}))
        );
        assert!(output.follow_up_messages.is_empty());
    }
    #[test]
    fn completes_path_based_presentation_with_assigned_ids() {
        let mut output = McpToolCallResult {
            content: "display".to_string(),
            is_error: false,
            raw: serde_json::json!({}),
            artifacts: vec![ChatToolArtifact {
                id: Some("art_local".to_string()),
                name: "local.txt".to_string(),
                mime_type: "text/plain".to_string(),
                data_url: String::new(),
                size_bytes: Some(5),
                path: Some("C:\\tmp\\local.txt".to_string()),
            }],
            structured_content: Some(serde_json::json!({
                "type": "artifact_presentation",
                "artifactIds": ["art_existing"]
            })),
            follow_up_user_messages: Vec::new(),
        };

        complete_artifact_presentation(&mut output);

        let expected = serde_json::json!({
            "type": "artifact_presentation",
            "artifactIds": ["art_existing", "art_local"]
        });
        assert_eq!(output.structured_content, Some(expected.clone()));
        assert_eq!(output.raw, expected);
        assert_eq!(
            tool_content_with_structured_output(&output, "native"),
            "display"
        );
        output.structured_content.as_mut().unwrap()["mode"] = serde_json::json!("prepare");
        let prepared_content = tool_content_with_structured_output(&output, "native");
        assert!(prepared_content.contains("Registered artifact IDs: art_existing, art_local"));
        assert!(prepared_content.contains("[label](artifact:art_ID)"));
    }

    #[test]
    fn artifact_hint_exposes_ids_without_claiming_automatic_display() {
        let artifacts = vec![ChatToolArtifact {
            id: Some("art_report".to_string()),
            name: "report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            data_url: String::new(),
            size_bytes: None,
            path: None,
        }];

        let hint = artifact_presentation_hint(&artifacts).expect("artifact hint");

        assert!(hint.contains("art_report: report.txt (text/plain)"));
        assert!(hint.contains("not shown automatically"));
        assert!(hint.contains("present_artifacts"));
        assert!(hint.contains("[label](artifact:art_ID)"));
        assert!(hint.contains("Internal QA screenshots"));
        assert!(hint.contains("Do not pass file contents, base64, or data URLs"));
        assert!(hint.contains("prepare selected existing local files"));
        assert!(hint.contains("user-provided disk path"));
        assert!(!hint.contains("Do not search the filesystem for generated filenames."));
    }
}
