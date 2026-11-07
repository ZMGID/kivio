//! Shared OpenAI-compatible Chat Completions wire protocol and host-configured HTTP provider.
use super::error::provider_error_code;
use super::usage::{model_usage_from_openai_value, model_usage_from_stream_value};
use super::{
    openai_messages_from_generate_request, pending_tool_calls_from_openai_message,
    stream_read_error, FinishReason, GenerateOutput, GenerateRequest, GeneratedImageData,
    LanguageModelProvider, ModelError, ModelFuture, ModelUsage, PendingToolCall, StreamPart,
    StreamSink,
};
use serde_json::Value;

/// Host-resolved wire options. Per-call provider_options override extra_body;
/// ToolChoice::None has final precedence over both.
#[derive(Clone, Debug, Default)]
pub struct OpenAiChatOptions {
    /// Resolved temperature; None omits the field in request_body.
    pub temperature: Option<f64>,
    pub extra_body: Value,
    pub prompt_cache_key: Option<String>,
    pub prompt_cache_retention: Option<String>,
    pub native_thinking_switch: bool,
}

/// Explicit transport configuration. The endpoint is the complete Chat Completions URL.
/// Credentials are supplied by the host and never read from disk or the environment.
pub struct OpenAiChatConfig {
    pub endpoint: reqwest::Url,
    pub api_key: Option<String>,
    pub headers: reqwest::header::HeaderMap,
    pub options: OpenAiChatOptions,
    /// Applies to buffered generation only. Configure stream idle timeouts on the client.
    pub request_timeout: Option<std::time::Duration>,
}

impl OpenAiChatConfig {
    pub fn new(endpoint: reqwest::Url) -> Self {
        Self {
            endpoint,
            api_key: None,
            headers: Default::default(),
            options: Default::default(),
            request_timeout: None,
        }
    }
}

/// Calls the supplied client's send once per invocation, without an SDK retry loop,
/// key rotation, or cache fallback. The client owns HTTP retries and redirects.
/// Dropping a generation future cancels its HTTP read.
pub struct OpenAiChatProvider {
    client: reqwest::Client,
    config: OpenAiChatConfig,
}

impl OpenAiChatProvider {
    pub fn new(client: reqwest::Client, config: OpenAiChatConfig) -> Self {
        Self { client, config }
    }

    async fn post(
        &self,
        request: &GenerateRequest,
        stream: bool,
    ) -> Result<reqwest::Response, ModelError> {
        let options = OpenAiChatOptions {
            temperature: request
                .options
                .temperature
                .or(self.config.options.temperature),
            ..self.config.options.clone()
        };
        let mut http = self
            .client
            .post(self.config.endpoint.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .headers(self.config.headers.clone())
            .json(&request_body(request, stream, &options));
        if let Some(key) = &self.config.api_key {
            http = http.bearer_auth(key);
        }
        if !stream {
            if let Some(timeout) = self.config.request_timeout {
                http = http.timeout(timeout);
            }
        }
        let response = http.send().await.map_err(|error| {
            ModelError::with_kind(
                format!("Chat request failed: {error}"),
                if error.is_timeout() {
                    super::ModelErrorKind::Timeout
                } else if error.is_builder() {
                    super::ModelErrorKind::InvalidRequest
                } else {
                    super::ModelErrorKind::Transport
                },
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|error| stream_read_error("Chat error body", &error))?;
            return Err(ModelError::from_provider_http(
                status.as_u16(),
                &body,
                format!(
                    "Chat API {}: {}",
                    status.as_u16(),
                    body.chars().take(500).collect::<String>()
                ),
            ));
        }
        Ok(response)
    }
}

impl LanguageModelProvider for OpenAiChatProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            let label = request_label(&request, "Chat API");
            let response = self.post(&request, false).await?;
            let raw = response
                .text()
                .await
                .map_err(|error| stream_read_error(&label, &error))?;
            let value = serde_json::from_str(&raw).map_err(|_| invalid_response(&label, &raw))?;
            output_from_chat_completion(&value, &raw, &label)
        })
    }

    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            let label = request_label(&request, "Chat stream");
            decode_chat_stream(self.post(&request, true).await?, sink, &label).await
        })
    }
}

impl crate::agent::provider_runtime::ModelRuntime for OpenAiChatProvider {
    fn generate<'a>(
        &'a self,
        call: crate::agent::provider_runtime::ModelCall,
        sink: &'a mut dyn crate::agent::provider_runtime::ModelOutputSink,
    ) -> crate::agent::provider_runtime::ProviderFuture<'a, Result<GenerateOutput, ModelError>>
    {
        Box::pin(async move {
            sink.request_started(&call);
            let request = crate::agent::history::request_from_history(
                &call.history,
                &call.model,
                call.tools,
                call.options,
                call.metadata,
            );
            if call.delivery == crate::agent::provider_runtime::ModelDelivery::Collected {
                LanguageModelProvider::stream(self, request, &mut |_part| Ok(())).await
            } else {
                LanguageModelProvider::stream(self, request, sink).await
            }
        })
    }
}

