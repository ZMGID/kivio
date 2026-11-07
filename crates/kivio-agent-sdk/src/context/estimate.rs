use crate::agent::history::{RuntimeBody, RuntimeMessage};
use crate::model::{MessagePart, ModelMessage};
use serde_json::Value;
pub fn estimate_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }
    ascii.div_ceil(4) + non_ascii
}

pub const IMAGE_PART_TYPES: [&str; 3] = ["image_url", "input_image", "image"];
pub const TEXT_PART_TYPES: [&str; 2] = ["text", "input_text"];

/// Estimates readable reasoning text or its summary, excluding encrypted data.
pub fn estimate_reasoning_item_tokens(item: &Value) -> usize {
    let text_tokens = |key: &str| -> Option<usize> {
        let parts = item.get(key)?.as_array()?;
        let texts: Vec<&str> = parts
            .iter()
            .filter_map(|p| p.get("text")?.as_str())
            .filter(|text| !text.is_empty())
            .collect();
        (!texts.is_empty()).then(|| texts.into_iter().map(estimate_tokens).sum())
    };
    text_tokens("content")
        .or_else(|| text_tokens("summary"))
        .unwrap_or(0)
}

/// Estimates reasoning once when native and plain-text forms coexist.
pub fn estimate_message_reasoning_tokens(message: &Value) -> usize {
    let native: usize = message
        .get("reasoning_items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|entry| estimate_reasoning_item_tokens(entry.get("item").unwrap_or(entry)))
                .sum()
        })
        .unwrap_or(0);
    if native > 0 {
        return native;
    }
    message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(Value::as_str)
        .map(estimate_tokens)
        .unwrap_or(0)
}

/// Estimates JSON text and object fields recursively, excluding image and `video_url` parts.
pub fn estimate_value_tokens(value: &Value) -> usize {
    match value {
        Value::String(text) => estimate_tokens(text),
        Value::Array(items) => items.iter().map(estimate_value_tokens).sum(),
        Value::Object(map) => {
            if map.contains_key("reasoning_items") || map.contains_key("anthropic_content") {
                return map
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "reasoning_items"
                                | "reasoning_content"
                                | "reasoning"
                                | "anthropic_content"
                        )
                    })
                    .map(|(key, value)| estimate_tokens(key) + estimate_value_tokens(value))
                    .sum::<usize>()
                    + estimate_message_reasoning_tokens(value);
            }
            if let Some(kind) = map.get("type").and_then(Value::as_str) {
                if kind == "reasoning"
                    && (map.contains_key("content") || map.contains_key("summary"))
                {
                    return estimate_reasoning_item_tokens(value);
                }
                if kind == "video_url" {
                    return 0;
                }
                if IMAGE_PART_TYPES.contains(&kind) {
                    return 0;
                }
                if TEXT_PART_TYPES.contains(&kind) {
                    return map.get("text").map(estimate_value_tokens).unwrap_or(0);
                }
            }
            map.iter()
                .map(|(key, value)| estimate_tokens(key) + estimate_value_tokens(value))
                .sum()
        }
        _ => estimate_tokens(&value.to_string()),
    }
}

pub fn estimate_messages_tokens(messages: &[Value]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Estimates the content, tool calls, and reasoning of one message.
pub fn estimate_message_tokens(message: &Value) -> usize {
    let tool_calls = message
        .get("tool_calls")
        .map(|calls| estimate_tokens(&calls.to_string()))
        .unwrap_or(0);
    let reasoning = estimate_message_reasoning_tokens(message);
    let content = match message.get("content") {
        Some(Value::String(text)) => estimate_tokens(text),
        Some(other) => estimate_value_tokens(other),
        None => 0,
    };
    content + tool_calls + reasoning + 4
}

pub fn estimate_model_messages_tokens(messages: &[ModelMessage]) -> usize {
    messages.iter().map(estimate_model_message_tokens).sum()
}

pub fn estimate_runtime_message_tokens(message: &RuntimeMessage) -> usize {
    match &message.body {
        RuntimeBody::System(text) => estimate_tokens(text) + 4,
        RuntimeBody::Model(message) => estimate_model_message_tokens(message),
    }
}

fn estimate_model_message_tokens(message: &ModelMessage) -> usize {
    let native_reasoning: usize = message
        .content
        .iter()
        .filter_map(|part| match part {
            MessagePart::ReasoningItem { item, .. } => Some(estimate_reasoning_item_tokens(item)),
            _ => None,
        })
        .sum();
    let parts: usize = message
        .content
        .iter()
        .map(|part| match part {
            MessagePart::Text { text } => estimate_tokens(text),
            MessagePart::AnthropicThinking { thinking, .. } => estimate_tokens(thinking),
            MessagePart::Reasoning { text } => {
                if native_reasoning == 0 {
                    estimate_tokens(text)
                } else {
                    0
                }
            }
            MessagePart::ToolCall {
                id,
                name,
                arguments_raw,
                signature,
                ..
            } => {
                let envelope =
                    r#"{"id":"","type":"function","function":{"name":"","arguments":""}}"#;
                estimate_tokens(envelope)
                    + estimate_json_text_tokens(id)
                    + estimate_json_text_tokens(name)
                    + estimate_json_text_tokens(arguments_raw)
                    + signature.as_ref().map_or(0, |signature| {
                        estimate_tokens(r#","thought_signature":" "#)
                            + estimate_json_text_tokens(signature.value())
                    })
            }
            MessagePart::ToolResult { content, .. } => estimate_tokens(content),
            MessagePart::AnthropicNativeBlock { block, .. } => block
                .get("text")
                .and_then(Value::as_str)
                .map(estimate_tokens)
                .unwrap_or(0),
            // 图片部件记 0（与 estimate_value_tokens 同口径，不把 base64 算进 token）。
            // reasoning item 同理：encrypted_content 是密文 base64，按字符估算会
            // 数倍虚高。
            MessagePart::Image { .. }
            | MessagePart::Video { .. }
            | MessagePart::ImageUrl { .. }
            | MessagePart::ReasoningItem { .. }
            | MessagePart::AnthropicRedactedThinking { .. } => 0,
        })
        .sum();
    parts + native_reasoning + 4
}

fn estimate_json_text_tokens(text: &str) -> usize {
    let (bytes, non_ascii) = text
        .chars()
        .filter(|ch| !ch.is_ascii())
        .fold((0, 0), |(bytes, chars), ch| {
            (bytes + ch.len_utf8(), chars + 1)
        });
    (json_escaped_bytes(text) - bytes).div_ceil(4) + non_ascii
}

pub(super) fn json_escaped_bytes(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}
