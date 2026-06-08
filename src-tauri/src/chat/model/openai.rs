use reqwest::header::ACCEPT_ENCODING;
use serde_json::Value;

use crate::api::send_with_failover;
use crate::settings::ModelProvider;
use crate::state::AppState;
use crate::usage::{
    chat_usage_source_for_label, error_kind_from_message, model_usage_from_openai_value,
    model_usage_from_stream_value, operation_from_label, record_model_call, UsageRecordInput,
};
use crate::utils;

use super::{
    openai_messages_from_generate_request, pending_tool_calls_from_openai_message,
    stream_read_error, GenerateOutput, GenerateRequest, LanguageModelProvider, ModelError,
    ModelFuture, ModelUsage, PendingToolCall, ProviderCapabilities, StreamPart, StreamSink,
};

pub struct OpenAiChatProvider<'a> {
    state: &'a AppState,
    provider: &'a ModelProvider,
    retry_attempts: usize,
}

impl<'a> OpenAiChatProvider<'a> {
    pub fn new(state: &'a AppState, provider: &'a ModelProvider, retry_attempts: usize) -> Self {
        Self {
            state,
            provider,
            retry_attempts,
        }
    }
}

impl LanguageModelProvider for OpenAiChatProvider<'_> {
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

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calling: self.provider.supports_tools,
            vision: true,
            streaming: true,
            reasoning: true,
        }
    }
}

impl OpenAiChatProvider<'_> {
    async fn generate_inner(&self, request: GenerateRequest) -> Result<GenerateOutput, ModelError> {
        let label = request_label(&request, "Chat API");
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
                self.state
                    .http
                    .post(self.chat_completions_url())
                    .bearer_auth(key)
                    .header(ACCEPT_ENCODING, "identity")
                    .json(&body)
                    .send()
            },
        )
        .await
        .map_err(|err| {
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &err);
            ModelError::new(err)
        })?;

        let raw = response.text().await.map_err(|err| {
            let message = format!("{label} read body: {err}");
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &message);
            ModelError::new(message)
        })?;
        let value: Value = serde_json::from_str(&raw).map_err(|err| {
            let message = format!(
                "{label} parse JSON: {} (body: {})",
                err,
                raw.chars().take(500).collect::<String>()
            );
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &message);
            ModelError::new(message)
        })?;
        let output = output_from_chat_completion(&value, &raw, &label)?;
        self.record_usage_success(
            &request,
            &label,
            started_at,
            started.elapsed(),
            output.usage.clone(),
        );
        Ok(output)
    }

    async fn stream_inner(
        &self,
        request: GenerateRequest,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<GenerateOutput, ModelError> {
        let label = request_label(&request, "Chat stream");
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
                self.state
                    .http
                    .post(self.chat_completions_url())
                    .bearer_auth(key)
                    .header(ACCEPT_ENCODING, "identity")
                    .json(&body)
                    .send()
            },
        )
        .await
        .map_err(|err| {
            self.record_usage_failure(&request, &label, started_at, started.elapsed(), &err);
            ModelError::new(err)
        })?;

        let mut buffer = String::new();
        let mut full = String::new();
        let mut reasoning_full = String::new();
        let mut tool_partials: Vec<PartialToolCall> = Vec::new();
        let mut finish_reason: Option<String> = None;
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
                model_error
            })?;
            let Some(chunk) = chunk else {
                break;
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buffer.find('\n') {
                let line: String = buffer.drain(..=pos).collect();
                let line = line.trim();
                if !line.starts_with("data:") {
                    continue;
                }
                let data = line.trim_start_matches("data:").trim();
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    let tool_calls = finish_tool_call_partials(&mut tool_partials, sink)?;
                    let reason = finish_reason.unwrap_or_else(|| "done".to_string());
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
                        provider_messages: Vec::new(),
                        cancelled: false,
                    };
                    self.record_usage_success(
                        &request,
                        &label,
                        started_at,
                        started.elapsed(),
                        output.usage.clone(),
                    );
                    return Ok(output);
                }
                let value: Value = match serde_json::from_str(data) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if let Some(next_usage) = model_usage_from_stream_value(&value) {
                    usage = Some(next_usage);
                }
                if let Some(reason) = openai_stream_finish_reason(&value) {
                    finish_reason = Some(reason.to_string());
                }
                handle_openai_stream_tool_calls(&value, &mut tool_partials, sink)?;
                if let Some((reasoning, mode)) = extract_chat_stream_reasoning(&value) {
                    if let Some(delta) =
                        append_chat_stream_text(&mut reasoning_full, reasoning, mode)
                    {
                        sink.emit(StreamPart::ReasoningDelta { delta })?;
                    }
                }
                if let Some((content, mode)) = extract_chat_stream_text(&value) {
                    if let Some(delta) = append_chat_stream_text(&mut full, content, mode) {
                        sink.emit(StreamPart::TextDelta { delta })?;
                    }
                }
            }
        }

        let tool_calls = finish_tool_call_partials(&mut tool_partials, sink)?;
        let reason = finish_reason.unwrap_or_else(|| "done".to_string());
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
            provider_messages: Vec::new(),
            cancelled: false,
        };
        self.record_usage_success(
            &request,
            &label,
            started_at,
            started.elapsed(),
            output.usage.clone(),
        );
        Ok(output)
    }

    fn chat_completions_url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.provider.base_url.trim_end_matches('/')
        )
    }

    fn request_body(&self, request: &GenerateRequest, stream: bool) -> Value {
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": openai_messages_from_generate_request(request),
            "temperature": request.options.temperature,
            "max_tokens": request.options.max_tokens,
        });
        if stream {
            body["stream"] = Value::Bool(true);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| tool.to_openai_tool())
                    .collect(),
            );
        }
        if !request.options.thinking_enabled
            && utils::provider_supports_thinking_field(&self.provider.base_url)
        {
            body["thinking"] = serde_json::json!({ "type": "disabled" });
        }
        if let Some(overrides) = request.options.provider_options.as_object() {
            for (key, value) in overrides {
                body[key] = value.clone();
            }
        }
        body
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

