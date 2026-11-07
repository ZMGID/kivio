use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelErrorKind {
    InvalidImage,
    ContentModeration,
    ContextOverflow,
    ToolsUnsupported,
    InvalidRequest,
    Auth,
    RateLimited,
    Timeout,
    Transport,
    ProviderUnavailable,
    EmptyResponse,
    TruncationRecoveryFailed,
    Cancelled,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub message: String,
    pub kind: ModelErrorKind,
    pub status: Option<u16>,
    pub provider_code: Option<String>,
    pub retry_after: Option<std::time::Duration>,
    retryable_stream: bool,
}

impl ModelError {
    pub fn new(message: impl Into<String>) -> Self {
        let message = message.into();
        let status = extract_status_code(&message);
        let kind = classify(status, &message);
        Self {
            status,
            ..Self::with_kind(message, kind)
        }
    }

    pub fn with_kind(message: impl Into<String>, kind: ModelErrorKind) -> Self {
        Self {
            message: message.into(),
            kind,
            status: None,
            provider_code: None,
            retry_after: None,
            retryable_stream: false,
        }
    }

    pub fn from_provider_http(status: u16, body: &str, diagnostic: String) -> Self {
        let provider_code = serde_json::from_str::<Value>(body)
            .ok()
            .as_ref()
            .and_then(|value| provider_error_code(value.get("error").unwrap_or(value)));
        Self {
            kind: classify_provider(Some(status), body, provider_code.as_deref()),
            message: diagnostic,
            status: Some(status),
            provider_code,
            retry_after: None,
            retryable_stream: false,
        }
    }

    pub fn from_provider_payload(code: Option<&str>, diagnostic: String) -> Self {
        let status = extract_status_code(&diagnostic);
        Self {
            kind: classify_provider(status, &diagnostic, code),
            message: diagnostic,
            status,
            provider_code: code.map(str::to_string),
            retry_after: None,
            retryable_stream: false,
        }
    }

    pub fn from_provider_stream(code: Option<&str>, diagnostic: String) -> Self {
        let mut error = Self::from_provider_payload(code, diagnostic);
        let lower = provider_signal(&error.message, code).to_ascii_lowercase();
        error.retryable_stream = !is_permanent_provider_error(&lower)
            && (is_missing_stream_terminal_error(&lower)
                || [
                    "overloaded_error",
                    "server_error",
                    "rate_limit_error",
                    "rate_limit_exceeded",
                    "service_unavailable",
                    "internal_server_error",
                    "service temporarily unavailable",
                    "service is temporarily unavailable",
                    "service is unavailable",
                    "server_busy",
                    "servers are currently busy",
                    "model is at capacity",
                    "currently experiencing high demand",
                    "unable to forward upstream",
                    "upstream_request_failed",
                    "upstream request failed",
                    "upstream_stream_read_error",
                    "stream was interrupted",
                    "stream disconnected before completion",
                    "resource_exhausted",
                ]
                .iter()
                .any(|code| lower.contains(code)));
        error
    }

    pub fn stream_ended_early(label: &str) -> Self {
        let mut error = Self::with_kind(
            format!("{label} 流式响应读取中断：stream ended without terminal event"),
            ModelErrorKind::Transport,
        );
        error.retryable_stream = true;
        error
    }

    pub fn is_retryable_stream_error(&self) -> bool {
        self.retryable_stream
    }

    pub fn is_cancelled(&self) -> bool {
        self.kind == ModelErrorKind::Cancelled
    }
}

pub fn stream_read_error(label: &str, err: &reqwest::Error) -> ModelError {
    let reason = if err.is_timeout() {
        "the provider stream timed out"
    } else if err.is_connect() {
        "the provider connection was interrupted"
    } else if err.is_decode() {
        "the provider sent an incomplete or invalid encoded stream chunk"
    } else {
        "the provider stream ended unexpectedly"
    };
    let mut error = ModelError::with_kind(
        format!(
            "{label} 流式响应读取中断：{reason}。这通常是临时的网络、代理或模型服务流式断包问题，请重试。"
        ),
        if err.is_timeout() { ModelErrorKind::Timeout } else { ModelErrorKind::Transport },
    );
    error.retryable_stream = true;
    error
}

