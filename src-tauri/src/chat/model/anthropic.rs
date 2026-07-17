use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_ENCODING};
use serde_json::Value;

use crate::api::{send_with_failover, with_standard_request_timeout};
use crate::settings::ModelProvider;
use crate::state::AppState;
use crate::usage::{
    chat_usage_source_for_label, error_kind_from_message, model_usage_from_anthropic_value,
    operation_from_label, record_model_call, UsageRecordInput,
};

use super::{
    parse_tool_arguments, stream_read_error, GenerateOutput, GenerateRequest,
    LanguageModelProvider, MessagePart, ModelError, ModelFuture, ModelMessage, ModelRole,
    ModelTool, ModelUsage, PendingToolCall, StreamPart, StreamSink,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicMessagesProvider<'a> {
    state: &'a AppState,
    provider: &'a ModelProvider,
    retry_attempts: usize,
}

impl<'a> AnthropicMessagesProvider<'a> {
    pub fn new(state: &'a AppState, provider: &'a ModelProvider, retry_attempts: usize) -> Self {
        Self {
            state,
            provider,
            retry_attempts,
        }
    }
}

impl LanguageModelProvider for AnthropicMessagesProvider<'_> {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move { self.generate_inner(request).await })
    }

    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move { self.stream_inner(request, sink).await })
    }
}