pub fn request_body(request: &GenerateRequest, stream: bool, options: &OpenAiChatOptions) -> Value {
    let mut body = serde_json::json!({
        "model": request.model,
        "messages": openai_messages_from_generate_request(request),
    });
    if request.options.max_tokens > 0 {
        body["max_tokens"] = Value::from(request.options.max_tokens);
    }
    if let Some(temperature) = options.temperature {
        body["temperature"] = serde_json::json!(temperature);
    }
    if stream {
        body["stream"] = Value::Bool(true);
        // usage 随流返回（OpenAI 标准参数；AI SDK/opencode 同款）。缺省时部分
        // provider 不在流里带 usage，token 统计只能靠估算。
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| tool.to_openai_tool())
                .collect(),
        );
        body["tool_choice"] = Value::String("auto".to_string());
    }
    if let Some(key) = options
        .prompt_cache_key
        .as_deref()
        .filter(|key| !key.is_empty())
    {
        body["prompt_cache_key"] = Value::String(key.to_string());
        if let Some(retention) = &options.prompt_cache_retention {
            body["prompt_cache_retention"] = Value::String(retention.clone());
        }
    }
    let native_thinking_switch = options.native_thinking_switch;
    if !request.options.thinking_enabled {
        if native_thinking_switch {
            body["thinking"] = serde_json::json!({ "type": "disabled" });
        } else {
            body["reasoning_effort"] = Value::String("none".to_string());
        }
    } else if native_thinking_switch {
        body["thinking"] = serde_json::json!({ "type": "enabled" });
    }
    // 思考等级 → OpenAI Chat `reasoning_effort`，原样下发（档位由模型库 reasoningEfforts 门控）。
    // 代理普遍接受;不发 Qwen/vLLM 私有的 enable_thinking / chat_template_kwargs。
    if let Some(effort) = request.options.thinking_level.as_deref() {
        body["reasoning_effort"] = Value::String(effort.to_string());
    }
    // 模型级额外请求体字段（model_overrides[model].extra_body）：原样 merge 进 body 根部，
    // 给严格端点塞标准 schema 外的私有旋钮（NVIDIA NIM / vLLM `chat_template_kwargs` 等）。
    // 放在单次 provider_options 之前，让显式的单次覆盖仍然最后生效。
    if let Some(extra) = options.extra_body.as_object() {
        for (key, value) in extra {
            body[key] = value.clone();
        }
    }
    if let Some(overrides) = request.options.provider_options.as_object() {
        for (key, value) in overrides {
            body[key] = value.clone();
        }
    }
    if request.options.tool_choice == super::ToolChoice::None {
        body.as_object_mut().unwrap().remove("tools");
        body["tool_choice"] = Value::String("none".to_string());
    }
    body
}

/// Decode a single production Chat Completions response. A finish reason commits
/// the response even if its optional usage tail disconnects; otherwise EOF is retryable.
pub async fn decode_chat_stream(
    mut response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
) -> Result<GenerateOutput, ModelError> {
    let mut sse = super::sse::SseDecoder::default();
    let mut full = String::new();
    let mut reasoning_full = String::new();
    let mut tool_partials: Vec<PartialToolCall> = Vec::new();
    let mut finish_reason = None;
    let mut usage: Option<ModelUsage> = None;
    let mut last_reasoning_summary_index: Option<u64> = None;
    // 代理仿 OpenRouter 出图：流式 chunk 的 `delta.images` 逐帧累积，finish 后并入 output。
    let mut images: Vec<GeneratedImageData> = Vec::new();

    'events: loop {
        let chunk_result = response.chunk().await;
        // finish_reason commits this model response. Losing the optional
        // usage/DONE tail must not replay an already completed request.
        if chunk_result.is_err() && finish_reason.is_some() {
            break;
        }
        let chunk = chunk_result.map_err(|err| stream_read_error(label, &err))?;
        let Some(chunk) = chunk else {
            break;
        };
        for data in sse.push(&chunk) {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                finish_reason =
                    Some(finish_reason.unwrap_or_else(|| FinishReason::from_openai("done")));
                break 'events;
            }
            let value: Value = match serde_json::from_str(data) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
                let message = format!("{label} stream error: {error}");
                let code = provider_error_code(error);
                return Err(ModelError::from_provider_stream(code.as_deref(), message));
            }
            if let Some(next_usage) = model_usage_from_stream_value(&value) {
                if let Some(input_tokens) = next_usage.input_tokens {
                    sink.emit(StreamPart::ContextUsage {
                        input_tokens,
                        output_tokens: next_usage.output_tokens.unwrap_or(0),
                    })?;
                }
                usage = Some(next_usage);
            }
            if let Some(reason) = openai_stream_finish_reason(&value) {
                finish_reason = Some(FinishReason::from_openai(reason));
            }
            handle_openai_stream_tool_calls(&value, &mut tool_partials, sink)?;
            for fragment in collect_chat_reasoning_fragments(&value) {
                if let Some(delta) = append_chat_stream_reasoning(
                    &mut reasoning_full,
                    &mut last_reasoning_summary_index,
                    &fragment,
                ) {
                    sink.emit(StreamPart::ReasoningDelta { delta })?;
                }
            }
            if let Some((content, mode)) = extract_chat_stream_text(&value) {
                if let Some(delta) = append_chat_stream_text(&mut full, content, mode) {
                    sink.emit(StreamPart::TextDelta { delta })?;
                }
            }
            for image in extract_chat_stream_images(&value) {
                sink.emit(StreamPart::ImageData {
                    mime_type: image.mime_type.clone(),
                    data: image.data.clone(),
                })?;
                images.push(image);
            }
        }
    }

    let reason = finish_reason.ok_or_else(|| ModelError::stream_ended_early(label))?;
    let tool_calls = finish_tool_call_partials(&mut tool_partials, sink)?;
    sink.emit(StreamPart::Finish {
        reason: reason.clone(),
        full: full.clone(),
    })?;
    let output = GenerateOutput {
        text: full,
        reasoning: non_empty(reasoning_full),
        tool_calls,
        usage,
        finish_reason: Some(reason),
        cancelled: false,
        web_search: None,
        images,
        reasoning_items: Vec::new(),
        canonical_message: None,
        billing_usage: None,
    };

    Ok(output)
}

