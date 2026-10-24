//! Shared model catalog decoding and connection probe wire payloads.
use std::collections::HashSet;
use crate::settings::ProviderApiFormat;
/// 从 `/models` JSON 抽出模型 id。
/// OpenAI / Anthropic / Responses：`data[].id`（或 `data[]` 字符串）。
/// Gemini 原生 ListModels：`models[].name`，形如 `models/gemini-2.5-flash`。
pub fn parse_model_list_ids(value: &serde_json::Value) -> Result<Vec<String>, String> {
    if let Some(msg) = models_api_error_message(value) {
        return Err(format!("Models API error: {msg}"));
    }

    let items = value
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| value.get("models").and_then(|v| v.as_array()))
        .or_else(|| value.as_array())
        .ok_or_else(|| {
            "Invalid response format: expected a 'data' or 'models' array".to_string()
        })?;

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        if !model_item_is_listable(item) {
            continue;
        }
        let Some(id) = model_item_id(item) else {
            continue;
        };
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    Ok(out)
}

fn models_api_error_message(value: &serde_json::Value) -> Option<String> {
    let err = value.get("error")?;
    err.as_str()
        .map(str::to_string)
        .or_else(|| {
            err.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .filter(|msg| !msg.trim().is_empty())
}

fn model_item_id(item: &serde_json::Value) -> Option<String> {
    if let Some(s) = item.as_str() {
        return normalize_listed_model_id(s);
    }
    item.get("id")
        .and_then(|v| v.as_str())
        .or_else(|| item.get("name").and_then(|v| v.as_str()))
        .and_then(normalize_listed_model_id)
}

fn normalize_listed_model_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Gemini ListModels 的 `name` 是 `models/gemini-2.5-flash`；对话 URL 会再拼一层
    // `/models/`，这里去掉前缀，和 generateContent 路径去重保持同一口径。
    let stripped = trimmed.strip_prefix("models/").unwrap_or(trimmed).trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// Gemini 原生条目带 `supportedGenerationMethods` 时，只收能 generate 的模型
///（embedding-only 进聊天选择器没有意义）。中转站 / OpenAI 形没有该字段，原样保留。
fn model_item_is_listable(item: &serde_json::Value) -> bool {
    let Some(methods) = item
        .get("supportedGenerationMethods")
        .and_then(|v| v.as_array())
    else {
        return true;
    };
    methods.iter().any(|method| {
        matches!(
            method.as_str(),
            Some("generateContent") | Some("streamGenerateContent") | Some("bidiGenerateContent")
        )
    })
}

/// 连接探测用的用户消息。不用 "hi" / "hello"：不少网关把极短问候当无效请求直接吞掉或不回。
const CONNECTION_TEST_PROMPT: &str = "Reply with exactly: kivio-ok";
/// 给推理模型留一点思考预算；连通性测试不需要长回答。
const CONNECTION_TEST_MAX_OUTPUT_TOKENS: u32 = 64;

pub fn connection_test_url_and_body(
    api_format: ProviderApiFormat,
    base: &str,
    model: &str,
) -> (String, serde_json::Value) {
    match api_format {
        ProviderApiFormat::AnthropicMessages => (
            format!("{base}/messages"),
            serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": CONNECTION_TEST_PROMPT }],
                "max_tokens": CONNECTION_TEST_MAX_OUTPUT_TOKENS,
            }),
        ),
        ProviderApiFormat::Gemini => (
            format!(
                "{base}/models/{}:generateContent",
                model.trim_start_matches("models/")
            ),
            serde_json::json!({
                "contents": [{ "role": "user", "parts": [{ "text": CONNECTION_TEST_PROMPT }] }],
                "generationConfig": { "maxOutputTokens": CONNECTION_TEST_MAX_OUTPUT_TOKENS },
            }),
        ),
        // xAI 与 OpenAI 的 Responses 端点同形，测试请求体本来就只有这三个字段，
        // 都不在 xAI 的拒收名单上，无需分叉。
        ProviderApiFormat::OpenAiResponses | ProviderApiFormat::XaiResponses => (
            format!("{base}/responses"),
            serde_json::json!({
                "model": model,
                "input": CONNECTION_TEST_PROMPT,
                "max_output_tokens": CONNECTION_TEST_MAX_OUTPUT_TOKENS,
            }),
        ),
        ProviderApiFormat::OpenAiChat => (
            format!("{base}/chat/completions"),
            serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": CONNECTION_TEST_PROMPT }],
                "max_tokens": CONNECTION_TEST_MAX_OUTPUT_TOKENS,
            }),
        ),
    }
}