impl AnthropicMessagesProvider<'_> {
    async fn generate_inner(&self, request: GenerateRequest) -> Result<GenerateOutput, ModelError> {
        let label = request_label(&request, "Anthropic Messages API");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let body = self.request_body(&request, false);
        let response = send_with_failover(
            self.state,
            &label,
            self.retry_attempts,
            &self.provider.id,
            &self.provider.api_keys,
            |key| {
                with_standard_request_timeout(crate::api::attach_json_body(
                    self.state
                        .http
                        .post(self.messages_url())
                        .headers(anthropic_headers(key).unwrap_or_default())
                        .header(ACCEPT_ENCODING, "identity"),
                    &body,
                    self.provider.compress_request_body,
                ))
                .send()
            },
        )
        .await
        .map_err(|err| {
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &err);
            self.record_debug_failure(&request, &label, false, &err, started_at, started.elapsed());
            ModelError::new(err)
        })?;

        let raw = response.text().await.map_err(|err| {
            let message = format!("{label} read body: {err}");
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &message);
            self.record_debug_failure(
                &request,
                &label,
                false,
                &message,
                started_at,
                started.elapsed(),
            );
            ModelError::new(message)
        })?;
        let value: Value = serde_json::from_str(&raw).map_err(|err| {
            let message = format!(
                "{label} parse JSON: {} (body: {})",
                err,
                raw.chars().take(500).collect::<String>()
            );
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &message);
            self.record_debug_failure(
                &request,
                &label,
                false,
                &message,
                started_at,
                started.elapsed(),
            );
            ModelError::new(message)
        })?;
        let output = output_from_anthropic_message(&value, &label)?;
        self.record_usage_success(
            &request,
            &label,
            started_at,
            started.elapsed(),
            output.usage.clone(),
        );
        self.record_debug_success(
            &request,
            &label,
            false,
            &output,
            started_at,
            started.elapsed(),
        );
        Ok(output)
    }

    async fn stream_inner(
        &self,
        request: GenerateRequest,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<GenerateOutput, ModelError> {
        let label = request_label(&request, "Anthropic stream");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let body = self.request_body(&request, true);
        let mut response = send_with_failover(
            self.state,
            &label,
            self.retry_attempts,
            &self.provider.id,
            &self.provider.api_keys,
            |key| {
                crate::api::attach_json_body(
                    self.state
                        .http
                        .post(self.messages_url())
                        .headers(anthropic_headers(key).unwrap_or_default())
                        .header(ACCEPT_ENCODING, "identity"),
                    &body,
                    self.provider.compress_request_body,
                )
                .send()
            },
        )
        .await
        .map_err(|err| {
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &err);
            self.record_debug_failure(&request, &label, true, &err, started_at, started.elapsed());
            ModelError::new(err)
        })?;

        let mut buffer = String::new();
        let mut utf8 = crate::api::Utf8StreamDecoder::default();
        let mut full = String::new();
        let mut reasoning_full = String::new();
        let mut tool_calls = Vec::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input_parts: Vec<String> = Vec::new();
        let mut finish_reason = "stop".to_string();
        let mut usage: Option<ModelUsage> = None;

        loop {
            let chunk = response.chunk().await.map_err(|err| {
                let model_error = stream_read_error(&label, &err);
                self.record_usage_failure(
                    &request,
                    &label,
                    started_at,
                    started.elapsed(),
                    &model_error.to_string(),
                );
                self.record_debug_failure(
                    &request,
                    &label,
                    true,
                    &model_error.to_string(),
                    started_at,
                    started.elapsed(),
                );
                model_error
            })?;
            let Some(chunk) = chunk else {
                break;
            };
            buffer.push_str(&utf8.push(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line: String = buffer.drain(..=pos).collect();
                match parse_anthropic_sse_event(&line) {
                    Some(AnthropicSseEvent::TextDelta(text)) => {
                        full.push_str(&text);
                        sink.emit(StreamPart::TextDelta { delta: text })?;
                    }
                    Some(AnthropicSseEvent::ThinkingDelta(thinking)) => {
                        reasoning_full.push_str(&thinking);
                        sink.emit(StreamPart::ReasoningDelta { delta: thinking })?;
                    }
                    Some(AnthropicSseEvent::ToolUseStart { id, name }) => {
                        sink.emit(StreamPart::ToolCallStart {
                            id: id.clone(),
                            name: name.clone(),
                        })?;
                        current_tool_id = id;
                        current_tool_name = name;
                        current_tool_input_parts.clear();
                    }
                    Some(AnthropicSseEvent::ToolInputDelta(json)) => {
                        if !current_tool_id.is_empty() {
                            sink.emit(StreamPart::ToolCallDelta {
                                id: current_tool_id.clone(),
                                delta: json.clone(),
                            })?;
                        }
                        current_tool_input_parts.push(json);
                    }
                    Some(AnthropicSseEvent::ContentBlockStop) => {
                        if !current_tool_id.is_empty() {
                            let call = assemble_tool_call_from_stream(
                                &current_tool_id,
                                &current_tool_name,
                                &current_tool_input_parts,
                            );
                            sink.emit(StreamPart::ToolCallDone { call: call.clone() })?;
                            tool_calls.push(call);
                        }
                        current_tool_id.clear();
                        current_tool_name.clear();
                        current_tool_input_parts.clear();
                    }
                    Some(AnthropicSseEvent::MessageStop) => {
                        sink.emit(StreamPart::Finish {
                            reason: finish_reason.clone(),
                            full: full.clone(),
                        })?;
                        let output =
                            stream_output(full, reasoning_full, tool_calls, finish_reason, usage);
                        self.record_usage_success(
                            &request,
                            &label,
                            started_at,
                            started.elapsed(),
                            output.usage.clone(),
                        );
                        self.record_debug_success(
                            &request,
                            &label,
                            true,
                            &output,
                            started_at,
                            started.elapsed(),
                        );
                        return Ok(output);
                    }
                    Some(AnthropicSseEvent::MessageStopWithReason {
                        reason,
                        usage: next_usage,
                    }) => {
                        finish_reason = finish_reason_from_anthropic_stop_reason(&reason);
                        if next_usage.is_some() {
                            usage = next_usage;
                        }
                        sink.emit(StreamPart::Finish {
                            reason: finish_reason.clone(),
                            full: full.clone(),
                        })?;
                        let output =
                            stream_output(full, reasoning_full, tool_calls, finish_reason, usage);
                        self.record_usage_success(
                            &request,
                            &label,
                            started_at,
                            started.elapsed(),
                            output.usage.clone(),
                        );
                        self.record_debug_success(
                            &request,
                            &label,
                            true,
                            &output,
                            started_at,
                            started.elapsed(),
                        );
                        return Ok(output);
                    }
                    Some(AnthropicSseEvent::Error(err)) => {
                        sink.emit(StreamPart::Error {
                            message: err.clone(),
                        })?;
                        return Err(ModelError::new(format!("Anthropic stream error: {err}")));
                    }
                    None => {}
                }
            }
        }

        sink.emit(StreamPart::Finish {
            reason: finish_reason.clone(),
            full: full.clone(),
        })?;
        Ok(stream_output(
            full,
            reasoning_full,
            tool_calls,
            finish_reason,
            usage,
        ))
    }

    fn messages_url(&self) -> String {
        anthropic_messages_url(&self.provider.base_url)
    }

    fn request_body(&self, request: &GenerateRequest, stream: bool) -> Value {
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": anthropic_messages_from_generate_request(request),
            "max_tokens": request.options.max_tokens,
        });
        if let Some(temperature) = crate::chat::model_metadata::temperature_for_request(
            request.options.temperature,
            Some(self.provider),
            &request.model,
        ) {
            body["temperature"] = serde_json::json!(temperature);
        }
        if !request.system.trim().is_empty() {
            body["system"] = Value::String(request.system.clone());
        }
        if stream {
            body["stream"] = Value::Bool(true);
        }
        let tools = anthropic_tools_from_model_tools(&request.tools);
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        // 思考等级（仅在用户显式选了等级时注入）。Claude 4.6+/4.7/4.8/Fable5 用
        // adaptive thinking + output_config.effort；budget_tokens 已被移除（发了 400）。
        if let Some(level) = request
            .options
            .thinking_level
            .as_deref()
            .filter(|l| !l.is_empty())
        {
            body["thinking"] = serde_json::json!({ "type": "adaptive" });
            body["output_config"] = serde_json::json!({ "effort": level });
        }
        if let Some(overrides) = request.options.provider_options.as_object() {
            for (key, value) in overrides {
                body[key] = value.clone();
            }
        }
        body
    }

    /// 重建本次请求实际会带的 headers（脱敏后）供请求调试面板展示。镜像 `anthropic_headers`
    /// （x-api-key / anthropic-version / content-type）+ 发送路径另加的 Accept-Encoding。
    /// x-api-key 用首个 key（正常发送用的也是它）派生脱敏预览。
    fn debug_request_headers(&self) -> std::collections::BTreeMap<String, String> {
        let mut headers = std::collections::BTreeMap::new();
        if let Some(key) = self.provider.api_keys.first() {
            headers.insert("x-api-key".to_string(), key.clone());
        }
        headers.insert(
            "anthropic-version".to_string(),
            ANTHROPIC_VERSION.to_string(),
        );
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("Accept-Encoding".to_string(), "identity".to_string());
        crate::chat::request_debug::sanitize_headers(headers)
    }

    /// 记录一次成功调用到请求调试缓冲。开关关时首行短路（零开销）。
    fn record_debug_success(
        &self,
        request: &GenerateRequest,
        label: &str,
        stream: bool,
        output: &GenerateOutput,
        started_at: i64,
        duration: std::time::Duration,
    ) {
        if !self.state.request_debug_enabled() {
            return;
        }
        let record = crate::chat::request_debug::build_debug_record(
            crate::chat::request_debug::DebugRecordArgs {
                provider: self.provider,
                request,
                label,
                started_at,
                duration_ms: duration.as_millis() as u64,
                status: "success",
                url: self.messages_url(),
                headers: self.debug_request_headers(),
                body: self.request_body(request, stream),
                stream,
                response: crate::chat::request_debug::RequestDebugResponse::from_output(
                    output,
                    Some(200),
                ),
            },
        );
        crate::chat::request_debug::record(self.state, record);
    }

    /// 记录一次失败调用到请求调试缓冲。开关关时首行短路（零开销）。
    fn record_debug_failure(
        &self,
        request: &GenerateRequest,
        label: &str,
        stream: bool,
        error: &str,
        started_at: i64,
        duration: std::time::Duration,
    ) {
        if !self.state.request_debug_enabled() {
            return;
        }
        let record = crate::chat::request_debug::build_debug_record(
            crate::chat::request_debug::DebugRecordArgs {
                provider: self.provider,
                request,
                label,
                started_at,
                duration_ms: duration.as_millis() as u64,
                status: "error",
                url: self.messages_url(),
                headers: self.debug_request_headers(),
                body: self.request_body(request, stream),
                stream,
                response: crate::chat::request_debug::RequestDebugResponse::from_error(
                    error,
                    crate::api::extract_status_code(error),
                ),
            },
        );
        crate::chat::request_debug::record(self.state, record);
    }

    fn record_usage_success(
        &self,
        request: &GenerateRequest,
        label: &str,
        started_at: i64,
        duration: std::time::Duration,
        usage: Option<ModelUsage>,
    ) {
        let source = request
            .metadata
            .usage_source
            .clone()
            .unwrap_or_else(|| chat_usage_source_for_label(label));
        let operation = request
            .metadata
            .usage_operation
            .clone()
            .unwrap_or_else(|| operation_from_label(label));
        record_model_call(
            self.state,
            UsageRecordInput {
                provider: self.provider,
                model: &request.model,
                source: &source,
                operation: &operation,
                status: "success",
                status_code: Some(200),
                usage,
                usage_source: "provider_reported",
                started_at,
                duration_ms: duration.as_millis() as u64,
                conversation_id: request.metadata.conversation_id.clone(),
                message_id: request.metadata.message_id.clone(),
                error_kind: None,
            },
        );
    }

    fn record_usage_failure(
        &self,
        request: &GenerateRequest,
        label: &str,
        started_at: i64,
        duration: std::time::Duration,
        error: &str,
    ) {
        let source = request
            .metadata
            .usage_source
            .clone()
            .unwrap_or_else(|| chat_usage_source_for_label(label));
        let operation = request
            .metadata
            .usage_operation
            .clone()
            .unwrap_or_else(|| operation_from_label(label));
        record_model_call(
            self.state,
            UsageRecordInput {
                provider: self.provider,
                model: &request.model,
                source: &source,
                operation: &operation,
                status: "error",
                status_code: crate::api::extract_status_code(error),
                usage: None,
                usage_source: "missing",
                started_at,
                duration_ms: duration.as_millis() as u64,
                conversation_id: request.metadata.conversation_id.clone(),
                message_id: request.metadata.message_id.clone(),
                error_kind: Some(error_kind_from_message(error)),
            },
        );
    }
}

