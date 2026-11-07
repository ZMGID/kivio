//! Authoritative display measurement for one prepared model request.
//!
//! Budget anchors stay in `context_estimate` / `resolve_usage_anchor`. This module
//! only decides what the meter and category breakdown may show: the latest
//! reported main-request measurement for the current lifecycle, never a rebuilt prompt
//! and never an older report with a lower sequence.

use serde_json::Value;

use super::context_policy::estimate_runtime_message_tokens;
use crate::chat::agent::history::{RuntimeBody, RuntimeMessage};
use crate::chat::agent_preparation::{estimate_tokens, estimate_value_tokens, IMAGE_PART_TYPES};
use crate::chat::model::{MessagePart, ModelTool};
use crate::chat::types::{ContextRequestMeasurement, ContextUsageSegment};
use crate::settings::ModelProvider;

/// In-process measurement. Higher `seq` wins over anything already on disk.
#[derive(Debug, Clone, Default)]
pub(crate) struct LiveContextMeasurement {
    pub seq: u64,
    pub lifecycle_id: u64,
    pub request_id: String,
    pub message_id: String,
    pub provider_id: String,
    pub model: String,
    pub reported_tokens: Option<u64>,
    pub segments: Vec<ContextUsageSegment>,
    /// Last coherent report while the bound request is waiting for its own usage.
    /// Pending categories must never be paired with the previous request's total.
    pub last_reported: Option<ContextRequestMeasurement>,
    /// Complete cache pairs produced by the in-flight reply, not the conversation total.
    pub run_cache: Option<(u64, u64)>,
    /// Categories for this request were already attached to one usage report.
    pub categories_published: bool,
    /// This bind has received its own provider report.
    pub report_received: bool,
}