pub fn provider_error_code(error: &Value) -> Option<String> {
    ["code", "type", "status"]
        .into_iter()
        .find_map(|key| error[key].as_str().map(str::to_string))
        .or_else(|| error["code"].as_u64().map(|code| code.to_string()))
}

fn provider_signal<'a>(message: &'a str, code: Option<&str>) -> std::borrow::Cow<'a, str> {
    match code {
        Some(code) => format!("{message} {code}").into(),
        None => message.into(),
    }
}

fn classify_provider(status: Option<u16>, body: &str, code: Option<&str>) -> ModelErrorKind {
    let kind = classify(status, &provider_signal(body, code));
    if kind != ModelErrorKind::Other {
        return kind;
    }
    match code.map(str::to_ascii_lowercase).as_deref() {
        Some("rate_limit_error" | "rate_limit_exceeded" | "resource_exhausted") => {
            ModelErrorKind::RateLimited
        }
        Some(
            "overloaded_error" | "server_error" | "service_unavailable" | "internal_server_error",
        ) => ModelErrorKind::ProviderUnavailable,
        Some("upstream_stream_read_error") => ModelErrorKind::Transport,
        _ => ModelErrorKind::Other,
    }
}

fn classify(status: Option<u16>, message: &str) -> ModelErrorKind {
    let trimmed = message.trim();
    if status.is_none()
        && (trimmed.is_empty() || trimmed.ends_with(" returned an empty assistant response"))
    {
        return ModelErrorKind::EmptyResponse;
    }
    if trimmed.starts_with("Truncated response recovery failed") {
        return ModelErrorKind::TruncationRecoveryFailed;
    }
    let lower = trimmed.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if is_invalid_image_error(trimmed) {
        return ModelErrorKind::InvalidImage;
    }
    if has(&[
        "content exists risk",
        "content policy",
        "content_policy",
        "content filter",
        "moderation",
        "safety",
        "sensitive",
        "审核",
        "违规",
        "敏感",
    ]) {
        return ModelErrorKind::ContentModeration;
    }
    if is_context_overflow(&lower) {
        return ModelErrorKind::ContextOverflow;
    }
    if has(&[
        "authentication_error",
        "permission_denied",
        "permission_error",
        "invalid_api_key",
    ]) {
        return ModelErrorKind::Auth;
    }
    if tools_unsupported(status, &lower) {
        return ModelErrorKind::ToolsUnsupported;
    }
    if is_permanent_provider_error(trimmed) {
        return ModelErrorKind::InvalidRequest;
    }
    match status {
        Some(401 | 403) => ModelErrorKind::Auth,
        Some(429) => ModelErrorKind::RateLimited,
        Some(400..=499) => ModelErrorKind::InvalidRequest,
        Some(500..=599) => ModelErrorKind::ProviderUnavailable,
        None if has(&["timed out", "timeout", "请求超时"]) => ModelErrorKind::Timeout,
        None if has(TRANSPORT_PATTERNS) => ModelErrorKind::Transport,
        _ => ModelErrorKind::Other,
    }
}

fn is_invalid_image_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["image", "图片", "base64", "mime", "media type"]
        .iter()
        .any(|word| lower.contains(word))
        && [
            "invalid",
            "decode",
            "corrupt",
            "format",
            "unsupported",
            "too large",
            "size",
            "dimension",
            "exceeds",
            "损坏",
            "格式",
            "超过",
            "超出",
        ]
        .iter()
        .any(|word| lower.contains(word))
}

/// Provider errors that require changing input, credentials or account state.
/// Shared by HTTP rate-limit handling and errors delivered inside a successful SSE response.
pub fn is_permanent_provider_error(message: &str) -> bool {
    if is_invalid_image_error(message) {
        return true;
    }
    let lower = message.to_ascii_lowercase();
    [
        "insufficient_quota",
        "out of budget",
        "quota exceeded",
        "billing",
        "available balance",
        "monthly usage limit reached",
        "gousagelimiterror",
        "freeusagelimiterror",
        "subscription_sharing_usage_limit_exceeded",
        "subscription_sharing_workplace_individual_usage_limit_exceeded",
        "invalid_request_error",
        "invalid_argument",
        "authentication_error",
        "permission_denied",
        "permission_error",
        "invalid_api_key",
        "unsupported_format",
    ]
    .iter()
    .any(|code| lower.contains(code))
}