pub fn output_from_chat_completion(
    value: &Value,
    raw: &str,
    label: &str,
) -> Result<GenerateOutput, ModelError> {
    let choice = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .ok_or_else(|| invalid_response(label, raw))?;
    let message = choice
        .get("message")
        .cloned()
        .ok_or_else(|| invalid_response(label, raw))?;
    let text = assistant_text_from_openai_message(&message);
    let reasoning = reasoning_from_openai_message(&message);
    let tool_calls = pending_tool_calls_from_openai_message(&message);
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let usage = model_usage_from_openai_response(value);

    Ok(GenerateOutput {
        text,
        reasoning,
        tool_calls,
        usage,
        finish_reason,
        provider_messages: vec![message],
        cancelled: false,
    })
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
    message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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

fn extract_chat_stream_reasoning(value: &Value) -> Option<(&str, ChatStreamTextMode)> {
    let choice = value.get("choices").and_then(|choices| choices.get(0))?;

    if let Some(reasoning) = choice
        .get("delta")
        .and_then(|delta| {
            delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
        })
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
    {
        return Some((reasoning, ChatStreamTextMode::Delta));
    }

    choice
        .get("message")
        .and_then(|message| {
            message
                .get("reasoning_content")
                .or_else(|| message.get("reasoning"))
        })
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
        .map(|content| (content, ChatStreamTextMode::Snapshot))
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
            .unwrap_or(partials.len() as u64) as usize;
        while partials.len() <= index {
            partials.push(PartialToolCall::default());
        }
        let partial = &mut partials[index];
        if let Some(id) = call.get("id").and_then(|value| value.as_str()) {
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
        if !partial.started {
            if let (Some(id), Some(name)) = (partial.id.as_deref(), partial.name.as_deref()) {
                sink.emit(StreamPart::ToolCallStart {
                    id: id.to_string(),
                    name: name.to_string(),
                })?;
                partial.started = true;
            }
        }
        if let Some(delta) = call
            .get("function")
            .and_then(|function| function.get("arguments"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        {
            partial.arguments_raw.push_str(delta);
            if let Some(id) = partial.id.as_deref() {
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
        let id = partial.id.unwrap_or_else(|| format!("tool_{index}"));
        let raw = if partial.arguments_raw.trim().is_empty() {
            "{}".to_string()
        } else {
            partial.arguments_raw
        };
        let (arguments, arguments_parse_error) = super::parse_tool_arguments(&raw);
        let call = PendingToolCall {
            id,
            function_name: name,
            arguments,
            arguments_raw: raw,
            arguments_parse_error,
        };
        sink.emit(StreamPart::ToolCallDone { call: call.clone() })?;
        calls.push(call);
    }
    Ok(calls)
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(output.finish_reason.as_deref(), Some("tool_calls"));
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
}