fn anthropic_headers(api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).map_err(|err| format!("Invalid API key: {err}"))?,
    );
    headers.insert(
        "anthropic-version",
        HeaderValue::from_static(ANTHROPIC_VERSION),
    );
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    Ok(headers)
}

fn anthropic_messages_url(base_url: &str) -> String {
    format!("{}/messages", base_url.trim_end_matches('/'))
}

pub fn anthropic_messages_from_generate_request(request: &GenerateRequest) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for message in &request.messages {
        match message.role {
            ModelRole::User => messages.push(serde_json::json!({
                "role": "user",
                "content": anthropic_content_blocks(message, ModelRole::User),
            })),
            ModelRole::Assistant => messages.push(serde_json::json!({
                "role": "assistant",
                "content": anthropic_content_blocks(message, ModelRole::Assistant),
            })),
            ModelRole::Tool => messages.push(serde_json::json!({
                "role": "user",
                "content": anthropic_content_blocks(message, ModelRole::Tool),
            })),
        }
    }
    merge_consecutive_anthropic_roles(&mut messages);
    messages
}

pub fn anthropic_tools_from_model_tools(tools: &[ModelTool]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tool in tools {
        let name = tool.openai_tool_name();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        out.push(serde_json::json!({
            "name": name,
            "description": tool.description,
            "input_schema": normalize_anthropic_schema(tool.input_schema.clone()),
        }));
    }
    out
}