pub fn output_from_chat_completion(
    value: &Value,
    raw: &str,
    label: &str,
) -> Result<GenerateOutput, ModelError> {
    let choice = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .ok_or_else(|| {
            let error = invalid_response(label, raw);
            if let Some(code) = value.get("error").and_then(provider_error_code) {
                ModelError::from_provider_payload(Some(&code), error.message)
            } else {
                error
            }
        })?;
    let message = choice
        .get("message")
        .ok_or_else(|| invalid_response(label, raw))?;
    let text = assistant_text_from_openai_message(message);
    let reasoning = reasoning_from_openai_message(message);
    let tool_calls = pending_tool_calls_from_openai_message(message);
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|value| value.as_str())
        .map(FinishReason::from_openai);
    let usage = model_usage_from_openai_response(value);
    // 代理仿 OpenRouter 出图：非流式在 message.images 携带图片数组，解析为协议无关 images。
    let images = message
        .get("images")
        .map(generated_images_from_openai_images)
        .unwrap_or_default();

    Ok(GenerateOutput {
        text,
        reasoning,
        tool_calls,
        usage,
        finish_reason,
        cancelled: false,
        web_search: None,
        images,
        reasoning_items: Vec::new(),
        canonical_message: None,
        billing_usage: None,
    })
}

fn invalid_response(label: &str, raw: &str) -> ModelError {
    ModelError::new(format!(
        "Invalid {label} response: {}",
        raw.chars().take(500).collect::<String>()
    ))
}