fn parse_leading_status_code(value: &str) -> Option<u16> {
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    if end == 0 {
        return None;
    }
    value[..end].parse().ok()
}

/// 从 HTTP 错误信息中提取状态码
/// 格式约定：`"{label} Error: {status} - {body}"`，
/// status 形如 `"429 Too Many Requests"`，第一段数字即可
/// 兼容少数防御性分支使用的 `"{label} HTTP {status}: {body}"`。
/// 网络错误（reqwest::Error 路径）格式为 `"{label} Error: <reqwest msg>"`，无前导数字 → 返回 None
pub fn extract_status_code(err_msg: &str) -> Option<u16> {
    if let Some(idx) = err_msg.find(" Error: ") {
        let rest = &err_msg[idx + " Error: ".len()..];
        if let Some(code) = parse_leading_status_code(rest) {
            return Some(code);
        }
    }

    if let Some(idx) = err_msg.find(" HTTP ") {
        let rest = &err_msg[idx + " HTTP ".len()..];
        return parse_leading_status_code(rest);
    }

    None
}

/// 上下文超长信号:覆盖 OpenAI / Anthropic / Gemini / 各代理 / DeepSeek 及国内供应商
/// 常见文案(对标 PI `ai/src/utils/overflow.ts` 的 `OVERFLOW_PATTERNS` 子集)。
/// 命中其一即认为是「上下文撑爆」类失败,应走压缩重试而非快速失败。
const OVERFLOW_PATTERNS: &[&str] = &[
    // OpenAI / 通用
    "maximum context",
    "context length",
    "context_length_exceeded",
    "context window",
    "too many tokens",
    "reduce the length",
    "reduce your prompt",
    "input is too long",
    "input too long",
    // Anthropic
    "prompt is too long",
    "prompt too long",
    "prompt exceeds max length",
    "maximum prompt length",
    "maximum allowed input length",
    "range of input length should be",
    "exceeds the available context size",
    "request too large",
    "request_too_large",
    // Gemini / 其它代理
    "exceeds the context",
    "token limit",
    "tokens limit",
    "exceed the context",
    "exceed context",
    // 国内供应商 / 中文文案
    "上下文长度",
    "上下文过长",
    "输入过长",
    "tokens 超",
];

/// 排除项:这些文案虽含「limit/token」等词,实为限流/配额而非上下文超长,
/// 命中则**不**判为 overflow(对标 PI `NON_OVERFLOW_PATTERNS`,防止把限流误判成 overflow)。
const NON_OVERFLOW_PATTERNS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "too many requests",
    "requests per",
    "quota",
    "insufficient",
    "billing",
    "usage limit",
    "available balance",
];

/// 传输层没答完的信号：超时 / 连接断 / SSE 断包。这些**没有** HTTP 状态码，
/// 只在 `extract_status_code` 返回 None 时才判定，免得把带状态码的 5xx 抢走。
const TRANSPORT_PATTERNS: &[&str] = &[
    "timed out",
    "timeout",
    "error sending request",
    "connection reset",
    "connection closed",
    "connection refused",
    "incomplete or invalid encoded stream chunk",
    "dns error",
    "流式响应读取中断",
    "请求超时",
    "without terminal",
    "missing terminal",
    "without finish_reason",
    "stream ended without",
];

/// 判断错误文本是否为「上下文超长」(overflow)。先排除限流/配额误判,再匹配 overflow 文案。
fn is_context_overflow(lower: &str) -> bool {
    if NON_OVERFLOW_PATTERNS.iter().any(|n| lower.contains(n)) {
        return false;
    }
    OVERFLOW_PATTERNS.iter().any(|n| lower.contains(n))
        || (lower.contains("input token count") && lower.contains("exceeds the maximum"))
        || (lower.contains("prompt has") && lower.contains("but the configured context size is"))
}

