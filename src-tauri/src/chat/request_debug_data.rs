//! Request debug data and redaction shared by native and legacy hosts.
use crate::chat::model::{GenerateOutput, GenerateRequest, ModelUsage, PendingToolCall};
use crate::settings::ModelProvider;
use crate::usage::{chat_usage_source_for_label, operation_from_label};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Cap on the recorded response text (chars) so a long answer can't blow up memory.
const MAX_RESPONSE_TEXT_CHARS: usize = 4000;

/// Header keys (lower-cased) whose values carry credentials and must be masked.
const SENSITIVE_HEADER_KEYS: &[&str] = &[
    "authorization",
    "x-api-key",
    "api-key",
    "x-goog-api-key",
    "x-api-token",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDebugRecord {
    pub id: String,
    pub created_at: i64,
    pub duration_ms: u64,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub api_format: String,
    pub operation: String,
    pub source: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    pub status: String,
    pub request: RequestDebugRequest,
    pub response: RequestDebugResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDebugRequest {
    pub url: String,
    /// Sanitized headers — credential values are already masked.
    pub headers: BTreeMap<String, String>,
    pub body: Value,
    pub stream: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDebugResponse {
    #[serde(default)]
    pub status_code: Option<u16>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    /// `null` when there were no tool calls, otherwise an array of call summaries.
    #[serde(default)]
    pub tool_calls: Value,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<ModelUsage>,
    /// 内置联网搜索解析结果（任务 07-23）：从响应里提取到的 queries + citations；
    /// `null` = 未解析到任何搜索痕迹（模型没搜 / 渠道没返回结构化字段）。
    #[serde(default)]
    pub web_search: Value,
    #[serde(default)]
    pub error: Option<String>,
}

impl RequestDebugResponse {
    /// Summarize a successful [`GenerateOutput`] (text truncated, tool calls + usage kept).
    pub fn from_output(output: &GenerateOutput, status_code: Option<u16>) -> Self {
        Self {
            status_code,
            text: truncate_opt(&output.text),
            reasoning: output.reasoning.clone(),
            tool_calls: tool_calls_to_value(&output.tool_calls),
            finish_reason: output.finish_reason.clone(),
            usage: output.usage.clone(),
            web_search: output
                .web_search
                .as_ref()
                .and_then(|ws| serde_json::to_value(ws).ok())
                .unwrap_or(Value::Null),
            error: None,
        }
    }

    /// Summarize a failed call from its error string + parsed HTTP status (if any).
    pub fn from_error(error: &str, status_code: Option<u16>) -> Self {
        Self {
            status_code,
            tool_calls: Value::Null,
            web_search: Value::Null,
            error: Some(error.to_string()),
            ..Default::default()
        }
    }
}

/// Inputs an adapter hands to [`build_debug_record`]. The adapter owns url/headers/body
/// (built from the SAME functions it uses to send, so the record can't drift) and the
/// response summary; this module derives operation/source and assembles the record.
pub struct DebugRecordArgs<'a> {
    pub provider: &'a ModelProvider,
    pub request: &'a GenerateRequest,
    pub label: &'a str,
    pub started_at: i64,
    pub duration_ms: u64,
    pub status: &'a str,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
    pub stream: bool,
    pub response: RequestDebugResponse,
}

/// Assemble a [`RequestDebugRecord`] from adapter-provided request/response parts.
pub fn build_debug_record(mut args: DebugRecordArgs<'_>) -> RequestDebugRecord {
    redact_video_payloads(&mut args.body);
    let source = args
        .request
        .metadata
        .usage_source
        .clone()
        .unwrap_or_else(|| chat_usage_source_for_label(args.label));
    let operation = args
        .request
        .metadata
        .usage_operation
        .clone()
        .unwrap_or_else(|| operation_from_label(args.label));
    RequestDebugRecord {
        id: format!("dbg_{}", Uuid::new_v4()),
        created_at: args.started_at,
        duration_ms: args.duration_ms,
        provider_id: args.provider.id.clone(),
        provider_name: args.provider.name.clone(),
        model: args.request.model.clone(),
        api_format: args.provider.api_format.clone(),
        operation,
        source,
        conversation_id: args.request.metadata.conversation_id.clone(),
        message_id: args.request.metadata.message_id.clone(),
        status: args.status.to_string(),
        request: RequestDebugRequest {
            url: args.url,
            headers: args.headers,
            body: args.body,
            stream: args.stream,
        },
        response: args.response,
    }
}

fn redact_video_payloads(value: &mut Value) {
    match value {
        Value::String(s) if s.starts_with("data:video/") => {
            *s = "[video bytes omitted]".into();
        }
        Value::Array(items) => items.iter_mut().for_each(redact_video_payloads),
        Value::Object(map) => {
            if map
                .get("mimeType")
                .or_else(|| map.get("mime_type"))
                .and_then(Value::as_str)
                .is_some_and(|s| s.starts_with("video/"))
            {
                if let Some(data) = map.get_mut("data") {
                    *data = Value::String("[video bytes omitted]".into());
                }
            }
            map.values_mut().for_each(redact_video_payloads);
        }
        _ => {}
    }
}

/// Return a copy of `headers` with credential values masked. Non-sensitive headers pass
/// through unchanged. Comparison is case-insensitive on the header name.
pub fn sanitize_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .map(|(name, value)| {
            if SENSITIVE_HEADER_KEYS.contains(&name.to_ascii_lowercase().as_str()) {
                (name, mask_secret(&value))
            } else {
                (name, value)
            }
        })
        .collect()
}

/// Mask a credential to a short preview. Keeps an optional `Bearer ` scheme prefix and the
/// first 4 chars of the secret (matching the capture-proxy convention `sk-xxxx…`).
pub fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let (prefix, secret) = match trimmed.split_once(' ') {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("bearer") => {
            (format!("{scheme} "), rest)
        }
        _ => (String::new(), trimmed),
    };
    let preview: String = secret.chars().take(4).collect();
    format!("{prefix}{preview}…")
}

fn truncate_opt(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    Some(truncate_chars(text, MAX_RESPONSE_TEXT_CHARS))
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("…[truncated]");
    out
}

fn tool_calls_to_value(calls: &[PendingToolCall]) -> Value {
    if calls.is_empty() {
        return Value::Null;
    }
    Value::Array(
        calls
            .iter()
            .map(|call| {
                serde_json::json!({
                    "id": call.id,
                    "name": call.function_name,
                    "arguments": call.arguments,
                    "argumentsRaw": call.arguments_raw,
                    "parseError": call.arguments_parse_error,
                })
            })
            .collect(),
    )
}