pub fn output_from_anthropic_message(
    value: &Value,
    label: &str,
) -> Result<GenerateOutput, ModelError> {
    if let Some(error) = value.get("error") {
        let msg = error
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("Unknown Anthropic error");
        return Err(ModelError::new(format!("{label}: {msg}")));
    }

    let parsed = parse_anthropic_response(value);
    let finish_reason = parsed.finish_reason.clone();
    let usage = anthropic_usage(value);
    let provider_message = openai_compatible_message(
        &parsed.content,
        parsed.reasoning.as_deref(),
        &parsed.tool_calls,
        Some(&finish_reason),
    );
    Ok(GenerateOutput {
        text: parsed.content,
        reasoning: parsed.reasoning,
        tool_calls: parsed.tool_calls,
        usage,
        finish_reason: Some(finish_reason),
        provider_messages: vec![provider_message],
        cancelled: false,
    })
}

struct AnthropicParsedResponse {
    content: String,
    reasoning: Option<String>,
    tool_calls: Vec<PendingToolCall>,
    finish_reason: String,
}

fn parse_anthropic_response(response: &Value) -> AnthropicParsedResponse {
    let mut content_parts = Vec::new();
    let mut reasoning_parts = Vec::new();
    let mut tool_calls = Vec::new();

    if let Some(blocks) = response.get("content").and_then(|value| value.as_array()) {
        for block in blocks {
            match block
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
            {
                "text" => {
                    if let Some(text) = block
                        .get("text")
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.is_empty())
                    {
                        content_parts.push(text.to_string());
                    }
                }
                "thinking" => {
                    if let Some(thinking) = block
                        .get("thinking")
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.is_empty())
                    {
                        reasoning_parts.push(thinking.to_string());
                    }
                }
                "tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    let arguments_raw = if input.is_null() {
                        "{}".to_string()
                    } else {
                        serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string())
                    };
                    tool_calls.push(PendingToolCall {
                        id,
                        function_name: name,
                        arguments: if input.is_null() {
                            serde_json::json!({})
                        } else {
                            input
                        },
                        arguments_raw,
                        arguments_parse_error: None,
                        signature: None,
                    });
                }
                _ => {}
            }
        }
    }

    let stop_reason = response
        .get("stop_reason")
        .and_then(|value| value.as_str())
        .unwrap_or("end_turn");

    AnthropicParsedResponse {
        content: content_parts.join("\n\n"),
        reasoning: if reasoning_parts.is_empty() {
            None
        } else {
            Some(reasoning_parts.join("\n\n"))
        },
        tool_calls,
        finish_reason: finish_reason_from_anthropic_stop_reason(stop_reason),
    }
}