fn assistant_text_from_openai_message(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                if part.get("type").and_then(|value| value.as_str()) == Some("text") {
                    part.get("text").and_then(|value| value.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn reasoning_from_openai_message(message: &Value) -> Option<String> {
    if let Some(joined) = join_reasoning_details(message.get("reasoning_details")) {
        return Some(joined);
    }
    reasoning_string_field(message)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn reasoning_string_field(value: &Value) -> Option<&str> {
    value
        .get("reasoning_content")
        .or_else(|| value.get("reasoning"))
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
}

fn summary_index_in(value: &Value) -> Option<u64> {
    value.get("summary_index").and_then(Value::as_u64)
}

fn reasoning_detail_text(item: &Value) -> Option<&str> {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    if kind.contains("encrypted") {
        return None;
    }
    item.get("summary")
        .or_else(|| item.get("text"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && *text != "[REDACTED]")
}

fn join_reasoning_details(details: Option<&Value>) -> Option<String> {
    let items = details?.as_array()?;
    let mut parts = Vec::new();
    for item in items {
        if let Some(text) = reasoning_detail_text(item) {
            parts.push(text);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

#[derive(Debug, Clone)]
struct ChatReasoningFragment {
    text: String,
    mode: ChatStreamTextMode,
    summary_index: Option<u64>,
}

fn fragments_from_reasoning_details(
    details: Option<&Value>,
    mode: ChatStreamTextMode,
) -> Vec<ChatReasoningFragment> {
    let Some(items) = details.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let text = reasoning_detail_text(item)?.to_string();
            Some(ChatReasoningFragment {
                text,
                mode,
                summary_index: item.get("index").and_then(Value::as_u64),
            })
        })
        .collect()
}

fn model_usage_from_openai_response(value: &Value) -> Option<ModelUsage> {
    model_usage_from_openai_value(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatStreamTextMode {
    Delta,
    Snapshot,
}

fn extract_chat_stream_text(value: &Value) -> Option<(&str, ChatStreamTextMode)> {
    let choice = value.get("choices").and_then(|choices| choices.get(0))?;

    if let Some(content) = choice
        .get("delta")
        .and_then(|delta| delta.get("content"))
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
    {
        return Some((content, ChatStreamTextMode::Delta));
    }

    if let Some(content) = choice
        .get("text")
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
    {
        return Some((content, ChatStreamTextMode::Delta));
    }

    choice
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
        .map(|content| (content, ChatStreamTextMode::Snapshot))
}

fn collect_chat_reasoning_fragments(value: &Value) -> Vec<ChatReasoningFragment> {
    let Some(choice) = value.get("choices").and_then(|choices| choices.get(0)) else {
        return Vec::new();
    };
    if let Some(delta) = choice.get("delta") {
        let mut parts = fragments_from_reasoning_details(
            delta.get("reasoning_details"),
            ChatStreamTextMode::Delta,
        );
        if parts.is_empty() {
            if let Some(text) = reasoning_string_field(delta) {
                parts.push(ChatReasoningFragment {
                    text: text.to_string(),
                    mode: ChatStreamTextMode::Delta,
                    summary_index: summary_index_in(delta)
                        .or_else(|| summary_index_in(choice))
                        .or_else(|| summary_index_in(value)),
                });
            }
        }
        return parts;
    }
    if let Some(message) = choice.get("message") {
        let mut parts = fragments_from_reasoning_details(
            message.get("reasoning_details"),
            ChatStreamTextMode::Snapshot,
        );
        if parts.is_empty() {
            if let Some(text) = reasoning_string_field(message) {
                parts.push(ChatReasoningFragment {
                    text: text.to_string(),
                    mode: ChatStreamTextMode::Snapshot,
                    summary_index: summary_index_in(message)
                        .or_else(|| summary_index_in(choice))
                        .or_else(|| summary_index_in(value)),
                });
            }
        }
        return parts;
    }
    Vec::new()
}

/// GPT-5 concise 标题经 Chat Completions 中转时常整段进 `reasoning_content`，
/// 且没有 `summary_index`。下一条又以大写字母开头、中间没空格，就会粘成
/// `constraintsAnalyzing`。token 增量（"Par"+"sing"）不会命中。
fn glued_summary_separator(existing: &str, incoming: &str) -> &'static str {
    if existing.is_empty() || incoming.is_empty() {
        return "";
    }
    if existing.ends_with('\n')
        || existing.ends_with(' ')
        || incoming.starts_with('\n')
        || incoming.starts_with(' ')
    {
        return "";
    }
    let Some(last) = existing.chars().last() else {
        return "";
    };
    let Some(first) = incoming.chars().next() else {
        return "";
    };
    if last.is_lowercase() && first.is_uppercase() && existing.len() >= 8 && incoming.len() >= 8 {
        "\n\n"
    } else {
        ""
    }
}

fn reasoning_part_break(existing: &str) -> &'static str {
    if existing.is_empty() || existing.ends_with("\n\n") {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

fn append_chat_stream_reasoning(
    full: &mut String,
    last_summary_index: &mut Option<u64>,
    fragment: &ChatReasoningFragment,
) -> Option<String> {
    if fragment.text.is_empty() {
        return None;
    }

    let mut emitted = String::new();
    if let Some(index) = fragment.summary_index {
        let changed = last_summary_index.is_some_and(|prev| prev != index);
        *last_summary_index = Some(index);
        if changed {
            let br = reasoning_part_break(full);
            full.push_str(br);
            emitted.push_str(br);
        }
    }

    match fragment.mode {
        ChatStreamTextMode::Delta => {
            if fragment.summary_index.is_none() {
                let sep = glued_summary_separator(full, &fragment.text);
                full.push_str(sep);
                emitted.push_str(sep);
            }
            full.push_str(&fragment.text);
            emitted.push_str(&fragment.text);
            Some(emitted)
        }
        ChatStreamTextMode::Snapshot => {
            let text = fragment.text.as_str();
            if text == full.as_str() || full.starts_with(text) {
                return if emitted.is_empty() {
                    None
                } else {
                    Some(emitted)
                };
            }
            if text.starts_with(full.as_str()) {
                let suffix = &text[full.len()..];
                if fragment.summary_index.is_none() {
                    let sep = glued_summary_separator(full, suffix);
                    emitted.push_str(sep);
                    let rebuilt = format!("{full}{sep}{suffix}");
                    full.clear();
                    full.push_str(&rebuilt);
                } else {
                    full.clear();
                    full.push_str(text);
                }
                emitted.push_str(suffix);
                return if emitted.is_empty() {
                    None
                } else {
                    Some(emitted)
                };
            }
            if fragment.summary_index.is_none() {
                let sep = glued_summary_separator(full, text);
                full.push_str(sep);
                emitted.push_str(sep);
            }
            full.push_str(text);
            emitted.push_str(text);
            Some(emitted)
        }
    }
}

fn append_chat_stream_text(
    full: &mut String,
    text: &str,
    mode: ChatStreamTextMode,
) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    match mode {
        ChatStreamTextMode::Delta => {
            full.push_str(text);
            Some(text.to_string())
        }
        ChatStreamTextMode::Snapshot => {
            if text == full || full.starts_with(text) {
                return None;
            }
            if text.starts_with(full.as_str()) {
                let delta = text[full.len()..].to_string();
                full.clear();
                full.push_str(text);
                return if delta.is_empty() { None } else { Some(delta) };
            }

            full.push_str(text);
            Some(text.to_string())
        }
    }
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments_raw: String,
    started: bool,
}

const MAX_STREAM_TOOL_CALLS: u64 = 1024;

fn handle_openai_stream_tool_calls(
    value: &Value,
    partials: &mut Vec<PartialToolCall>,
    sink: &mut (dyn StreamSink + Send),
) -> Result<(), ModelError> {
    let Some(calls) = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("tool_calls"))
        .and_then(|tool_calls| tool_calls.as_array())
    else {
        return Ok(());
    };

    for call in calls {
        let index = call
            .get("index")
            .and_then(|value| value.as_u64())
            .unwrap_or(partials.len() as u64);
        if index >= MAX_STREAM_TOOL_CALLS {
            return Err(ModelError::with_kind(
                format!("Invalid Chat stream response: tool call index {index} exceeds the limit of {} calls", MAX_STREAM_TOOL_CALLS),
                super::ModelErrorKind::Transport,
            ));
        }
        let index = index as usize;
        while partials.len() <= index {
            partials.push(PartialToolCall::default());
        }
        let partial = &mut partials[index];
        // id 只在该 tool_call 首个分片给;续片不带 id 或带空串 `""`(如 SenseNova）。
        // 只在**非空**时才覆盖，否则空续片会把首片的真 id 冲掉 → 回放空 tool_call_id →
        // 严格校验端点(商汤 DeepSeek 等)400 invalid tool_call_id。与下面 name 的空过滤一致。
        if let Some(id) = call
            .get("id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        {
            partial.id = Some(id.to_string());
        }
        if let Some(name) = call
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        {
            partial.name = Some(name.to_string());
        }
        let mut started_now = false;
        if !partial.started {
            if let (Some(id), Some(name)) = (partial.id.as_deref(), partial.name.as_deref()) {
                sink.emit(StreamPart::ToolCallStart {
                    id: id.to_string(),
                    name: name.to_string(),
                })?;
                partial.started = true;
                started_now = true;
            }
        }
        if started_now && !partial.arguments_raw.is_empty() {
            if let Some(id) = partial.id.as_deref() {
                sink.emit(StreamPart::ToolCallDelta {
                    id: id.to_string(),
                    delta: partial.arguments_raw.clone(),
                })?;
            }
        }
        if let Some(delta) = call
            .get("function")
            .and_then(|function| function.get("arguments"))
            .and_then(|value| match value {
                // 标准：参数以字符串分片流式到达，逐片累加。
                Value::String(text) => (!text.is_empty()).then(|| text.clone()),
                // 兼容：部分 OpenAI 兼容网关把 arguments 作为完整 JSON **对象**一次性发来
                // （非分片）。序列化成字符串当作整段累加，否则参数会被静默丢弃。
                Value::Null => None,
                other => serde_json::to_string(other).ok(),
            })
        {
            partial.arguments_raw.push_str(&delta);
            if partial.started {
                let Some(id) = partial.id.as_deref() else {
                    continue;
                };
                sink.emit(StreamPart::ToolCallDelta {
                    id: id.to_string(),
                    delta: delta.to_string(),
                })?;
            }
        }
    }
    Ok(())
}

fn finish_tool_call_partials(
    partials: &mut Vec<PartialToolCall>,
    sink: &mut (dyn StreamSink + Send),
) -> Result<Vec<PendingToolCall>, ModelError> {
    let mut calls = Vec::new();
    for (index, partial) in partials.drain(..).enumerate() {
        let Some(name) = partial.name else {
            continue;
        };
        // 兜底:None 或空串都生成合法非空 id,绝不产出空 tool_call_id。
        let id = partial
            .id
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("call_{index}"));
        let raw = if partial.arguments_raw.trim().is_empty() {
            "{}".to_string()
        } else {
            partial.arguments_raw
        };
        if !partial.started {
            sink.emit(StreamPart::ToolCallStart {
                id: id.clone(),
                name: name.clone(),
            })?;
            if raw != "{}" {
                sink.emit(StreamPart::ToolCallDelta {
                    id: id.clone(),
                    delta: raw.clone(),
                })?;
            }
        }
        let (arguments, arguments_parse_error) = super::parse_tool_arguments(&raw);
        let call = PendingToolCall {
            id,
            function_name: name,
            arguments,
            arguments_raw: raw,
            arguments_parse_error,
            signature: None,
        };
        sink.emit(StreamPart::ToolCallDone { call: call.clone() })?;
        calls.push(call);
    }
    Ok(calls)
}

/// 从 OpenRouter 风格的 `images` 数组解析出模型生成的图片（协议无关形态）。
/// 形如 `[{type:"image_url", image_url:{url:"data:image/png;base64,..."}}]`（这些代理仿
/// OpenRouter 出图）。仅收 data URL；http(s) 直链或格式异常项静默跳过（不报错，保持无图
/// 响应字节等价——绝大多数普通 chat 无该字段，行为完全不变）。
fn generated_images_from_openai_images(images: &Value) -> Vec<GeneratedImageData> {
    let Some(array) = images.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in array {
        let Some(url) = item
            .get("image_url")
            .or_else(|| item.get("imageUrl"))
            .and_then(|value| value.get("url"))
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        if let Some(image) = parse_openai_image_data_url(url) {
            out.push(image);
        }
    }
    out
}

/// 解析 data URL 为 GeneratedImageData（mime + 纯 base64）。非 data URL（含 http(s) 直链）
/// 或非 base64 编码返回 None 静默跳过——现阶段不下载远程直链，只避免误伤正常 chat。
fn parse_openai_image_data_url(url: &str) -> Option<GeneratedImageData> {
    let trimmed = url.trim();
    let rest = trimmed.strip_prefix("data:")?;
    let (metadata, payload) = rest.split_once(',')?;
    if !metadata
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return None;
    }
    let payload = payload.trim();
    if payload.is_empty() {
        return None;
    }
    let mime_type = metadata
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| value.starts_with("image/"))
        .unwrap_or("image/png")
        .to_string();
    Some(GeneratedImageData {
        mime_type,
        data: payload.to_string(),
    })
}

/// 流式 chunk 的 `choices[0].delta.images` → 协议无关 images（形状同非流式 message.images）。
fn extract_chat_stream_images(value: &Value) -> Vec<GeneratedImageData> {
    value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("images"))
        .map(generated_images_from_openai_images)
        .unwrap_or_default()
}

fn openai_stream_finish_reason(value: &Value) -> Option<&str> {
    value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(|value| value.as_str())
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn request_label(request: &GenerateRequest, fallback: &str) -> String {
    request
        .metadata
        .label
        .trim()
        .is_empty()
        .then(|| fallback.to_string())
        .unwrap_or_else(|| request.metadata.label.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_tool_index_is_bounded_before_allocation() {
        for index in [MAX_STREAM_TOOL_CALLS, u64::MAX] {
            let chunk = serde_json::json!({"choices":[{"delta":{"tool_calls":[{
                "index": index, "id":"call", "function":{"name":"test","arguments":"{}"}
            }]}}]});
            let mut partials = Vec::new();
            let mut emitted = Vec::new();
            let error = handle_openai_stream_tool_calls(&chunk, &mut partials, &mut |part| {
                emitted.push(part);
                Ok(())
            })
            .unwrap_err();
            assert_eq!(error.kind, super::super::ModelErrorKind::Transport);
            assert!(!error.is_retryable_stream_error());
            assert!(partials.is_empty());
            assert!(emitted.is_empty());
        }
    }

    #[test]
    fn resolved_options_and_tool_suppression_keep_wire_precedence() {
        let mut request = GenerateRequest {
            model: "model".into(),
            system: "system".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            options: super::super::GenerateOptions {
                temperature: Some(99.0),
                max_tokens: 0,
                tool_choice: super::super::ToolChoice::None,
                provider_options: serde_json::json!({"tools":[{"type":"function"}], "tool_choice":"auto", "vendor":"call"}),
                ..Default::default()
            },
            metadata: Default::default(),
        };
        let options = OpenAiChatOptions {
            extra_body: serde_json::json!({"vendor":"model"}),
            prompt_cache_retention: Some("24h".into()),
            ..Default::default()
        };
        let body = request_body(&request, true, &options);
        assert!(body.get("tools").is_none());
        assert_eq!(body["tool_choice"], "none");
        assert_eq!(body["vendor"], "call");
        assert!(body.get("temperature").is_none());
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("prompt_cache_retention").is_none());
        assert_eq!(body["stream_options"]["include_usage"], true);
        request.options.provider_options = Value::Null;
        let body = request_body(&request, false, &options);
        assert_eq!(body["vendor"], "model");
    }
    #[test]
    fn parses_text_reasoning_and_tool_calls() {
        let value = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "I'll call a tool.",
                    "reasoning_content": "Need data.",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "web_search",
                            "arguments": "{\"query\":\"kivio\"}"
                        }
                    }]
                }
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8}
        });

        let output = output_from_chat_completion(&value, "{}", "test").expect("output");

        assert_eq!(output.text, "I'll call a tool.");
        assert_eq!(output.reasoning.as_deref(), Some("Need data."));
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].function_name, "web_search");
        assert_eq!(
            output.usage.as_ref().and_then(|usage| usage.total_tokens),
            Some(8)
        );
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("tool_calls")
        );
    }

    fn fold_chat_reasoning(chunks: &[Value]) -> String {
        let mut full = String::new();
        let mut last_index = None;
        for chunk in chunks {
            for fragment in collect_chat_reasoning_fragments(chunk) {
                let _ = append_chat_stream_reasoning(&mut full, &mut last_index, &fragment);
            }
        }
        full
    }

    fn reasoning_delta_chunk(text: &str) -> Value {
        serde_json::json!({
            "choices": [{ "delta": { "reasoning_content": text } }]
        })
    }

    #[test]
    fn chat_stream_glues_token_deltas_but_splits_summary_titles() {
        // DeepSeek 式 token 增量必须仍拼成一词。
        assert_eq!(
            fold_chat_reasoning(&[
                reasoning_delta_chunk("Par"),
                reasoning_delta_chunk("sing table"),
            ]),
            "Parsing table"
        );

        // 中转把 GPT-5 每条 concise 标题整段塞进 reasoning_content、中间没空格。
        assert_eq!(
            fold_chat_reasoning(&[
                reasoning_delta_chunk("Parsing table and defining drawing constraints"),
                reasoning_delta_chunk("Analyzing shape-based drawing strategy for guarantee"),
            ]),
            "Parsing table and defining drawing constraints\n\n\
             Analyzing shape-based drawing strategy for guarantee"
        );
    }

    #[test]
    fn chat_stream_reasoning_summary_index_separates_parts() {
        let chunks = [
            serde_json::json!({
                "choices": [{ "delta": {
                    "reasoning_content": "Establishing 21 as minimal guarantee",
                    "summary_index": 0
                }}]
            }),
            serde_json::json!({
                "choices": [{ "delta": {
                    "reasoning_content": " by feel",
                    "summary_index": 0
                }}]
            }),
            serde_json::json!({
                "choices": [{ "delta": {
                    "reasoning_content": "Confirming rounds guarantee A and P",
                    "summary_index": 1
                }}]
            }),
        ];
        assert_eq!(
            fold_chat_reasoning(&chunks),
            "Establishing 21 as minimal guarantee by feel\n\n\
             Confirming rounds guarantee A and P"
        );
    }

    #[test]
    fn chat_stream_reasoning_details_are_joined_by_index() {
        let chunks = [
            serde_json::json!({
                "choices": [{ "delta": { "reasoning_details": [{
                    "type": "reasoning.summary",
                    "summary": "Parsing table and defining drawing constraints",
                    "index": 0
                }]}}]
            }),
            serde_json::json!({
                "choices": [{ "delta": { "reasoning_details": [{
                    "type": "reasoning.summary",
                    "summary": "Analyzing shape-based drawing strategy for guarantee",
                    "index": 1
                }]}}]
            }),
        ];
        assert_eq!(
            fold_chat_reasoning(&chunks),
            "Parsing table and defining drawing constraints\n\n\
             Analyzing shape-based drawing strategy for guarantee"
        );
    }

    #[test]
    fn nonstream_reasoning_details_are_preferred_over_glued_string() {
        let value = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "21",
                    "reasoning_content": "TitleATitleB",
                    "reasoning_details": [
                        { "type": "reasoning.summary", "summary": "Title A", "index": 0 },
                        { "type": "reasoning.summary", "summary": "Title B", "index": 1 }
                    ]
                }
            }]
        });
        let output = output_from_chat_completion(&value, "{}", "test").expect("output");
        assert_eq!(output.reasoning.as_deref(), Some("Title A\n\nTitle B"));
    }

    #[test]
    fn stream_tool_call_chunks_become_stream_parts() {
        let mut parts = Vec::new();
        let mut sink = |part| {
            parts.push(part);
            Ok(())
        };
        let mut partials = Vec::new();
        let start = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "function": {"name": "web_search", "arguments": "{\"q\""}
                    }]
                }
            }]
        });
        let next = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": ":\"rust\"}"}
                    }]
                }
            }]
        });

        handle_openai_stream_tool_calls(&start, &mut partials, &mut sink).expect("start");
        handle_openai_stream_tool_calls(&next, &mut partials, &mut sink).expect("next");
        let calls = finish_tool_call_partials(&mut partials, &mut sink).expect("finish");

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["q"], "rust");
        assert!(matches!(parts[0], StreamPart::ToolCallStart { .. }));
        assert!(matches!(
            parts.last(),
            Some(StreamPart::ToolCallDone { .. })
        ));
    }

    #[test]
    fn stream_tool_call_keeps_first_chunk_id_over_empty_continuation() {
        // SenseNova 等:首片给真 id,续片给空 id `""`。空续片不得覆盖真 id。
        let mut parts = Vec::new();
        let mut sink = |part| {
            parts.push(part);
            Ok(())
        };
        let mut partials = Vec::new();
        let start = serde_json::json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0, "id": "call_real", "function": {"name": "bash", "arguments": "{\"c\""}
            }]}}]
        });
        let cont = serde_json::json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0, "id": "", "function": {"arguments": ":\"x\"}"}
            }]}}]
        });
        handle_openai_stream_tool_calls(&start, &mut partials, &mut sink).expect("start");
        handle_openai_stream_tool_calls(&cont, &mut partials, &mut sink).expect("cont");
        let calls = finish_tool_call_partials(&mut partials, &mut sink).expect("finish");
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].id, "call_real",
            "empty continuation id must not overwrite real id"
        );
        assert_eq!(calls[0].arguments["c"], "x");
    }

    #[test]
    fn stream_tool_call_synthesizes_id_when_all_empty() {
        // id 从头到尾都空 → 生成合法非空 id,绝不发空 tool_call_id。
        let mut parts = Vec::new();
        let mut sink = |part| {
            parts.push(part);
            Ok(())
        };
        let mut partials = Vec::new();
        let chunk = serde_json::json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0, "id": "", "function": {"name": "bash", "arguments": "{}"}
            }]}}]
        });
        handle_openai_stream_tool_calls(&chunk, &mut partials, &mut sink).expect("chunk");
        let calls = finish_tool_call_partials(&mut partials, &mut sink).expect("finish");
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].id.is_empty(), "id must not be empty");
        assert_eq!(calls[0].id, "call_0");
    }

    #[test]
    fn stream_tool_call_accepts_object_form_arguments() {
        // 部分 OpenAI 兼容网关（如 codex/spark 代理）把 function.arguments 作为完整 JSON
        // 对象一次性发来，而非字符串分片。回归：旧逻辑只认字符串会丢成 `{}`，使必填参数缺失。
        let mut parts = Vec::new();
        let mut sink = |part| {
            parts.push(part);
            Ok(())
        };
        let mut partials = Vec::new();
        let chunk = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "function": { "name": "web_search", "arguments": { "query": "吉林市 明天 天气" } }
                    }]
                }
            }]
        });

        handle_openai_stream_tool_calls(&chunk, &mut partials, &mut sink).expect("chunk");
        let calls = finish_tool_call_partials(&mut partials, &mut sink).expect("finish");

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_name, "web_search");
        assert_eq!(calls[0].arguments["query"], "吉林市 明天 天气");
        assert!(calls[0].arguments_parse_error.is_none());
    }

    #[test]
    fn parses_message_images_from_openrouter_style_response() {
        // 代理仿 OpenRouter：非流式 message.images 携带 data URL 图片数组。
        let value = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "here is your image",
                    "images": [{
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,aGVsbG8=" }
                    }]
                }
            }]
        });

        let output = output_from_chat_completion(&value, "{}", "test").expect("output");
        assert_eq!(output.text, "here is your image");
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].mime_type, "image/png");
        assert_eq!(output.images[0].data, "aGVsbG8=");
    }

    #[test]
    fn response_without_images_has_empty_images() {
        // 无 images 字段：行为字节等价，images 为空（普通 chat 非回归）。
        let value = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "plain text" }
            }]
        });
        let output = output_from_chat_completion(&value, "{}", "test").expect("output");
        assert!(output.images.is_empty());
    }

    #[test]
    fn stream_delta_images_emit_image_data_and_aggregate() {
        // 流式 delta.images：解析出图 → 发 StreamPart::ImageData 且累积进最终 images。
        // 复刻 stream_inner 每 chunk 的 emit + push 逻辑（无需真实 HTTP）。
        let chunk = serde_json::json!({
            "choices": [{
                "delta": {
                    "images": [{
                        "type": "image_url",
                        "image_url": { "url": "data:image/webp;base64,d29ybGQ=" }
                    }]
                }
            }]
        });

        let mut parts = Vec::new();
        let mut images: Vec<GeneratedImageData> = Vec::new();
        {
            let mut sink = |part| {
                parts.push(part);
                Ok::<(), ModelError>(())
            };
            for image in extract_chat_stream_images(&chunk) {
                sink.emit(StreamPart::ImageData {
                    mime_type: image.mime_type.clone(),
                    data: image.data.clone(),
                })
                .expect("emit");
                images.push(image);
            }
        }

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].mime_type, "image/webp");
        assert_eq!(images[0].data, "d29ybGQ=");
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            StreamPart::ImageData { mime_type, data } => {
                assert_eq!(mime_type, "image/webp");
                assert_eq!(data, "d29ybGQ=");
            }
            other => panic!("expected ImageData, got {other:?}"),
        }
    }

    #[test]
    fn stream_delta_without_images_yields_nothing() {
        // delta 无 images：解析为空（普通流式 chat 非回归）。
        let chunk = serde_json::json!({
            "choices": [{ "delta": { "content": "hi" } }]
        });
        assert!(extract_chat_stream_images(&chunk).is_empty());
    }

    #[test]
    fn parse_openai_image_data_url_skips_http_and_non_base64() {
        // http(s) 直链、非 base64 data URL、非 data URL 均静默跳过（返回 None，不报错）。
        assert!(parse_openai_image_data_url("https://example.com/a.png").is_none());
        assert!(parse_openai_image_data_url("data:image/png,notbase64").is_none());
        assert!(parse_openai_image_data_url("not-a-url").is_none());
        let ok = parse_openai_image_data_url("data:image/jpeg;base64,aGVsbG8=").expect("parsed");
        assert_eq!(ok.mime_type, "image/jpeg");
        assert_eq!(ok.data, "aGVsbG8=");
    }

    #[test]
    fn stream_tool_call_replays_arguments_buffered_before_start() {
        let mut parts = Vec::new();
        let mut partials = Vec::new();
        let early_arguments = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": "{\"path\":\"demo.txt\""}
                    }]
                }
            }]
        });
        let delayed_start = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_delayed",
                        "function": {"name": "write_file"}
                    }]
                }
            }]
        });
        let final_arguments = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": ",\"content\":\"ok\"}"}
                    }]
                }
            }]
        });

        {
            let mut sink = |part| {
                parts.push(part);
                Ok(())
            };
            handle_openai_stream_tool_calls(&early_arguments, &mut partials, &mut sink)
                .expect("early arguments");
        }
        assert!(parts.is_empty());
        let mut sink = |part| {
            parts.push(part);
            Ok(())
        };
        handle_openai_stream_tool_calls(&delayed_start, &mut partials, &mut sink)
            .expect("delayed start");
        handle_openai_stream_tool_calls(&final_arguments, &mut partials, &mut sink)
            .expect("final arguments");
        let calls = finish_tool_call_partials(&mut partials, &mut sink).expect("finish");

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_delayed");
        assert_eq!(calls[0].arguments["content"], "ok");
        assert!(matches!(parts[0], StreamPart::ToolCallStart { .. }));
        assert!(matches!(
            &parts[1],
            StreamPart::ToolCallDelta { delta, .. } if delta.contains("demo.txt")
        ));
        assert!(matches!(
            parts.last(),
            Some(StreamPart::ToolCallDone { .. })
        ));
    }
}
