//! Shared usage decoding and classification, independent of desktop host.
use crate::{
    chat::model::{GenerateRequest, ModelUsage},
    settings::ModelProvider,
};
use serde_json::Value;

pub struct UsageRecordInput<'a> {
    pub provider: &'a ModelProvider,
    pub model: &'a str,
    pub source: &'a str,
    pub operation: &'a str,
    pub status: &'a str,
    pub status_code: Option<u16>,
    pub usage: Option<ModelUsage>,
    pub usage_source: &'a str,
    pub started_at: i64,
    pub duration_ms: u64,
    pub first_token_ms: Option<u64>,
    pub reasoning_effort: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub error_kind: Option<String>,
}

/// Explicit effort sent for this request. `None` means the provider/model default was used.
pub fn reasoning_effort_for_request(request: &GenerateRequest) -> Option<String> {
    if request.options.thinking_enabled {
        request.options.thinking_level.clone()
    } else {
        Some("none".to_string())
    }
}

pub fn model_usage_from_openai_value(value: &Value) -> Option<ModelUsage> {
    let usage = value.get("usage")?;
    let prompt_details = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"));
    let completion_details = usage
        .get("completion_tokens_details")
        .or_else(|| usage.get("output_tokens_details"));
    Some(ModelUsage {
        input_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("input_tokens").and_then(Value::as_u64)),
        output_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("output_tokens").and_then(Value::as_u64)),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
        cached_input_tokens: prompt_details
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64),
        cache_creation_input_tokens: prompt_details
            .and_then(|details| details.get("cache_creation_tokens"))
            .and_then(Value::as_u64),
        reasoning_tokens: completion_details
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_u64),
        // 内置 provider 路径：窗口来自 model_metadata，不由响应携带。
        context_window_tokens: None,
        context_tokens: None,
    })
}

pub fn model_usage_from_anthropic_value(value: &Value) -> Option<ModelUsage> {
    let usage = value.get("usage")?;
    let input = usage.get("input_tokens").and_then(Value::as_u64);
    let output = usage.get("output_tokens").and_then(Value::as_u64);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64);
    let cache_read = usage.get("cache_read_input_tokens").and_then(Value::as_u64);
    Some(ModelUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input.zip(output).map(|(a, b)| a.saturating_add(b)),
        cached_input_tokens: cache_read,
        cache_creation_input_tokens: cache_creation,
        reasoning_tokens: None,
        // 内置 provider 路径：窗口来自 model_metadata，不由响应携带。
        context_window_tokens: None,
        context_tokens: None,
    })
}

pub fn model_usage_from_stream_value(value: &Value) -> Option<ModelUsage> {
    model_usage_from_openai_value(value).or_else(|| model_usage_from_anthropic_value(value))
}

pub fn chat_usage_source_for_label(label: &str) -> String {
    let lower = label.to_ascii_lowercase();
    if lower.contains("title") {
        "chat_title_summary".to_string()
    } else if lower.contains("compression") {
        "chat_compression".to_string()
    } else if lower.contains("auxiliary vision") {
        "chat_aux_vision".to_string()
    } else if lower.contains("auxiliary video") {
        "chat_aux_video".to_string()
    } else if lower.contains("image generation") {
        "chat_image_generation".to_string()
    } else if lower.contains("prompt optimize") {
        "chat_prompt_optimize".to_string()
    } else {
        "chat".to_string()
    }
}

pub fn operation_from_label(label: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in label.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_sep = false;
        } else if !last_was_sep && !out.is_empty() {
            out.push('_');
            last_was_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "model_call".to_string()
    } else {
        out
    }
}

pub fn error_kind_from_message(message: &str) -> String {
    if let Some(code) = crate::api::extract_status_code(message) {
        return format!("http_{code}");
    }
    let lower = message.to_ascii_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".to_string()
    } else if lower.contains("cancelled") || lower.contains("canceled") {
        "cancelled".to_string()
    } else if lower.contains("parse") || lower.contains("json") {
        "parse_error".to_string()
    } else if lower.contains("read body") || lower.contains("stream") {
        "stream_error".to_string()
    } else {
        "request_error".to_string()
    }
}

/// 失败记录的 usage status：错误文案命中取消语义时记 `"cancelled"`，否则 `"error"`。
/// 四个模型适配器的 `record_usage_failure` 签名只有错误字符串（不带 `ModelError`），
/// 故在共享层按与 `error_kind_from_message` 同一口径判定，避免改四处签名。
pub fn failure_status_from_message(message: &str) -> &'static str {
    if error_kind_from_message(message) == "cancelled" {
        "cancelled"
    } else {
        "error"
    }
}