enum AnthropicSseEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolUseStart {
        id: String,
        name: String,
    },
    ToolInputDelta(String),
    ContentBlockStop,
    MessageStop,
    MessageStopWithReason {
        reason: String,
        usage: Option<ModelUsage>,
    },
    Error(String),
}

fn parse_anthropic_sse_event(line: &str) -> Option<AnthropicSseEvent> {
    let line = line.trim();
    if !line.starts_with("data:") {
        return None;
    }
    let data = line.trim_start_matches("data:").trim();
    if data.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(data).ok()?;
    match value
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
    {
        "content_block_start" => {
            let block = value.get("content_block")?;
            if block.get("type").and_then(|value| value.as_str()) != Some("tool_use") {
                return None;
            }
            Some(AnthropicSseEvent::ToolUseStart {
                id: block
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "content_block_delta" => {
            let delta = value.get("delta")?;
            match delta
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
            {
                "text_delta" => delta
                    .get("text")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(|text| AnthropicSseEvent::TextDelta(text.to_string())),
                "thinking_delta" => delta
                    .get("thinking")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(|thinking| AnthropicSseEvent::ThinkingDelta(thinking.to_string())),
                "input_json_delta" => Some(AnthropicSseEvent::ToolInputDelta(
                    delta
                        .get("partial_json")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                )),
                _ => None,
            }
        }
        "content_block_stop" => Some(AnthropicSseEvent::ContentBlockStop),
        "message_delta" => {
            let reason = value
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(|value| value.as_str())
                .unwrap_or("end_turn")
                .to_string();
            Some(AnthropicSseEvent::MessageStopWithReason {
                reason,
                usage: model_usage_from_anthropic_value(&value),
            })
        }
        "message_stop" => Some(AnthropicSseEvent::MessageStop),
        "error" => Some(AnthropicSseEvent::Error(
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(|value| value.as_str())
                .unwrap_or("Unknown Anthropic error")
                .to_string(),
        )),
        _ => None,
    }
}

fn assemble_tool_call_from_stream(
    id: &str,
    name: &str,
    input_json_parts: &[String],
) -> PendingToolCall {
    let raw = input_json_parts.join("");
    let arguments_raw = if raw.trim().is_empty() {
        "{}".to_string()
    } else {
        raw
    };
    let (arguments, arguments_parse_error) = parse_tool_arguments(&arguments_raw);
    PendingToolCall {
        id: id.to_string(),
        function_name: name.to_string(),
        arguments,
        arguments_raw,
        arguments_parse_error,
        signature: None,
    }
}

fn anthropic_content_blocks(message: &ModelMessage, role: ModelRole) -> Vec<Value> {
    let mut blocks = Vec::new();
    for part in &message.content {
        match part {
            MessagePart::Text { text } => blocks.push(serde_json::json!({
                "type": "text",
                "text": text,
            })),
            MessagePart::Image { mime_type, data } => {
                if matches!(role, ModelRole::User) {
                    blocks.push(serde_json::json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": mime_type,
                            "data": data,
                        }
                    }));
                }
            }
            MessagePart::ImageUrl { url } => {
                if matches!(role, ModelRole::User) {
                    blocks.push(serde_json::json!({
                        "type": "image",
                        "source": { "type": "url", "url": url },
                    }));
                }
            }
            MessagePart::ToolCall {
                id,
                name,
                arguments,
                arguments_raw,
                ..
            } => {
                if matches!(role, ModelRole::Assistant) {
                    let input = if arguments.is_null() {
                        serde_json::from_str(arguments_raw).unwrap_or(Value::Null)
                    } else {
                        arguments.clone()
                    };
                    blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                }
            }
            MessagePart::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } => {
                blocks.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                    "is_error": is_error,
                }));
            }
            MessagePart::Reasoning { text } => {
                if matches!(role, ModelRole::Assistant) {
                    blocks.push(serde_json::json!({
                        "type": "thinking",
                        "thinking": text,
                    }));
                }
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": "",
        }));
    }
    blocks
}

