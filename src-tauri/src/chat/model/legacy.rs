use serde_json::Value;

use crate::mcp::ChatToolDefinition;

use super::types::{
    model_message_from_openai_message, openai_message_text_content,
    prepend_text_to_first_user_message, split_workbench_system_suffix, GenerateOptions,
    GenerateRequest, GenerateRequestContext, ModelMessage, ModelTool, RequestMetadata,
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