#[cfg(test)]
pub fn is_tools_unsupported_error(err: &str) -> bool {
    tools_unsupported(extract_status_code(err), &err.to_ascii_lowercase())
}

fn tools_unsupported(status: Option<u16>, lower: &str) -> bool {
    if !matches!(status, Some(400 | 404 | 422 | 501)) {
        return false;
    }
    // The stage prefix itself contains "tools". A 400 mentioning malformed
    // history/schema/reasoning is not a capability rejection.
    [
        "tools",
        "tool_choice",
        "function calling",
        "function_call",
        "function call",
    ]
    .iter()
    .any(|feature| {
        lower.contains(&format!("{feature} not supported"))
            || lower.contains(&format!("{feature} is not supported"))
            || lower.contains(&format!("{feature} are not supported"))
            || lower.contains(&format!("does not support {feature}"))
            || lower.contains(&format!("unsupported {feature}"))
    }) || (lower.contains("tools[") && lower.contains("unknown variant `function`"))
}

/// Recognize relay errors that explicitly say no completion marker arrived.
/// Partial text alone is not proof of completion; the stream owner may retry.
pub fn is_missing_stream_terminal_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("without terminal")
        || lower.contains("missing terminal")
        || lower.contains("without finish_reason")
        || lower.contains("stream closed before response.completed")
        || (lower.contains("stream ended")
            && (lower.contains("completed") || lower.contains("terminal event")))
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ModelError {}

impl From<String> for ModelError {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ModelError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_and_sse_retryability_are_independent() {
        let unavailable = ModelError::from_provider_http(
            503,
            r#"{"error":{"code":"overloaded_error"}}"#,
            "request exhausted".into(),
        );
        assert_eq!(unavailable.kind, ModelErrorKind::ProviderUnavailable);
        assert_eq!(unavailable.status, Some(503));
        assert_eq!(
            unavailable.provider_code.as_deref(),
            Some("overloaded_error")
        );
        assert!(!unavailable.is_retryable_stream_error());

        let stream = ModelError::from_provider_stream(Some("overloaded_error"), "try again".into());
        assert_eq!(stream.kind, ModelErrorKind::ProviderUnavailable);
        assert!(stream.is_retryable_stream_error());
        let invalid = ModelError::from_provider_stream(
            Some("invalid_request_error"),
            "server_error: invalid image data".into(),
        );
        assert_eq!(invalid.kind, ModelErrorKind::InvalidImage);
        assert!(!invalid.is_retryable_stream_error());

        let unknown =
            ModelError::from_provider_stream(Some("NEW_CODE"), "unknown diagnostic".into());
        assert_eq!(unknown.kind, ModelErrorKind::Other);
        assert_eq!(unknown.provider_code.as_deref(), Some("NEW_CODE"));
        assert_eq!(unknown.to_string(), "unknown diagnostic");
        assert!(!unknown.is_retryable_stream_error());
    }

    #[test]
    fn provider_recovery_preserves_classification_precedence() {
        let classify_http =
            |status, body| ModelError::from_provider_http(status, body, body.into()).kind;
        assert_eq!(
            classify_http(400, "invalid image data exceeds the context window"),
            ModelErrorKind::InvalidImage
        );
        assert_eq!(
            classify_http(400, "safety: prompt too long"),
            ModelErrorKind::ContentModeration
        );
        assert_eq!(
            classify_http(400, "invalid_request_error: prompt too long"),
            ModelErrorKind::ContextOverflow
        );
        assert_eq!(
            classify_http(400, "invalid_request_error: tools not supported"),
            ModelErrorKind::ToolsUnsupported
        );
        assert_eq!(
            classify_http(502, "tools not supported"),
            ModelErrorKind::ProviderUnavailable
        );
        assert_eq!(
            ModelError::new("Truncated response recovery failed: prompt too long").kind,
            ModelErrorKind::TruncationRecoveryFailed
        );
    }
}