fn merge_consecutive_anthropic_roles(messages: &mut Vec<Value>) {
    if messages.len() < 2 {
        return;
    }
    let mut i = 1;
    while i < messages.len() {
        let prev_role = messages[i - 1]
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let curr_role = messages[i]
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if prev_role == curr_role {
            let curr_content = messages[i]
                .get("content")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            if let Some(prev) = messages[i - 1]
                .get_mut("content")
                .and_then(|value| value.as_array_mut())
            {
                prev.extend(curr_content);
            }
            messages.remove(i);
        } else {
            i += 1;
        }
    }
}

fn normalize_anthropic_schema(schema: Value) -> Value {
    if let Some(any_of) = schema.get("anyOf").and_then(|value| value.as_array()) {
        if any_of.len() == 2 {
            let has_null = any_of
                .iter()
                .any(|item| item.get("type").and_then(|value| value.as_str()) == Some("null"));
            if has_null {
                if let Some(non_null) = any_of
                    .iter()
                    .find(|item| item.get("type").and_then(|value| value.as_str()) != Some("null"))
                {
                    let mut result = non_null.clone();
                    if let Some(description) = schema.get("description") {
                        result["description"] = description.clone();
                    }
                    return result;
                }
            }
        }
    }

    if schema.get("type").and_then(|value| value.as_str()) == Some("object") {
        let mut result = schema.clone();
        if let Some(obj) = result.as_object_mut() {
            // Anthropic/Bedrock reject oneOf/allOf/anyOf at the top level of a tool
            // input_schema (even alongside `type: object`). The constraints they encode
            // (e.g. present_artifacts' "require one of artifact_ids/paths") can't be
            // expressed here anyway — drop them; tool execution still validates.
            obj.remove("oneOf");
            obj.remove("allOf");
            obj.remove("anyOf");
            // Anthropic requires a properties map even when empty.
            obj.entry("properties").or_insert_with(|| serde_json::json!({}));
        }
        return result;
    }

    schema
}

fn anthropic_usage(value: &Value) -> Option<ModelUsage> {
    model_usage_from_anthropic_value(value)
}

fn openai_compatible_message(
    text: &str,
    reasoning: Option<&str>,
    tool_calls: &[PendingToolCall],
    finish_reason: Option<&str>,
) -> Value {
    let mut message = serde_json::json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { Value::String(text.to_string()) },
    });
    if let Some(reasoning) = reasoning.map(str::trim).filter(|value| !value.is_empty()) {
        message["reasoning_content"] = Value::String(reasoning.to_string());
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(
            tool_calls
                .iter()
                .map(|call| {
                    serde_json::json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.function_name,
                            "arguments": call.arguments_raw,
                        }
                    })
                })
                .collect(),
        );
    }
    if let Some(finish_reason) = finish_reason {
        message["finish_reason"] = Value::String(finish_reason.to_string());
    }
    message
}

fn stream_output(
    text: String,
    reasoning: String,
    tool_calls: Vec<PendingToolCall>,
    finish_reason: String,
    usage: Option<ModelUsage>,
) -> GenerateOutput {
    let reasoning = non_empty(reasoning);
    let provider_message = openai_compatible_message(
        &text,
        reasoning.as_deref(),
        &tool_calls,
        Some(&finish_reason),
    );
    GenerateOutput {
        text,
        reasoning,
        tool_calls,
        usage,
        finish_reason: Some(finish_reason),
        provider_messages: vec![provider_message],
        cancelled: false,
    }
}

