use serde_json::Value;

use crate::mcp::ChatToolDefinition;

use super::types::{
    model_message_from_openai_message, openai_message_text_content, GenerateOptions,
    GenerateRequest, GenerateRequestContext, MessagePart, ModelMessage, ModelRole, ModelTool,
    RequestMetadata,
};

pub fn generate_request_from_openai_messages(
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    options: GenerateOptions,
    label: &str,
    context: GenerateRequestContext,
) -> GenerateRequest {
    let mut system_parts = Vec::new();
    let mut model_messages = Vec::new();
    for message in messages {
        let role = message
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if role == "system" {
            if let Some(content) = openai_message_text_content(&message) {
                system_parts.push(content);
            }
            continue;
        }
        if let Some(model_message) = model_message_from_openai_message(&message) {
            model_messages.push(model_message);
        }
    }
    generate_request_from_model_messages(
        model,
        system_parts,
        model_messages,
        tools
            .unwrap_or_default()
            .iter()
            .map(ModelTool::from)
            .collect(),
        options,
        RequestMetadata {
            label: label.to_string(),
            conversation_id: context.conversation_id,
            message_id: context.message_id,
            ..RequestMetadata::default()
        },
    )
}

pub fn generate_request_from_model_messages(
    model: &str,
    mut system_parts: Vec<String>,
    mut model_messages: Vec<ModelMessage>,
    tools: Vec<ModelTool>,
    options: GenerateOptions,
    metadata: RequestMetadata,
) -> GenerateRequest {
    if let Some(first) = system_parts.first_mut() {
        if let Some((prefix, suffix)) = split_workbench_system_suffix(first) {
            *first = prefix;
            if !prepend_text_to_first_user_message(&mut model_messages, &suffix) {
                // No user turn to park it on — keep the paragraph in system.
                if first.is_empty() {
                    *first = suffix;
                } else {
                    first.push_str("\n\n");
                    first.push_str(&suffix);
                }
            }
        }
    }
    system_parts.retain(|part| !part.trim().is_empty());
    GenerateRequest {
        model: model.to_string(),
        system: system_parts.join("\n\n"),
        messages: model_messages,
        tools,
        options,
        metadata,
    }
}

/// Last paragraph of the chat system prompt when a per-conversation workbench
/// is in play. Must stay last so `split_workbench_system_suffix` can peel it
/// off the stable prefix (cross-conversation prompt cache).
pub const WORKBENCH_LOCATION_PROMPT_HEAD: &str = "Current default workbench:";

/// Pull the per-conversation workbench paragraph out of the system prompt.
///
/// Ordinary chats interpolate `…/conv_<id>` into that paragraph, which would
/// otherwise sit in the middle of `system` and break provider prefix-cache
/// matching for everything after it (skills + tool schemas). Callers park the
/// suffix on the first user message so it lands *after* tools in the token
/// stream.
pub fn split_workbench_system_suffix(system: &str) -> Option<(String, String)> {
    let mut paras: Vec<&str> = system.split("\n\n").collect();
    let idx = paras
        .iter()
        .rposition(|para| para.trim().starts_with(WORKBENCH_LOCATION_PROMPT_HEAD))?;
    let suffix = paras.remove(idx).trim().to_string();
    if suffix.is_empty() {
        return None;
    }
    let prefix = paras
        .into_iter()
        .map(str::trim)
        .filter(|para| !para.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    Some((prefix, suffix))
}

pub fn prepend_text_to_first_user_message(messages: &mut [ModelMessage], prefix: &str) -> bool {
    let Some(message) = messages.iter_mut().find(|m| m.role == ModelRole::User) else {
        return false;
    };
    if let Some(MessagePart::Text { text }) = message
        .content
        .iter_mut()
        .find(|part| matches!(part, MessagePart::Text { .. }))
    {
        if text.is_empty() {
            *text = prefix.to_string();
        } else {
            *text = format!("{prefix}\n\n{text}");
        }
    } else {
        message.content.insert(
            0,
            MessagePart::Text {
                text: prefix.to_string(),
            },
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_workbench_system_suffix_extracts_path_even_when_not_last() {
        let system = "static role\n\n\
Current default workbench: `/tmp/conv_abc`. When the user does not specify a location, use relative paths or the default cwd so files and basic work land here.\n\n\
MCP note after the path.";
        let (prefix, suffix) = split_workbench_system_suffix(system).expect("suffix");
        assert_eq!(prefix, "static role\n\nMCP note after the path.");
        assert!(suffix.starts_with(WORKBENCH_LOCATION_PROMPT_HEAD));
        assert!(suffix.contains("conv_abc"));
        assert!(!prefix.contains("conv_abc"));
    }

    #[test]
    fn generate_request_parks_workbench_on_first_user_not_system() {
        let workbench = format!(
            "{WORKBENCH_LOCATION_PROMPT_HEAD} `/tmp/conv_abc`. When the user does not specify a location, use relative paths or the default cwd so files and basic work land here."
        );
        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": format!("You are Kivio.\n\n{workbench}"),
            }),
            serde_json::json!({ "role": "user", "content": "hello" }),
            serde_json::json!({ "role": "assistant", "content": "hi" }),
            serde_json::json!({ "role": "user", "content": "next" }),
        ];
        let request = generate_request_from_openai_messages(
            "m",
            messages,
            None,
            Default::default(),
            "t",
            Default::default(),
        );
        assert_eq!(request.system, "You are Kivio.");
        assert!(!request.system.contains("conv_abc"));

        let first_user = request
            .messages
            .iter()
            .find(|m| m.role == ModelRole::User)
            .expect("first user");
        let MessagePart::Text { text } = &first_user.content[0] else {
            panic!("expected text part");
        };
        assert!(text.starts_with(&workbench));
        assert!(text.ends_with("hello"));

        let last_user = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == ModelRole::User)
            .expect("last user");
        let MessagePart::Text { text } = &last_user.content[0] else {
            panic!("expected text part");
        };
        assert_eq!(text, "next");
    }

    #[test]
    fn generate_request_leaves_system_alone_without_workbench_paragraph() {
        let messages = vec![
            serde_json::json!({ "role": "system", "content": "You are Kivio." }),
            serde_json::json!({ "role": "user", "content": "hello" }),
        ];
        let request = generate_request_from_openai_messages(
            "m",
            messages,
            None,
            Default::default(),
            "t",
            Default::default(),
        );
        assert_eq!(request.system, "You are Kivio.");
        let MessagePart::Text { text } = &request.messages[0].content[0] else {
            panic!("expected text part");
        };
        assert_eq!(text, "hello");
    }
}