impl LiveContextMeasurement {
    pub(crate) fn stored(&self) -> ContextRequestMeasurement {
        if self.reported_tokens.is_none() {
            if let Some(previous) = self.last_reported.as_ref().filter(|previous| {
                previous.lifecycle_id == self.lifecycle_id
                    && previous.provider_id == self.provider_id
                    && previous.model == self.model
            }) {
                return ContextRequestMeasurement {
                    seq: self.seq,
                    ..previous.clone()
                };
            }
        }
        ContextRequestMeasurement {
            seq: self.seq,
            lifecycle_id: self.lifecycle_id,
            request_id: self.request_id.clone(),
            provider_id: self.provider_id.clone(),
            model: self.model.clone(),
            reported_tokens: self.reported_tokens,
            segments: self.segments.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DisplayMeasurement {
    pub reported_tokens: Option<u64>,
    pub segments: Vec<ContextUsageSegment>,
    pub seq: u64,
    pub lifecycle_id: u64,
    pub token_count_source: Option<&'static str>,
    pub persist: Option<ContextRequestMeasurement>,
}

/// UTF-16 length of text the model actually sees. Image and video payloads are
/// skipped in place so base64 is neither copied nor counted.
pub(crate) fn utf16_content_chars(value: &Value) -> usize {
    match value {
        Value::String(text) => utf16_text_chars(text),
        Value::Array(items) => items.iter().map(utf16_content_chars).sum(),
        Value::Object(map) => {
            if map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "video_url" || IMAGE_PART_TYPES.contains(&kind))
            {
                return 0;
            }
            map.iter()
                .map(|(key, value)| key.encode_utf16().count() + utf16_content_chars(value))
                .sum()
        }
        _ => 0,
    }
}

fn utf16_text_chars(text: &str) -> usize {
    let trimmed = text.trim_start();
    if trimmed.starts_with("data:") || trimmed.starts_with("kivio-attachment://") {
        0
    } else {
        text.encode_utf16().count()
    }
}

fn push_category(
    segments: &mut Vec<ContextUsageSegment>,
    id: &str,
    label: &str,
    tokens: usize,
    chars: usize,
) {
    if tokens == 0 && chars == 0 {
        return;
    }
    segments.push(ContextUsageSegment {
        id: id.to_string(),
        label: label.to_string(),
        estimated_tokens: tokens,
        chars,
        color: crate::chat::agent_preparation::context_segment_color(id).map(str::to_string),
    });
}

/// Three display groups measured from the request that is about to be sent.
pub(crate) fn measure_prepared_request(
    messages: &[RuntimeMessage],
    tools: &[ModelTool],
    provider: Option<&ModelProvider>,
    model: &str,
) -> Vec<ContextUsageSegment> {
    measure_request(
        messages.iter().map(|message| {
            let tokens = estimate_runtime_message_tokens(message)
                + super::context_estimate::runtime_message_image_tokens(message, provider, model);
            let chars = match &message.body {
                RuntimeBody::System(text) => utf16_text_chars(text),
                RuntimeBody::Model(message) => message
                    .content
                    .iter()
                    .map(|part| match part {
                        MessagePart::Text { text }
                        | MessagePart::Reasoning { text }
                        | MessagePart::AnthropicThinking { thinking: text, .. }
                        | MessagePart::ToolResult { content: text, .. } => utf16_text_chars(text),
                        MessagePart::ToolCall {
                            name,
                            arguments_raw,
                            ..
                        } => utf16_text_chars(name) + utf16_text_chars(arguments_raw),
                        MessagePart::AnthropicNativeBlock { block, .. } => {
                            utf16_content_chars(block)
                        }
                        _ => 0,
                    })
                    .sum(),
            };
            (
                matches!(message.body, RuntimeBody::System(_)),
                tokens,
                chars,
            )
        }),
        tools,
    )
}

fn measure_request(
    messages: impl Iterator<Item = (bool, usize, usize)>,
    tools: &[ModelTool],
) -> Vec<ContextUsageSegment> {
    let mut system_tokens = 0usize;
    let mut system_chars = 0usize;
    let mut conversation_tokens = 0usize;
    let mut conversation_chars = 0usize;
    for (system, tokens, chars) in messages {
        if system {
            system_tokens += tokens;
            system_chars += chars;
        } else {
            conversation_tokens += tokens;
            conversation_chars += chars;
        }
    }
    let mut tool_tokens = 0usize;
    let mut tool_chars = 0usize;
    for tool in tools {
        let name = &tool.name;
        tool_tokens += estimate_tokens(&name)
            + estimate_tokens(&tool.description)
            + estimate_value_tokens(&tool.input_schema);
        tool_chars += name.encode_utf16().count()
            + tool.description.encode_utf16().count()
            + utf16_content_chars(&tool.input_schema);
    }
    let mut segments = Vec::new();
    push_category(
        &mut segments,
        "system_prompt",
        "System prompt",
        system_tokens,
        system_chars,
    );
    push_category(&mut segments, "tools", "Tools", tool_tokens, tool_chars);
    push_category(
        &mut segments,
        "conversation",
        "Conversation",
        conversation_tokens,
        conversation_chars,
    );
    segments
}

fn legacy_display(conversation: &crate::chat::types::Conversation) -> DisplayMeasurement {
    let state = &conversation.context_state;
    let reported = (state.token_count_source.as_deref() == Some("provider_context_reported"))
        .then_some(state.reported_context_tokens)
        .flatten();
    DisplayMeasurement {
        reported_tokens: reported,
        segments: if reported.is_some() {
            state.segments.clone()
        } else {
            Vec::new()
        },
        seq: state.measurement_seq,
        lifecycle_id: state.lifecycle_id,
        token_count_source: reported.map(|_| "provider_context_reported"),
        persist: None,
    }
}

/// Last reported request for this lifecycle, retained while a new request waits.
/// Without any report the meter stays unknown; budget anchors are not promoted.
pub(crate) fn resolve_display(
    conversation: &crate::chat::types::Conversation,
    live: Option<&LiveContextMeasurement>,
) -> DisplayMeasurement {
    let disk = conversation.context_state.request_measurement.as_ref();
    // A seq floor with no request, tokens, or segments must not hide a disk report.
    // A newer lifecycle still wins: that is an invalidation, even when it is empty.
    let live = live.filter(|live| {
        live.lifecycle_id > conversation.context_state.lifecycle_id
            || !live.request_id.is_empty()
            || live.reported_tokens.is_some()
            || !live.segments.is_empty()
    });
    let live_wins = live.is_some_and(|live| {
        disk.is_none_or(|disk| live.lifecycle_id > disk.lifecycle_id || live.seq >= disk.seq)
    });
    let chosen_seq = if live_wins {
        live.map(|live| live.seq)
    } else {
        disk.map(|disk| disk.seq)
    };
    let Some(seq) = chosen_seq else {
        return legacy_display(conversation);
    };
    let current_lifecycle = if live_wins {
        live.map(|live| live.lifecycle_id)
            .unwrap_or(conversation.context_state.lifecycle_id)
    } else {
        disk.map(|disk| disk.lifecycle_id)
            .unwrap_or(conversation.context_state.lifecycle_id)
    }
    .max(conversation.context_state.lifecycle_id);

    let (provider_id, model, lifecycle_id, reported, segments, request_id) = if live_wins {
        let live = live.expect("live wins only when present").stored();
        (
            live.provider_id.clone(),
            live.model.clone(),
            live.lifecycle_id,
            live.reported_tokens,
            live.segments.clone(),
            live.request_id.clone(),
        )
    } else {
        let disk = disk.expect("disk wins only when present");
        (
            disk.provider_id.clone(),
            disk.model.clone(),
            disk.lifecycle_id,
            disk.reported_tokens,
            disk.segments.clone(),
            disk.request_id.clone(),
        )
    };
    let identity_ok = lifecycle_id == current_lifecycle
        && provider_id == conversation.provider_id
        && model == conversation.model;
    let persist = Some(ContextRequestMeasurement {
        seq,
        lifecycle_id: current_lifecycle,
        request_id,
        provider_id,
        model,
        reported_tokens: if identity_ok { reported } else { None },
        segments: if identity_ok {
            segments.clone()
        } else {
            Vec::new()
        },
    });
    if !identity_ok {
        return DisplayMeasurement {
            reported_tokens: None,
            segments: Vec::new(),
            seq,
            lifecycle_id: current_lifecycle,
            token_count_source: None,
            persist,
        };
    }
    DisplayMeasurement {
        reported_tokens: reported,
        segments,
        seq,
        lifecycle_id: current_lifecycle,
        token_count_source: reported.map(|_| "provider_context_reported"),
        persist,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_request_skips_image_payloads_and_keeps_three_groups() {
        let payload = format!("data:image/png;base64,{}", "A".repeat(4_000));
        let messages = vec![
            serde_json::json!({"role": "system", "content": "rules"}),
            serde_json::json!({"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": payload}},
                {"type": "text", "text": "hi"}
            ]}),
        ];
        let tool = ModelTool {
            name: "read".into(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        };
        let segments = measure_prepared_request(
            &crate::chat::agent::history::import_history(messages),
            &[tool],
            None,
            "gpt-4o",
        );
        let ids: Vec<_> = segments.iter().map(|segment| segment.id.as_str()).collect();
        assert_eq!(ids, vec!["system_prompt", "tools", "conversation"]);
        let conversation = segments
            .iter()
            .find(|segment| segment.id == "conversation")
            .unwrap();
        assert!(
            conversation.chars < 100,
            "image payload leaked into chars: {}",
            conversation.chars
        );
        assert!(conversation.chars >= "hi".encode_utf16().count());
    }
}