fn finish_reason_from_anthropic_stop_reason(reason: &str) -> String {
    match reason {
        "end_turn" => "stop",
        "tool_use" => "tool_calls",
        "max_tokens" => "length",
        _ => "stop",
    }
    .to_string()
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

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::model::GenerateOptions;
    use crate::settings::ModelInfo;

    #[test]
    fn strips_top_level_combinators_from_object_schema() {
        // present_artifacts-style schema: object + top-level anyOf. Anthropic rejects
        // the combinator; normalize must drop it and keep the object usable.
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "paths": { "type": "array" } },
            "anyOf": [{ "required": ["artifact_ids"] }, { "required": ["paths"] }],
            "additionalProperties": false
        });
        let out = normalize_anthropic_schema(schema);
        assert!(out.get("anyOf").is_none(), "top-level anyOf must be dropped");
        assert!(out.get("oneOf").is_none());
        assert!(out.get("allOf").is_none());
        assert!(out.get("properties").is_some());
        assert_eq!(out["additionalProperties"], serde_json::json!(false));
    }

    #[test]
    fn object_without_properties_gets_empty_map() {
        let out = normalize_anthropic_schema(serde_json::json!({ "type": "object" }));
        assert_eq!(out["properties"], serde_json::json!({}));
    }

    #[test]
    fn tool_result_and_following_image_user_merge_into_one_turn() {
        // `read` appends an image as a separate user message after the tool
        // result. Both Tool and User roles serialize to Anthropic "user", so
        // the merge must fold tool_result + image into a single user turn.
        let mut messages = vec![
            serde_json::json!({
                "role": "user",
                "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "read it" }]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" }
                }]
            }),
        ];
        merge_consecutive_anthropic_roles(&mut messages);

        assert_eq!(messages.len(), 1, "consecutive user turns must merge");
        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[1]["type"], "image");
    }

    /// Build a real Anthropic request body via the production `request_body`
    /// path and assert how `thinking_level` maps to the wire.
    fn build_anthropic_body(
        thinking_level: Option<&str>,
        provider_temperature: Option<f64>,
        request_temperature: Option<f64>,
    ) -> Value {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut model_overrides = std::collections::HashMap::new();
        if let Some(temperature) = provider_temperature {
            model_overrides.insert(
                "claude-opus-4-8".into(),
                ModelInfo {
                    temperature: Some(temperature),
                    ..ModelInfo::default()
                },
            );
        }
        let provider = crate::settings::ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://api.anthropic.com".into(),
            available_models: vec!["claude-opus-4-8".into()],
            enabled_models: vec!["claude-opus-4-8".into()],
            enabled: true,
            api_format: "anthropic_messages".into(),
            model_overrides,
            compress_request_body: false,
        };
        let adapter = AnthropicMessagesProvider::new(&state, &provider, 1);
        let request = GenerateRequest {
            model: "claude-opus-4-8".into(),
            system: "sys".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions {
                temperature: request_temperature,
                thinking_level: thinking_level.map(|s| s.to_string()),
                ..Default::default()
            },
            metadata: Default::default(),
        };
        adapter.request_body(&request, false)
    }

    #[test]
    fn thinking_level_maps_to_adaptive_effort() {
        // 未设等级 → 不发 thinking / output_config（与改动前一致：Anthropic 默认不思考）。
        let none = build_anthropic_body(None, None, None);
        assert!(none.get("thinking").is_none(), "body: {none}");
        assert!(none.get("output_config").is_none(), "body: {none}");

        // 选了等级 → adaptive thinking + output_config.effort（4.6+ 正确写法，非 budget_tokens）。
        let high = build_anthropic_body(Some("high"), None, None);
        eprintln!("[anthropic effort=high] {high}");
        assert_eq!(high["thinking"]["type"], "adaptive");
        assert_eq!(high["output_config"]["effort"], "high");
        assert!(
            high.get("budget_tokens").is_none(),
            "must not send budget_tokens"
        );
    }

    #[test]
    fn request_body_temperature_is_optional_and_model_scoped() {
        let default_body = build_anthropic_body(None, None, None);
        assert!(default_body.get("temperature").is_none());

        let configured_body = build_anthropic_body(None, Some(0.4), None);
        assert_eq!(configured_body["temperature"], serde_json::json!(0.4));

        let explicit_body = build_anthropic_body(None, Some(0.4), Some(1.2));
        assert_eq!(explicit_body["temperature"], serde_json::json!(1.2));
    }

    #[test]
    fn canonical_text_image_and_tool_result_become_content_blocks() {
        let request = GenerateRequest {
            model: "claude".to_string(),
            system: "system".to_string(),
            messages: vec![
                ModelMessage {
                    role: ModelRole::User,
                    content: vec![
                        MessagePart::Text {
                            text: "Look".to_string(),
                        },
                        MessagePart::Image {
                            mime_type: "image/png".to_string(),
                            data: "abc".to_string(),
                        },
                    ],
                },
                ModelMessage {
                    role: ModelRole::Tool,
                    content: vec![MessagePart::ToolResult {
                        tool_call_id: "toolu_1".to_string(),
                        content: "done".to_string(),
                        is_error: false,
                        artifacts: Vec::new(),
                    }],
                },
            ],
            tools: Vec::new(),
            options: Default::default(),
            metadata: Default::default(),
        };

        let messages = anthropic_messages_from_generate_request(&request);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let content = messages[0]["content"].as_array().expect("content");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[2]["type"], "tool_result");
    }

    #[test]
    fn canonical_assistant_tool_call_becomes_tool_use() {
        let request = GenerateRequest {
            model: "claude".to_string(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::Assistant,
                content: vec![MessagePart::ToolCall {
                    id: "toolu_1".to_string(),
                    name: "web_search".to_string(),
                    arguments: serde_json::json!({"query": "rust"}),
                    arguments_raw: "{\"query\":\"rust\"}".to_string(),
                    signature: None,
                }],
            }],
            tools: Vec::new(),
            options: Default::default(),
            metadata: Default::default(),
        };

        let messages = anthropic_messages_from_generate_request(&request);

        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert_eq!(messages[0]["content"][0]["input"]["query"], "rust");
    }

    #[test]
    fn parses_anthropic_output_to_generate_output() {
        let response = serde_json::json!({
            "content": [
                {"type": "thinking", "thinking": "Plan"},
                {"type": "text", "text": "Answer"},
                {"type": "tool_use", "id": "toolu_1", "name": "web_search", "input": {"query": "kivio"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 7, "output_tokens": 11}
        });

        let output = output_from_anthropic_message(&response, "test").expect("output");

        assert_eq!(output.text, "Answer");
        assert_eq!(output.reasoning.as_deref(), Some("Plan"));
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            output.usage.as_ref().and_then(|usage| usage.total_tokens),
            Some(18)
        );
    }

    #[test]
    fn parses_anthropic_stream_text_reasoning_and_tool_use() {
        let events = [
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Plan\"}}",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}",
            "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_123\",\"name\":\"web_search\",\"input\":{}}}",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\"\"}}",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\":\\\"kivio\\\"}\"}}",
            "data: {\"type\":\"content_block_stop\"}",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}",
        ];

        assert!(matches!(
            parse_anthropic_sse_event(events[0]),
            Some(AnthropicSseEvent::ThinkingDelta(delta)) if delta == "Plan"
        ));
        assert!(matches!(
            parse_anthropic_sse_event(events[1]),
            Some(AnthropicSseEvent::TextDelta(delta)) if delta == "Hello"
        ));
        assert!(matches!(
            parse_anthropic_sse_event(events[2]),
            Some(AnthropicSseEvent::ToolUseStart { id, name })
                if id == "toolu_123" && name == "web_search"
        ));
        assert!(matches!(
            parse_anthropic_sse_event(events[5]),
            Some(AnthropicSseEvent::ContentBlockStop)
        ));
        assert!(matches!(
            parse_anthropic_sse_event(events[6]),
            Some(AnthropicSseEvent::MessageStopWithReason { reason, .. }) if reason == "tool_use"
        ));

        let input_parts = events[3..=4]
            .iter()
            .filter_map(|event| match parse_anthropic_sse_event(event) {
                Some(AnthropicSseEvent::ToolInputDelta(delta)) => Some(delta),
                _ => None,
            })
            .collect::<Vec<_>>();
        let call = assemble_tool_call_from_stream("toolu_123", "web_search", &input_parts);

        assert_eq!(call.function_name, "web_search");
        assert_eq!(call.arguments["query"], "kivio");
        assert!(call.arguments_parse_error.is_none());
    }
}
