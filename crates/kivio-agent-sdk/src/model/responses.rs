//! Shared Responses wire protocol and host-configured HTTP provider.
pub mod chatgpt_plan;

use serde_json::Value;

use super::error::provider_error_code;
use super::usage::model_usage_from_openai_value;
use super::{
    parse_tool_arguments, responses_input_with_replay, stream_read_error, BuiltinWebSearch,
    FinishKind, FinishReason, GenerateOutput, GenerateRequest, GeneratedImageData,
    LanguageModelProvider, MessagePart, ModelError, ModelErrorKind, ModelFuture, ModelUsage,
    PendingToolCall, ProviderReasoningItem, StreamPart, StreamSink, WebCitation,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesFormat {
    #[default]
    OpenAi,
    Xai,
}

/// Transport identity of a reasoning item. Stored outside the opaque API item.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResponsesReasoningSource {
    format: ResponsesFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
}

impl ResponsesReasoningSource {
    fn new(
        format: ResponsesFormat,
        provider_id: Option<&str>,
        endpoint: Option<&reqwest::Url>,
    ) -> Option<Self> {
        let provider_id = provider_id.filter(|id| !id.is_empty()).map(str::to_string);
        let endpoint = endpoint.map(|url| {
            // Keep URL credentials and query parameters out of persisted history.
            format!("{}{}", url.origin().ascii_serialization(), url.path())
        });
        (provider_id.is_some() || endpoint.is_some()).then_some(Self {
            format,
            provider_id,
            endpoint,
        })
    }
}

/// Host-resolved wire defaults. Encoded call options override extra_body, then
/// per-call provider_options apply; ToolChoice::None has final precedence.
#[derive(Clone, Debug)]
pub struct OpenAiResponsesOptions {
    pub format: ResponsesFormat,
    pub temperature: Option<f64>,
    pub replay_reasoning: bool,
    pub prompt_cache_key: Option<String>,
    pub prompt_cache_retention: Option<String>,
    pub endpoint: Option<reqwest::Url>,
    /// Host provider identifier. Distinguishes configurations sharing an endpoint.
    pub provider_id: Option<String>,
    pub cache_retention: Option<super::openai::CacheRetention>,
    pub extra_body: Value,
}

impl Default for OpenAiResponsesOptions {
    fn default() -> Self {
        Self {
            format: ResponsesFormat::OpenAi,
            temperature: None,
            replay_reasoning: true,
            prompt_cache_key: None,
            prompt_cache_retention: None,
            endpoint: None,
            provider_id: None,
            cache_retention: None,
            extra_body: Value::Null,
        }
    }
}

/// Explicit transport configuration with a complete Responses endpoint URL.
/// Credentials are supplied by the host, never read from disk or the environment.
pub struct OpenAiResponsesConfig {
    pub endpoint: reqwest::Url,
    pub api_key: Option<String>,
    pub headers: reqwest::header::HeaderMap,
    pub options: OpenAiResponsesOptions,
    /// Applies to buffered generation. Configure stream idle timeouts on the client.
    pub request_timeout: Option<std::time::Duration>,
}

impl OpenAiResponsesConfig {
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

/// Sends once per invocation without SDK retry, key rotation, or cache fallback.
/// Dropping the generation future cancels its HTTP read.
pub struct OpenAiResponsesProvider {
    client: reqwest::Client,
    config: OpenAiResponsesConfig,
    chatgpt_plan: bool,
}

impl OpenAiResponsesProvider {
    pub fn new(client: reqwest::Client, config: OpenAiResponsesConfig) -> Self {
        Self {
            client,
            config,
            chatgpt_plan: false,
        }
    }

    /// Uses the official ChatGPT plan route with a host-authorized access token.
    pub fn new_chatgpt_plan(client: reqwest::Client, access_token: String) -> Self {
        let mut config =
            OpenAiResponsesConfig::new("https://api.openai.com/v1/responses".parse().unwrap());
        config.api_key = Some(access_token);
        Self {
            client,
            config,
            chatgpt_plan: true,
        }
    }

    fn bind_output_provenance(&self, output: &mut GenerateOutput) {
        bind_output_provenance(
            output,
            self.config.options.format,
            self.config.options.provider_id.as_deref(),
            Some(&self.config.endpoint),
        );
    }

    async fn post(
        &self,
        request: &GenerateRequest,
        stream: bool,
    ) -> Result<reqwest::Response, ModelError> {
        if request.messages.iter().any(|message| {
            message
                .content
                .iter()
                .any(|part| matches!(part, MessagePart::Video { .. }))
        }) {
            return Err(ModelError::with_kind(
                "Responses does not support video input",
                ModelErrorKind::InvalidRequest,
            ));
        }
        let options = OpenAiResponsesOptions {
            endpoint: Some(self.config.endpoint.clone()),
            temperature: request
                .options
                .temperature
                .or(self.config.options.temperature),
            ..self.config.options.clone()
        };
        let mut body = request_body(request, stream, &options);
        if self.chatgpt_plan {
            chatgpt_plan::prepare_body(&mut body)?;
        }
        let mut http = self
            .client
            .post(self.config.endpoint.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .headers(self.config.headers.clone())
            .json(&body);
        if self.chatgpt_plan {
            http = http.header(reqwest::header::ACCEPT, "text/event-stream");
        }
        if let Some(key) = &self.config.api_key {
            http = http.bearer_auth(key);
        }
        if !stream {
            if let Some(timeout) = self.config.request_timeout {
                http = http.timeout(timeout);
            }
        }
        super::transport::send(http, &request_label(request, "Responses API")).await
    }
}

impl LanguageModelProvider for OpenAiResponsesProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            if self.chatgpt_plan {
                return self.stream(request, &mut DiscardSink).await;
            }
            let label = request_label(&request, "Responses API");
            let response = self.post(&request, false).await?;
            let mut output = read_responses_body(response, &label, &request.model).await?;
            self.bind_output_provenance(&mut output);
            Ok(output)
        })
    }

    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            let label = request_label(&request, "Responses stream");
            let buffered = !self.chatgpt_plan
                && uses_buffered_delivery(
                    Some(&self.config.endpoint),
                    self.config.options.format,
                    &request.model,
                );
            let response = self.post(&request, !buffered).await?;
            let mut output = if buffered {
                decode_responses_buffered_stream(response, sink, &label, &request.model).await?
            } else {
                decode_stream(response, sink, &label, &request.model, self.chatgpt_plan).await?
            };
            self.bind_output_provenance(&mut output);
            Ok(output)
        })
    }
}

/// Bind new reasoning replay data from the raw decoders to the provider that returned it.
/// Match the format, provider ID and endpoint supplied to the request encoder.
pub fn bind_output_provenance(
    output: &mut GenerateOutput,
    format: ResponsesFormat,
    provider_id: Option<&str>,
    endpoint: Option<&reqwest::Url>,
) {
    let deepseek = format == ResponsesFormat::OpenAi && is_official_deepseek(endpoint);
    let had_plaintext_reasoning = output
        .reasoning_items
        .iter()
        .any(|item| !has_encrypted_reasoning(&item.item));
    if had_plaintext_reasoning && !deepseek {
        output
            .reasoning_items
            .retain(|item| has_encrypted_reasoning(&item.item));
    }
    let source = ResponsesReasoningSource::new(format, provider_id, endpoint);
    for item in &mut output.reasoning_items {
        item.source.clone_from(&source);
    }
    if let Some(replay) = &mut output.canonical_message {
        if let Some(MessagePart::XaiOutput { source: origin, .. }) = replay.content.first_mut() {
            if format == ResponsesFormat::Xai && source.is_some() {
                *origin = source;
            } else {
                output.canonical_message = None;
            }
        }
    }
    if (deepseek || had_plaintext_reasoning) && output.reasoning.is_some() {
        // Keep the UI reasoning separate from unscoped Chat reasoning history.
        let mut replay = output.replay_message();
        replay
            .content
            .retain(|part| !matches!(part, MessagePart::Reasoning { .. }));
        output.canonical_message = Some(replay);
    }
}

fn is_official_deepseek(endpoint: Option<&reqwest::Url>) -> bool {
    endpoint.is_some_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.deepseek.com")
            && url.port().is_none()
    })
}

impl crate::agent::provider_runtime::ModelRuntime for OpenAiResponsesProvider {
    fn generate<'a>(
        &'a self,
        call: crate::agent::provider_runtime::ModelCall,
        sink: &'a mut dyn crate::agent::provider_runtime::ModelOutputSink,
    ) -> crate::agent::provider_runtime::ProviderFuture<'a, Result<GenerateOutput, ModelError>>
    {
        Box::pin(crate::agent::provider_runtime::generate_with_provider(
            self, call, sink,
        ))
    }
}

/// OpenAI documents these models as Responses-only with no streaming transport.
pub fn uses_buffered_delivery(
    endpoint: Option<&reqwest::Url>,
    format: ResponsesFormat,
    model: &str,
) -> bool {
    format == ResponsesFormat::OpenAi
        && super::openai::is_official_openai(endpoint)
        && matches!(
            model,
            "o1-pro" | "o1-pro-2025-03-19" | "o3-pro" | "o3-pro-2025-06-10"
        )
}

/// Decode a successful JSON or SSE-only response and bind replay data to its model.
pub fn decode_responses_body(
    raw: &str,
    label: &str,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    decode_responses_body_at_endpoint(raw, label, model, None)
}

/// Decode DeepSeek plaintext reasoning only for its official response endpoint.
/// Hosts bind the resulting replay data with [`bind_output_provenance`].
pub fn decode_responses_body_at_endpoint(
    raw: &str,
    label: &str,
    model: &str,
    endpoint: Option<&reqwest::Url>,
) -> Result<GenerateOutput, ModelError> {
    let deepseek = is_official_deepseek(endpoint);
    let mut output = match serde_json::from_str::<Value>(raw) {
        Ok(value) => {
            let mut output = output_from_responses_payload(&value, raw, label, deepseek)?;
            if is_stateless_grok(model) {
                if let Some(items) = value["output"].as_array() {
                    preserve_xai_output(&mut output, model, items.iter());
                }
            }
            output
        }
        Err(error) if !looks_like_sse(raw) => {
            return Err(ModelError::new(format!(
                "{label} parse JSON: {error} (body: {})",
                raw.chars().take(500).collect::<String>()
            )));
        }
        Err(_) => output_from_sse_body(raw, deepseek, model)?,
    };
    stamp_reasoning_item_model(&mut output, model);
    Ok(output)
}

async fn read_responses_body(
    response: reqwest::Response,
    label: &str,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    let endpoint = response.url().clone();
    let raw = response
        .text()
        .await
        .map_err(|error| stream_read_error(label, &error))?;
    decode_responses_body_at_endpoint(&raw, label, model, Some(&endpoint))
}

/// Deliver a buffered response to clients using the streaming interface.
pub async fn decode_responses_buffered_stream(
    response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    let output = read_responses_body(response, label, model).await?;
    if let Some(reasoning) = &output.reasoning {
        sink.emit(StreamPart::ReasoningDelta {
            delta: reasoning.clone(),
        })?;
    }
    if let Some(search) = &output.web_search {
        sink.emit(StreamPart::WebSearch {
            queries: search.queries.clone(),
            citations: search.citations.clone(),
        })?;
    }
    if !output.text.is_empty() {
        sink.emit(StreamPart::TextDelta {
            delta: output.text.clone(),
        })?;
    }
    for image in &output.images {
        sink.emit(StreamPart::ImageData {
            mime_type: image.mime_type.clone(),
            data: image.data.clone(),
        })?;
    }
    for call in &output.tool_calls {
        sink.emit(StreamPart::ToolCallStart {
            id: call.id.clone(),
            name: call.function_name.clone(),
        })?;
        sink.emit(StreamPart::ToolCallDelta {
            id: call.id.clone(),
            delta: call.arguments_raw.clone(),
        })?;
        sink.emit(StreamPart::ToolCallDone { call: call.clone() })?;
    }
    if let Some(usage) = &output.usage {
        if let Some(input_tokens) = usage.input_tokens {
            sink.emit(StreamPart::ContextUsage {
                input_tokens,
                output_tokens: usage.output_tokens.unwrap_or(0),
            })?;
        }
    }
    if let Some(reason) = &output.finish_reason {
        sink.emit(StreamPart::Finish {
            reason: reason.clone(),
            full: output.text.clone(),
        })?;
    }
    Ok(output)
}

/// Completion commits the response even when a proxy keeps the HTTP body open.
pub async fn decode_responses_stream(
    response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    decode_stream(response, sink, label, model, false).await
}

async fn decode_stream(
    mut response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
    model: &str,
    chatgpt_plan: bool,
) -> Result<GenerateOutput, ModelError> {
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut sse = super::sse::SseDecoder::default();
    let mut state = ResponsesStreamState {
        model: model.to_string(),
        deepseek: is_official_deepseek(Some(response.url())),
        chatgpt_plan,
        ..Default::default()
    };
    'stream: loop {
        let chunk = response.chunk().await.map_err(|error| {
            stream_read_error(label, &error).with_request_id(request_id.as_deref())
        })?;
        let Some(chunk) = chunk else {
            break;
        };
        for data in sse
            .push(&chunk)
            .map_err(|e| e.with_request_id(request_id.as_deref()))?
        {
            if let Some(error) = process_sse_data(&data, &mut state, sink)
                .map_err(|e| e.with_request_id(request_id.as_deref()))?
            {
                return Err(error.with_request_id(request_id.as_deref()));
            }
            if state.finish_reason.is_some() {
                break 'stream;
            }
        }
    }
    if state.finish_reason.is_none() {
        return Err(ModelError::stream_ended_early(label).with_request_id(request_id.as_deref()));
    }
    state.finish(sink)
}

fn xai_reasoning_effort(model: &str, effort: &str) -> Option<&'static str> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "none" | "off"
            if matches!(
                model.strip_suffix("-latest").unwrap_or(model),
                "grok-4.5" | "grok-4.6" | "grok-4.7"
            ) =>
        {
            Some("low")
        }
        "none" | "off" => Some("none"),
        "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" | "max" => Some("high"),
        "xhigh" => Some("xhigh"),
        _ => None,
    }
}

/// grok 的 `include`：按已启用的服务端工具列出要回传的字段，不显式 include 的话
/// 搜索来源和执行输出根本不会回来。没有服务端工具时返回空（调用方据此不发该字段）。
fn xai_include_values(tools: Option<&Value>) -> Vec<Value> {
    let mut values: Vec<Value> = Vec::new();
    let Some(tools) = tools.and_then(Value::as_array) else {
        return values;
    };
    for tool in tools {
        let extra = match tool.get("type").and_then(Value::as_str).map(str::trim) {
            Some("web_search") => "web_search_call.action.sources",
            Some("file_search") => "file_search_call.results",
            Some("code_interpreter") => "code_interpreter_call.outputs",
            _ => continue,
        };
        let extra = Value::String(extra.to_string());
        if !values.contains(&extra) {
            values.push(extra);
        }
    }
    values
}

/// Adapt Chat JSON options when a host reroutes a request to OpenAI Responses.
/// The returned value is the Responses model default body. Per-call options
/// are translated in the request and retain precedence over those defaults.
pub fn translate_chat_route_options(
    request: &mut GenerateRequest,
    extra_body: &Value,
) -> Result<Value, ModelError> {
    let tools_enabled = request.options.tool_choice != super::ToolChoice::None;
    let call_fields = request
        .options
        .provider_options
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut default_fields = extra_body.as_object().cloned().unwrap_or_default();
    default_fields.retain(|key, _| !call_fields.contains_key(key));
    let mut defaults = translate_chat_fields(default_fields, tools_enabled)?;
    let mut overrides = translate_chat_fields(call_fields, tools_enabled)?;

    if request.options.provider_options.get("text").is_none() {
        if let (Some(default_text), Some(call_text)) = (
            defaults.get("text").and_then(Value::as_object),
            overrides.get_mut("text").and_then(Value::as_object_mut),
        ) {
            for (field, value) in default_text {
                call_text.entry(field).or_insert_with(|| value.clone());
            }
        }
    }
    // Chat's raw model defaults follow typed tools. Preserve that layering
    // when Responses places typed tools after its model defaults.
    for field in ["tools", "tool_choice"] {
        if let Some(value) = defaults.remove(field) {
            overrides.entry(field).or_insert(value);
        }
    }
    if request.options.max_tokens == 0
        && ["max_tokens", "max_completion_tokens", "max_output_tokens"]
            .iter()
            .any(|field| extra_body.get(field).is_some())
    {
        if let Some(value) = defaults.remove("max_output_tokens") {
            overrides.entry("max_output_tokens").or_insert(value);
        }
    }
    if request.options.thinking_enabled && request.options.thinking_level.is_none() {
        request.options.thinking_level = defaults
            .get("reasoning")
            .and_then(|reasoning| reasoning["effort"].as_str())
            .map(str::to_owned);
    }
    request.options.provider_options = Value::Object(overrides);
    Ok(Value::Object(defaults))
}

fn unsupported_chat_override(field: &str) -> ModelError {
    ModelError::with_kind(
        format!("OpenAI Responses cannot use the Chat override `{field}`. Remove it or use a Responses-compatible option."),
        ModelErrorKind::InvalidRequest,
    )
}

fn flatten_chat_member(value: &mut Value, member: &str, field: &str) -> Result<bool, ModelError> {
    let Some(object) = value.as_object_mut() else {
        return Ok(false);
    };
    let Some(nested) = object.remove(member) else {
        return Ok(false);
    };
    let Value::Object(nested) = nested else {
        return Err(unsupported_chat_override(field));
    };
    for (key, value) in nested {
        object.entry(key).or_insert(value);
    }
    Ok(true)
}

fn translate_chat_tool(tool: &mut Value, definition: bool) -> Result<(), ModelError> {
    match tool["type"].as_str() {
        Some("function") => {
            if flatten_chat_member(tool, "function", "tools.function")? && definition {
                tool.as_object_mut()
                    .unwrap()
                    .entry("strict")
                    .or_insert(Value::Bool(false));
            }
        }
        Some("custom") => {
            flatten_chat_member(tool, "custom", "tools.custom")?;
        }
        _ => {}
    }
    Ok(())
}

fn translate_chat_fields(
    mut fields: serde_json::Map<String, Value>,
    tools_enabled: bool,
) -> Result<serde_json::Map<String, Value>, ModelError> {
    if let Some(count) = fields.remove("n") {
        if !count.is_null() && count.as_u64() != Some(1) {
            return Err(unsupported_chat_override("n"));
        }
    }
    for field in [
        "audio",
        "frequency_penalty",
        "function_call",
        "functions",
        "logit_bias",
        "messages",
        "modalities",
        "prediction",
        "presence_penalty",
        "seed",
        "stop",
        "web_search_options",
    ] {
        if fields.remove(field).is_some_and(|value| !value.is_null()) {
            return Err(unsupported_chat_override(field));
        }
    }
    let modern_limit = fields.remove("max_completion_tokens");
    let legacy_limit = fields.remove("max_tokens");
    if let Some(limit) = modern_limit.or(legacy_limit) {
        fields.entry("max_output_tokens").or_insert(limit);
    }
    if let Some(effort) = fields.remove("reasoning_effort") {
        let reasoning = if effort.is_null() {
            Value::Null
        } else {
            serde_json::json!({ "effort": effort })
        };
        fields.entry("reasoning").or_insert(reasoning);
    }
    if let Some(options) = fields
        .get_mut("stream_options")
        .and_then(Value::as_object_mut)
    {
        options.remove("include_usage");
        if options.is_empty() {
            fields.remove("stream_options");
        }
    }
    let format = fields
        .remove("response_format")
        .filter(|value| !value.is_null());
    let verbosity = fields.remove("verbosity").filter(|value| !value.is_null());
    if format.is_some() || verbosity.is_some() {
        let text = fields
            .entry("text")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| unsupported_chat_override("response_format"))?;
        if let Some(mut format) = format {
            if format["type"] == "json_schema"
                && !flatten_chat_member(&mut format, "json_schema", "response_format")?
            {
                return Err(unsupported_chat_override("response_format"));
            }
            text.entry("format").or_insert(format);
        }
        if let Some(verbosity) = verbosity {
            text.entry("verbosity").or_insert(verbosity);
        }
    }
    if fields.remove("logprobs") == Some(Value::Bool(true)) {
        let include = fields
            .entry("include")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(|| unsupported_chat_override("logprobs"))?;
        if !include
            .iter()
            .any(|value| value == "message.output_text.logprobs")
        {
            include.push("message.output_text.logprobs".into());
        }
    }
    if tools_enabled {
        if let Some(tools) = fields.get_mut("tools").and_then(Value::as_array_mut) {
            for tool in tools {
                translate_chat_tool(tool, true)?;
            }
        }
        if let Some(choice) = fields.get_mut("tool_choice") {
            if flatten_chat_member(choice, "allowed_tools", "tool_choice")? {
                let tools = choice["tools"]
                    .as_array_mut()
                    .ok_or_else(|| unsupported_chat_override("tool_choice"))?;
                for tool in tools {
                    translate_chat_tool(tool, false)?;
                }
            } else {
                translate_chat_tool(choice, false)?;
            }
        }
    } else {
        fields.remove("tools");
        fields.remove("tool_choice");
    }
    Ok(fields)
}

pub fn request_body(
    request: &GenerateRequest,
    stream: bool,
    options: &OpenAiResponsesOptions,
) -> Value {
    let is_xai = options.format == ResponsesFormat::Xai;
    let official_openai = super::openai::is_official_openai(options.endpoint.as_ref()) && !is_xai;
    let deepseek = is_official_deepseek(options.endpoint.as_ref()) && !is_xai;
    // Grok 4.7 returns encrypted reasoning for stateless replay. Earlier xAI
    // models retain their existing behavior.
    let supports_replay = !is_xai || is_stateless_grok(&request.model);
    let source = ResponsesReasoningSource::new(
        options.format,
        options.provider_id.as_deref(),
        options.endpoint.as_ref(),
    );
    let mut input = responses_input_with_replay(&request.messages, |model, item_source| {
        options.replay_reasoning
            && supports_replay
            && model == request.model
            && match item_source {
                Some(item_source) => Some(item_source) == source.as_ref(),
                // Keep model-only replay for legacy OpenAI history.
                None => !is_xai && !deepseek,
            }
    });
    input.retain(|item| item["type"] != "reasoning" || reasoning_item_replayable(item, deepseek));
    if official_openai || deepseek {
        for (index, item) in input.iter_mut().enumerate() {
            let assistant = item["role"] == "assistant";
            if let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) {
                for part in parts {
                    match part["type"].as_str() {
                        Some("output_text") if assistant && official_openai => {
                            part["annotations"] = Value::Array(vec![]);
                        }
                        // A generated image needs its original image_generation_call item to
                        // replay as assistant output. Older history only has the attachment.
                        Some("input_image") if assistant => {
                            *part = serde_json::json!({
                                "type":"output_text",
                                "text":"[历史助手消息中的图片]",
                            });
                            if official_openai {
                                part["annotations"] = Value::Array(vec![]);
                            }
                        }
                        Some("input_image") if official_openai && part.get("detail").is_none() => {
                            part["detail"] = "auto".into();
                        }
                        _ => {}
                    }
                }
                if assistant && official_openai {
                    item["type"] = "message".into();
                    item["id"] = format!("msg_kivio_{index}").into();
                    item["status"] = "completed".into();
                }
            }
        }
    }
    let mut body = serde_json::json!({
        "model": request.model,
        "input": input,
        "store": false,
    });
    super::apply_body_options(&mut body, &options.extra_body, stream && !is_xai);
    if let Some(temperature) = options.temperature {
        body["temperature"] = serde_json::json!(temperature);
    }
    if !request.system.trim().is_empty() {
        if is_xai {
            if let Some(input) = body["input"].as_array_mut() {
                input.insert(
                    0,
                    serde_json::json!({
                        "role": "system",
                        "content": [{ "type": "input_text", "text": request.system }],
                    }),
                );
            }
        } else {
            body["instructions"] = Value::String(request.system.clone());
        }
    }
    if request.options.max_tokens > 0 {
        body["max_output_tokens"] = Value::from(request.options.max_tokens);
    } else {
        body.as_object_mut().unwrap().remove("max_output_tokens");
    }
    if stream {
        body["stream"] = Value::Bool(true);
    }
    let mut tools: Vec<Value> = request
        .tools
        .iter()
        .map(|tool| {
            let mut tool = tool.to_openai_responses_tool();
            if official_openai {
                // Preserve the tool schema's optional properties on Responses.
                tool["strict"] = Value::Bool(false);
            }
            tool
        })
        .collect();
    if request.options.builtin_web_search {
        tools.push(serde_json::json!({ "type": "web_search" }));
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = Value::String("auto".to_string());
    }
    let effort = if request.options.thinking_enabled {
        request.options.thinking_level.as_deref()
    } else {
        Some("none")
    };
    if is_xai {
        if let Some(mapped) = effort.and_then(|level| xai_reasoning_effort(&request.model, level)) {
            body["reasoning"] = serde_json::json!({ "effort": mapped });
        }
    } else if !request.options.thinking_enabled || deepseek {
        if let Some(effort) = effort {
            body["reasoning"] = serde_json::json!({ "effort": effort });
        }
    } else if let Some(effort) = effort {
        body["reasoning"] = serde_json::json!({ "effort": effort, "summary": "auto" });
    } else {
        body["reasoning"] = serde_json::json!({ "summary": "auto" });
    }
    if !is_xai
        && !deepseek
        && request.options.thinking_enabled
        && (!official_openai || body.get("include").is_none())
    {
        body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
    }
    if let Some(key) = options
        .prompt_cache_key
        .as_deref()
        .filter(|key| !is_xai && !deepseek && !key.is_empty())
    {
        body["prompt_cache_key"] = Value::String(key.to_string());
        if let Some(retention) = &options.prompt_cache_retention {
            body["prompt_cache_retention"] = Value::String(retention.clone());
        }
    }
    super::openai::apply_request_capabilities(
        &mut body,
        &request.model,
        options.endpoint.as_ref(),
        options.cache_retention,
        true,
    );
    super::apply_body_options(
        &mut body,
        &request.options.provider_options,
        stream && !is_xai,
    );
    if official_openai {
        super::openai::finish_official_request(&mut body, request, options.endpoint.as_ref(), true);
    }
    if request.options.tool_choice == super::ToolChoice::None {
        body.as_object_mut().unwrap().remove("tools");
        body["tool_choice"] = Value::String("none".to_string());
    }
    if is_xai && body.get("include").is_none() {
        let include = xai_include_values(body.get("tools"));
        if !include.is_empty() {
            body["include"] = Value::Array(include);
        }
    }
    body
}

/// A function-call item being assembled across streaming events. Keyed internally by
/// `item_id` (the `fc_…` id the delta/done events reference); `call_id` (the `call_…`
/// id) is what the model expects echoed back as `function_call_output.call_id`.
struct ResponsesToolPartial {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    done: bool,
}

#[derive(Default)]
struct ResponsesStreamState {
    refusal_parts: std::collections::BTreeMap<(String, u64), String>,
    text: String,
    reasoning: String,
    /// 本次请求的模型名，随 reasoning item 落进历史（回放前校验同模型，密文模型键控）。
    model: String,
    deepseek: bool,
    chatgpt_plan: bool,
    /// SSE item completion order can differ from output order. Terminal gateway
    /// snapshots can also omit a server tool already delivered by a done event.
    xai_items: std::collections::BTreeMap<u64, Value>,
    /// Provider-native reasoning used to continue stateless tool rounds.
    reasoning_items: Vec<Value>,
    tool_calls: Vec<ResponsesToolPartial>,
    finish_reason: Option<FinishReason>,
    usage: Option<ModelUsage>,
    web_search: Option<BuiltinWebSearch>,
    /// grok(xAI) 特有:`web_search_call.action.sources[]` 是检索命中的 URL 列表。
    /// 质量低于正文 `url_citation`(模型逐句引用),故只收集不直接入 citations;
    /// `finish()` 时若整轮没有任何 url_citation(grok 搜完常直接岔去客户端 web_fetch、
    /// 该轮不产正文注解)才把 sources 转正为 citations 兜底,保证搜索卡有来源可显示。
    sources_fallback: Vec<WebCitation>,
    /// hosted image generation（`image_generation_call`）产出的图。gpt-5.x 在
    /// Responses 上可自行决定用托管出图回答，此时 `message.output_text` 是空串
    /// ——图就是答案。不收这些图会让整轮变成「空助手响应」而报错。
    images: Vec<GeneratedImageData>,
    /// 当前 reasoning summary part 的 `summary_index`。GPT-5 系 concise 标题
    /// 通常没有尾换行；index 一变就要另起一段，否则「constraintsAnalyzing」。
    last_reasoning_summary_index: Option<u64>,
}

impl ResponsesStreamState {
    fn record_xai_item(&mut self, item: &Value, index: Option<u64>) -> Option<u64> {
        if !is_stateless_grok(&self.model) || !xai_output_item(item) {
            return None;
        }
        let existing = self.xai_items.iter().find_map(|(index, old)| {
            (item["id"].is_string() && old["id"] == item["id"]).then_some(*index)
        });
        let index = match existing {
            Some(index) => index,
            None => {
                let start = index.unwrap_or_else(|| {
                    self.xai_items
                        .last_key_value()
                        .map_or(0, |(i, _)| i.saturating_add(1))
                });
                (start..=u64::MAX).find(|i| !self.xai_items.contains_key(i))?
            }
        };
        // A partial terminal snapshot does not supersede encrypted output already received.
        if !self
            .xai_items
            .get(&index)
            .is_some_and(|old| has_encrypted_reasoning(old) && !has_encrypted_reasoning(item))
        {
            self.xai_items.insert(index, item.clone());
        }
        Some(index)
    }

    fn preserve_reasoning_item(&mut self, item: &Value) {
        if !reasoning_item_replayable(item, self.deepseek) {
            return;
        }
        let existing = item["id"].as_str().and_then(|id| {
            self.reasoning_items
                .iter_mut()
                .find(|existing| existing["id"] == id)
        });
        if let Some(existing) = existing {
            if self.deepseek {
                // The final response can include a fuller snapshot than output_item.done.
                *existing = item.clone();
            }
        } else {
            self.reasoning_items.push(item.clone());
        }
    }

    fn partial_mut(&mut self, item_id: &str) -> Option<&mut ResponsesToolPartial> {
        self.tool_calls
            .iter_mut()
            .find(|partial| partial.item_id == item_id)
    }

    /// 累加一个内置搜索查询词（去重）。流式下 web_search_call 走 `output_item.done` 事件，
    /// 且不重现在 `response.completed` 的 output 里，故必须边流边收。
    fn push_web_search_query(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        let ws = self
            .web_search
            .get_or_insert_with(BuiltinWebSearch::default);
        if !ws.queries.iter().any(|q| q == query) {
            ws.queries.push(query.to_string());
        }
    }

    /// 累加一条内置搜索来源（按 url 去重）。流式下走 `output_text.annotation.added` 事件。
    fn push_web_search_citation(&mut self, url: &str, title: &str) {
        let url = url.trim();
        if url.is_empty() {
            return;
        }
        let ws = self
            .web_search
            .get_or_insert_with(BuiltinWebSearch::default);
        if !ws.citations.iter().any(|c| c.url == url) {
            ws.citations.push(WebCitation {
                title: title.trim().to_string(),
                url: url.to_string(),
                ..Default::default()
            });
        }
    }

    /// Emit the accumulated search progress, including an empty start snapshot.
    fn emit_web_search_snapshot(
        &self,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<(), ModelError> {
        let (queries, citations) = match &self.web_search {
            Some(ws) => (ws.queries.clone(), ws.citations.clone()),
            None => (Vec::new(), Vec::new()),
        };
        sink.emit(StreamPart::WebSearch { queries, citations })
    }

    fn summary_index_of(value: &Value) -> Option<u64> {
        value.get("summary_index").and_then(Value::as_u64)
    }

    /// `summary_index` 变了，或 `part.added` 开了下一条（网关没带 index）时插入 `\n\n`。
    fn begin_reasoning_summary_part(
        &mut self,
        summary_index: Option<u64>,
        force_new: bool,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<(), ModelError> {
        let started_new_part = match summary_index {
            Some(index) => {
                let changed = self
                    .last_reasoning_summary_index
                    .is_some_and(|prev| prev != index);
                self.last_reasoning_summary_index = Some(index);
                changed
            }
            None => force_new && !self.reasoning.is_empty(),
        };
        if started_new_part {
            self.emit_reasoning_summary_separator(sink)?;
        }
        Ok(())
    }

    fn emit_reasoning_summary_separator(
        &mut self,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<(), ModelError> {
        let separator = summary_separator(&self.reasoning);
        self.push_reasoning_delta(separator, sink)
    }

    fn push_reasoning_delta(
        &mut self,
        delta: &str,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<(), ModelError> {
        if delta.is_empty() {
            return Ok(());
        }
        self.reasoning.push_str(delta);
        sink.emit(StreamPart::ReasoningDelta {
            delta: delta.to_string(),
        })
    }

    fn read_tool_item(
        &mut self,
        item: &Value,
        complete: bool,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<(), ModelError> {
        // The host dispatches by local name. A same-named function in a different
        // namespace must never be executed as one of our tools.
        if self.chatgpt_plan && item["namespace"] != chatgpt_plan::LOCAL_NAMESPACE {
            return Err(ModelError::with_kind(
                "ChatGPT returned a function call outside the local tool namespace",
                ModelErrorKind::InvalidRequest,
            ));
        }
        let item_id = item["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .or_else(|| item["call_id"].as_str().filter(|id| !id.is_empty()))
            .ok_or_else(|| ModelError::new("Responses function_call is missing its identifier"))?;
        let call_id = item["call_id"].as_str().unwrap_or(item_id);
        let name = item["name"].as_str().unwrap_or_default();
        let existing = self
            .tool_calls
            .iter_mut()
            .find(|call| call.item_id == item_id || call.call_id == call_id);
        let call = match existing {
            Some(call) => call,
            None => {
                sink.emit(StreamPart::ToolCallStart {
                    id: call_id.into(),
                    name: name.into(),
                })?;
                self.tool_calls.push(ResponsesToolPartial {
                    item_id: item_id.into(),
                    call_id: call_id.into(),
                    name: name.into(),
                    arguments: String::new(),
                    done: false,
                });
                self.tool_calls.last_mut().unwrap()
            }
        };
        if call.done {
            return Ok(());
        }
        if !name.is_empty() {
            call.name = name.into();
        }
        if let Some(arguments) = item["arguments"].as_str().filter(|args| !args.is_empty()) {
            call.arguments = arguments.into();
        }
        if complete && function_item_is_complete(item) {
            if call.name.is_empty() {
                return Err(ModelError::new(
                    "Responses completed a function_call without a name",
                ));
            }
            call.done = true;
            sink.emit(StreamPart::ToolCallDone {
                call: pending_tool_call_from_partial(call),
            })?;
        }
        Ok(())
    }

    fn finish(mut self, sink: &mut (dyn StreamSink + Send)) -> Result<GenerateOutput, ModelError> {
        if self.deepseek && self.reasoning.is_empty() {
            let reasoning = self
                .reasoning_items
                .iter()
                .flat_map(deepseek_reasoning_text)
                .collect::<String>();
            self.push_reasoning_delta(&reasoning, sink)?;
        }
        // The Agent handles length-truncated batches without executing them. On a
        // successful turn each executable call must have a completed provider item.
        if !self
            .finish_reason
            .as_ref()
            .is_some_and(|reason| reason.kind == FinishKind::Length)
        {
            if let Some(call) = self.tool_calls.iter().find(|call| !call.done) {
                return Err(ModelError::new(format!(
                    "Responses stream ended with an unfinished function_call ({})",
                    call.call_id
                )));
            }
        }
        let tool_calls: Vec<PendingToolCall> = self
            .tool_calls
            .iter()
            .map(pending_tool_call_from_partial)
            .collect();
        let reason = self.finish_reason.clone().unwrap_or_else(|| {
            if tool_calls.is_empty() {
                FinishReason::new(FinishKind::Stop, "")
            } else {
                FinishReason::new(FinishKind::ToolCalls, "")
            }
        });
        sink.emit(StreamPart::Finish {
            reason: reason.clone(),
            full: self.text.clone(),
        })?;
        // grok sources 兜底:整轮没有任何正文 url_citation(grok 搜完常直接岔去客户端
        // web_fetch,该轮不产注解)时,把检索命中的 sources URL 转正为来源;有 url_citation
        // 时以注解为准,sources 丢弃。
        let citations_empty = self
            .web_search
            .as_ref()
            .map(|ws| ws.citations.is_empty())
            .unwrap_or(true);
        if citations_empty && !self.sources_fallback.is_empty() {
            let ws = self
                .web_search
                .get_or_insert_with(BuiltinWebSearch::default);
            ws.citations = std::mem::take(&mut self.sources_fallback);
        }
        if let Some(ws) = self.web_search.as_ref().filter(|ws| !ws.is_empty()) {
            sink.emit(StreamPart::WebSearch {
                queries: ws.queries.clone(),
                citations: ws.citations.clone(),
            })?;
        }
        let model = self.model;
        let mut output = GenerateOutput {
            text: self.text,
            reasoning: non_empty(self.reasoning),
            tool_calls,
            usage: self.usage,
            finish_reason: Some(reason),
            cancelled: false,
            web_search: self.web_search,
            images: self.images,
            canonical_message: None,
            billing_usage: None,
            reasoning_items: self
                .reasoning_items
                .into_iter()
                .map(|item| ProviderReasoningItem {
                    model: model.clone(),
                    item,
                    source: None,
                })
                .collect(),
        };
        preserve_xai_output(&mut output, &model, self.xai_items.values());
        Ok(output)
    }
}

fn is_stateless_grok(model: &str) -> bool {
    matches!(model, "grok-4.7" | "grok-4.7-latest")
}

// xAI distinguishes hosted custom calls by their call ID (X / attachment search).
fn xai_server_tool(item: &Value) -> bool {
    match item["type"].as_str() {
        Some(
            "web_search_call"
            | "x_search_call"
            | "file_search_call"
            | "code_interpreter_call"
            | "mcp_call",
        ) => true,
        Some("custom_tool_call") => item["call_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("xs_") || id.starts_with("as_")),
        _ => false,
    }
}

fn xai_output_item(item: &Value) -> bool {
    // Replay describes what the model returned, including a length-limited last item.
    // Only the separate function-call decoder can mark a local call ready to execute.
    if !function_item_is_complete(item)
        && !matches!(item["status"].as_str(), Some("incomplete" | "in_progress"))
    {
        return false;
    }
    match item["type"].as_str() {
        Some("message") => {
            item["role"] == "assistant"
                && item["content"].as_array().is_some_and(|parts| {
                    parts.iter().all(|part| match part["type"].as_str() {
                        Some("output_text") => part["text"].is_string(),
                        Some("refusal") => part["refusal"].is_string(),
                        _ => false,
                    })
                })
        }
        Some("function_call") => {
            item["call_id"].is_string() && item["name"].is_string() && item["arguments"].is_string()
        }
        Some("reasoning") => has_encrypted_reasoning(item),
        _ => xai_server_tool(item) && item["id"].as_str().is_some_and(|id| !id.is_empty()),
    }
}

fn preserve_xai_output<'a>(
    output: &mut GenerateOutput,
    model: &str,
    items: impl Iterator<Item = &'a Value> + Clone,
) {
    let truncated = output
        .finish_reason
        .as_ref()
        .is_some_and(|reason| reason.kind == FinishKind::Length);
    if !items
        .clone()
        .any(|item| (truncated || xai_server_tool(item)) && xai_output_item(item))
    {
        return;
    }
    let mut replay = output.replay_message();
    replay.content.retain(|part| {
        !matches!(
            part,
            MessagePart::ReasoningItem { .. } | MessagePart::Reasoning { .. }
        )
    });
    replay.content.insert(
        0,
        MessagePart::XaiOutput {
            model: model.into(),
            source: None,
            items: items
                .filter(|item| xai_output_item(item))
                .cloned()
                .collect(),
        },
    );
    output.canonical_message = Some(replay);
}

pub(super) fn xai_replay_items<'a>(
    message: &'a super::ModelMessage,
    can_replay: &impl Fn(&str, Option<&ResponsesReasoningSource>) -> bool,
) -> Option<&'a [Value]> {
    let MessagePart::XaiOutput {
        model,
        items,
        source: Some(source),
    } = message.content.first()?
    else {
        return None;
    };
    if message.role != super::ModelRole::Assistant
        || source.format != ResponsesFormat::Xai
        || !can_replay(model, Some(source))
        || !items.iter().all(xai_output_item)
    {
        return None;
    }
    // Edits and interrupted tool-call repairs must take precedence over old wire output.
    let text = items
        .iter()
        .flat_map(message_text_parts)
        .collect::<String>();
    let native_calls = items
        .iter()
        .filter(|item| item["type"] == "function_call")
        .map(|item| {
            (
                item["call_id"].as_str(),
                item["name"].as_str(),
                Some(responses_tool_arguments(item["arguments"].as_str())),
            )
        });
    let mut current_calls = message.content.iter().filter_map(|part| match part {
        MessagePart::ToolCall {
            id,
            name,
            arguments_raw,
            ..
        } => Some((
            Some(id.as_str()),
            Some(name.as_str()),
            Some(arguments_raw.as_str()),
        )),
        _ => None,
    });
    // Concurrent calls may finish in a different order in SSE. Receipts pair by ID.
    let mut native_calls: Vec<_> = native_calls.collect();
    let calls_match = current_calls.all(|call| {
        if let Some(index) = native_calls.iter().position(|native| *native == call) {
            native_calls.swap_remove(index);
            true
        } else {
            false
        }
    }) && native_calls.is_empty();
    (text == message.text_content() && calls_match).then_some(items)
}

fn has_encrypted_reasoning(item: &Value) -> bool {
    item.get("encrypted_content")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn deepseek_reasoning_text(item: &Value) -> impl Iterator<Item = &str> {
    item["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "reasoning_text")
        .filter_map(|part| part["text"].as_str())
}

fn reasoning_item_replayable(item: &Value, deepseek: bool) -> bool {
    if deepseek {
        deepseek_reasoning_text(item).any(|text| !text.trim().is_empty())
    } else {
        has_encrypted_reasoning(item)
    }
}

/// 从一个 `image_generation_call` output item 取出生成的图。`result` 是裸 base64
/// （无 `data:` 前缀），mime 由 `output_format`（png/jpeg/webp）推出，缺省 png。
/// 解析尽力而为：没有 `result` 就返回 None，绝不 panic。
fn responses_image_from_item(item: &Value) -> Option<GeneratedImageData> {
    let data = item
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let format = item
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or("png")
        .trim()
        .to_ascii_lowercase();
    let mime_type = match format.as_str() {
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    };
    Some(GeneratedImageData {
        mime_type: mime_type.to_string(),
        data: data.to_string(),
    })
}

/// Dispatch a single streaming Responses event into the accumulating state, emitting
/// `StreamPart`s as content arrives. Returns `Ok(Some(err))` for a terminal provider
/// error (caller records the failure and aborts), `Ok(None)` otherwise. Free function
/// (no `&self`) so the stream loop and unit tests share one code path.
fn handle_responses_stream_event(
    value: &Value,
    state: &mut ResponsesStreamState,
    sink: &mut (dyn StreamSink + Send),
) -> Result<Option<ModelError>, ModelError> {
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "response.refusal.delta" | "response.refusal.done" => {
            let snapshot = event_type.ends_with(".done");
            let text = value[if snapshot { "refusal" } else { "delta" }]
                .as_str()
                .unwrap_or_default();
            push_refusal(
                state,
                value["item_id"].as_str().unwrap_or_default(),
                value["content_index"].as_u64().unwrap_or(0),
                text,
                snapshot,
                sink,
            )?;
        }
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                if !delta.is_empty() {
                    state.text.push_str(delta);
                    sink.emit(StreamPart::TextDelta {
                        delta: delta.to_string(),
                    })?;
                }
            }
        }
        "response.reasoning_summary_part.added" => {
            state.begin_reasoning_summary_part(
                ResponsesStreamState::summary_index_of(value),
                true,
                sink,
            )?;
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_summary.delta" => {
            state.begin_reasoning_summary_part(
                ResponsesStreamState::summary_index_of(value),
                false,
                sink,
            )?;
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                state.push_reasoning_delta(delta, sink)?;
            }
        }
        "response.reasoning_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                state.push_reasoning_delta(delta, sink)?;
            }
        }
        "response.output_item.added" => {
            if let Some(item) = value.get("item") {
                match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => state.read_tool_item(item, false, sink)?,
                    // 内置搜索开始：查询词此刻还没到，发一帧空快照 = 立即开一张 Running 卡。
                    Some("web_search_call") => {
                        state.emit_web_search_snapshot(sink)?;
                    }
                    _ => {}
                }
            }
        }
        "response.function_call_arguments.delta" => {
            let item_id = value.get("item_id").and_then(Value::as_str).unwrap_or("");
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                if let Some(partial) = state.partial_mut(item_id).filter(|call| !call.done) {
                    partial.arguments.push_str(delta);
                    sink.emit(StreamPart::ToolCallDelta {
                        id: partial.call_id.clone(),
                        delta: delta.to_string(),
                    })?;
                }
            }
        }
        "response.function_call_arguments.done" => {
            let item_id = value.get("item_id").and_then(Value::as_str).unwrap_or("");
            if let Some(arguments) = value.get("arguments").and_then(Value::as_str) {
                if let Some(partial) = state.partial_mut(item_id).filter(|call| !call.done) {
                    // The `done` event carries the full argument string — prefer it over
                    // the accumulated deltas to avoid any drift.
                    partial.arguments = arguments.to_string();
                }
            }
        }
        "response.output_item.done" => {
            if let Some(item) = value.get("item") {
                state.record_xai_item(item, value["output_index"].as_u64());
                finish_refusal_parts(state, item, sink)?;
                match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => state.read_tool_item(item, true, sink)?,
                    // 内置搜索：hosted web_search 的查询词在此事件到达（completed 的 output 里不重现）。
                    Some("web_search_call") => {
                        if let Some(action) = item.get("action") {
                            if let Some(q) = action.get("query").and_then(Value::as_str) {
                                state.push_web_search_query(q);
                            }
                            if let Some(list) = action.get("queries").and_then(Value::as_array) {
                                for q in list.iter().filter_map(Value::as_str) {
                                    state.push_web_search_query(q);
                                }
                            }
                            // grok(xAI) 特有:action.sources[] 是检索命中的 URL,先收集,
                            // finish() 时无 url_citation 才转正为兜底来源。
                            if let Some(sources) = action.get("sources").and_then(Value::as_array) {
                                for source in sources {
                                    let Some(citation) = grok_source_citation(source) else {
                                        continue;
                                    };
                                    if !state.sources_fallback.iter().any(|c| c.url == citation.url)
                                    {
                                        state.sources_fallback.push(citation);
                                    }
                                }
                            }
                        }
                        // push 完 emit 当前累加快照 → 实时更新卡（带查询词）。
                        state.emit_web_search_snapshot(sink)?;
                    }
                    // hosted 出图：完整 base64 在 item.result。message 的 output_text 常为
                    // 空串（图就是答案），不收这张图整轮会被判成空助手响应。
                    Some("image_generation_call") => {
                        if let Some(image) = responses_image_from_item(item) {
                            sink.emit(StreamPart::ImageData {
                                mime_type: image.mime_type.clone(),
                                data: image.data.clone(),
                            })?;
                            state.images.push(image);
                        }
                    }
                    Some("reasoning") => {
                        state.preserve_reasoning_item(item);
                    }
                    _ => {}
                }
            }
        }
        // 内置搜索来源:url_citation 注解随正文流式到达。
        "response.output_text.annotation.added" => {
            if let Some(annotation) = value.get("annotation") {
                if annotation.get("type").and_then(Value::as_str) == Some("url_citation") {
                    if let Some(url) = annotation.get("url").and_then(Value::as_str) {
                        let title = annotation
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        state.push_web_search_citation(url, title);
                        // push 完 emit 当前累加快照 → 实时更新卡（带来源）。
                        state.emit_web_search_snapshot(sink)?;
                    }
                }
            }
        }
        "response.incomplete" if state.chatgpt_plan => {
            let reason = value["response"]["incomplete_details"]["reason"]
                .as_str()
                .unwrap_or("incomplete");
            return Ok(Some(ModelError::from_provider_payload(
                Some(reason),
                format!("ChatGPT response incomplete: {reason}"),
            )));
        }
        "response.completed" | "response.incomplete" => {
            if let Some(response) = value.get("response") {
                if let Some(error) = responses_terminal_error(response) {
                    return Ok(Some(error));
                }
                if let Some(usage) = model_usage_from_openai_value(response) {
                    if let Some(input_tokens) = usage.input_tokens {
                        sink.emit(StreamPart::ContextUsage {
                            input_tokens,
                            output_tokens: usage.output_tokens.unwrap_or(0),
                        })?;
                    }
                    state.usage = Some(usage);
                }
                // 完整 output 里若还带 web_search_call / message 注解，合并进已流式累加的结果
                // （不覆盖：hosted web_search_call 通常只在 output_item.done 里出现，不重现在此）。
                if let Some(output) = response.get("output").and_then(Value::as_array) {
                    if let Some(extra) = web_search_from_responses_output(output) {
                        for q in extra.queries {
                            state.push_web_search_query(&q);
                        }
                        for c in extra.citations {
                            state.push_web_search_citation(&c.url, &c.title);
                        }
                    }
                    if is_stateless_grok(&state.model) {
                        let mut output_index = 0;
                        for item in output {
                            if let Some(index) = state.record_xai_item(item, Some(output_index)) {
                                output_index = index.saturating_add(1);
                            }
                        }
                    }
                    // A gateway can deliver some or all of the text only in response.output.
                    // Replay the missing suffix when the snapshot agrees with published deltas.
                    let text: String = if state
                        .xai_items
                        .values()
                        .any(|item| item["type"] == "message")
                    {
                        state
                            .xai_items
                            .values()
                            .flat_map(message_text_parts)
                            .collect()
                    } else {
                        output.iter().flat_map(message_text_parts).collect()
                    };
                    let suffix = text.strip_prefix(&state.text);
                    if let Some(delta) = suffix.filter(|text| !text.is_empty()) {
                        state.text.push_str(delta);
                        sink.emit(StreamPart::TextDelta {
                            delta: delta.into(),
                        })?;
                    }
                    for item in output {
                        if suffix.is_none() {
                            // Partial terminal snapshots may still contain a new refusal block.
                            finish_refusal_parts(state, item, sink)?;
                        }
                        if item["type"] == "function_call" {
                            state.read_tool_item(item, response["status"] == "completed", sink)?;
                        } else if item["type"] == "reasoning" {
                            state.preserve_reasoning_item(item);
                        }
                    }
                }
                if let Some(status) = response.get("status").and_then(Value::as_str) {
                    state.finish_reason = Some(responses_finish_reason(
                        status,
                        response,
                        !state.tool_calls.is_empty(),
                    ));
                }
            }
        }
        "response.cancelled" => {
            return Ok(Some(ModelError::with_kind(
                "Responses cancelled",
                ModelErrorKind::Cancelled,
            )));
        }
        "response.failed" | "error" => {
            let code = responses_error_payload(value)
                .and_then(provider_error_code)
                .or_else(|| value["code"].as_str().map(str::to_string));
            let error = if state.chatgpt_plan {
                let payload = responses_error_payload(value).unwrap_or(value);
                ModelError::from_provider_payload(
                    code.as_deref(),
                    format!("ChatGPT Responses error: {payload}"),
                )
            } else {
                ModelError::from_provider_stream(
                    code.as_deref(),
                    responses_stream_error_text(value),
                )
            };
            return Ok(Some(error));
        }
        _ => {}
    }
    Ok(None)
}

/// Decode one complete event payload, shared by live streaming and buffered fallback.
fn process_sse_data(
    data: &str,
    state: &mut ResponsesStreamState,
    sink: &mut (dyn StreamSink + Send),
) -> Result<Option<ModelError>, ModelError> {
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(None);
    }
    let value: Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    handle_responses_stream_event(&value, state, sink)
}

/// True if `body` looks like a Responses SSE stream rather than a JSON object — i.e. it
/// contains a line starting with `data:` or `event:` (tolerant of CRLF and leading
/// whitespace). Used to decide whether a non-JSON non-stream response body can be salvaged.
fn looks_like_sse(body: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("data:") || line.starts_with("event:")
    })
}

/// Parse a fully-buffered Responses **SSE** body (a provider that streamed despite
/// `stream:false`) into a `GenerateOutput`, reusing the exact streaming accumulation. The
/// non-stream path has no live consumer, so events are fed through a discarding sink.
fn output_from_sse_body(
    body: &str,
    deepseek: bool,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    let mut state = ResponsesStreamState {
        model: model.into(),
        deepseek,
        ..Default::default()
    };
    let mut sink = DiscardSink;
    for data in super::sse::SseDecoder::default().push(body.as_bytes())? {
        if let Some(err) = process_sse_data(&data, &mut state, &mut sink)? {
            return Err(err);
        }
        if state.finish_reason.is_some() {
            break;
        }
    }
    if state.finish_reason.is_none() {
        return Err(ModelError::stream_ended_early("Responses SSE body"));
    }
    state.finish(&mut sink)
}

/// A `StreamSink` that drops every part. Used by the non-stream SSE fallback where the
/// accumulated `GenerateOutput` is the only consumer and no live deltas are needed.
struct DiscardSink;

impl StreamSink for DiscardSink {
    fn emit(&mut self, _part: StreamPart) -> Result<(), ModelError> {
        Ok(())
    }
}

// Compatible endpoints may omit item status in their final snapshot.
fn function_item_is_complete(item: &Value) -> bool {
    item["status"].is_null() || item["status"] == "completed"
}

fn responses_tool_arguments(arguments: Option<&str>) -> &str {
    arguments
        .filter(|raw| !raw.trim().is_empty())
        .unwrap_or("{}")
}

fn pending_tool_call_from_partial(partial: &ResponsesToolPartial) -> PendingToolCall {
    let raw = responses_tool_arguments(Some(&partial.arguments)).to_owned();
    let (arguments, arguments_parse_error) = parse_tool_arguments(&raw);
    PendingToolCall {
        id: partial.call_id.clone(),
        function_name: partial.name.clone(),
        arguments,
        arguments_raw: raw,
        arguments_parse_error,
        signature: None,
    }
}

/// Parse a non-streaming `/v1/responses` body into a `GenerateOutput`.
pub fn output_from_responses(
    value: &Value,
    raw: &str,
    label: &str,
) -> Result<GenerateOutput, ModelError> {
    output_from_responses_payload(value, raw, label, false)
}

fn output_from_responses_payload(
    value: &Value,
    raw: &str,
    label: &str,
    deepseek: bool,
) -> Result<GenerateOutput, ModelError> {
    // DeepSeek / OpenAI Responses 成功体带 `"error": null`，不能当成失败。
    match value.get("error") {
        None | Some(Value::Null) => {}
        Some(error) if error.as_object().is_some_and(|obj| obj.is_empty()) => {}
        Some(error) => {
            let code = provider_error_code(error);
            return Err(ModelError::from_provider_payload(
                code.as_deref(),
                format!("{label}: {}", responses_error_message(error)),
            ));
        }
    }
    if let Some(error) = responses_terminal_error(value) {
        return Err(error);
    }
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_response(label, raw))?;

    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let mut images = Vec::new();
    let mut reasoning_items: Vec<Value> = Vec::new();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => text.extend(message_text_parts(item)),
            Some("function_call") => {
                if !function_item_is_complete(item) && value["status"] != "incomplete" {
                    return Err(ModelError::new(format!(
                        "{label}: unfinished function_call in final response"
                    )));
                }
                let call_id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let raw_args = responses_tool_arguments(item["arguments"].as_str()).to_owned();
                let (arguments, arguments_parse_error) = parse_tool_arguments(&raw_args);
                tool_calls.push(PendingToolCall {
                    id: call_id,
                    function_name: name,
                    arguments,
                    arguments_raw: raw_args,
                    arguments_parse_error,
                    signature: None,
                });
            }
            // hosted 出图：与流式 `output_item.done` 同一形态，走同一提取器。
            Some("image_generation_call") => {
                if let Some(image) = responses_image_from_item(item) {
                    images.push(image);
                }
            }
            // The response decoder binds replay data to the request model.
            Some("reasoning") => {
                if deepseek {
                    reasoning.extend(deepseek_reasoning_text(item));
                } else if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                    for text in summary
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .filter(|text| !text.is_empty())
                    {
                        reasoning.push_str(summary_separator(&reasoning));
                        reasoning.push_str(text);
                    }
                }
                if reasoning_item_replayable(item, deepseek) {
                    reasoning_items.push(item.clone());
                }
            }
            _ => {}
        }
    }

    let usage = model_usage_from_openai_value(value);
    let finish_reason = value
        .get("status")
        .and_then(Value::as_str)
        .map(|status| responses_finish_reason(status, value, !tool_calls.is_empty()));

    Ok(GenerateOutput {
        text,
        reasoning: non_empty(reasoning),
        tool_calls,
        usage,
        finish_reason,
        cancelled: false,
        web_search: web_search_from_responses_output(output),
        canonical_message: None,
        billing_usage: None,
        images,
        reasoning_items: reasoning_items
            .into_iter()
            .map(|item| ProviderReasoningItem {
                model: String::new(),
                item,
                source: None,
            })
            .collect(),
    })
}

fn summary_separator(text: &str) -> &'static str {
    if text.is_empty() || text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

/// 非流式/SSE 兜底解析拿不到请求模型，reasoning item 的 `model` 先留空；
/// 调用方在拿到 `GenerateOutput` 后用请求模型统一盖戳（空 model 的 item 回放时
/// 永远匹配不上任何请求模型，等于被静默丢弃——所以漏盖戳只是少回放，不会错回放）。
fn stamp_reasoning_item_model(output: &mut GenerateOutput, model: &str) {
    for entry in &mut output.reasoning_items {
        if entry.model.is_empty() {
            entry.model = model.to_string();
        }
    }
}

/// 把 grok(xAI) `web_search_call.action.sources[]` 的单项解析成 `WebCitation`。
/// 尽力而为：url 必须有；title 取 `title`/`name`（都没有则留空，前端按域名兜底），
/// snippet 取 `description`/`snippet`（截断到 400 字符，防超大载荷拖慢流式卡）。
fn grok_source_citation(source: &Value) -> Option<WebCitation> {
    let url = source
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let title = source
        .get("title")
        .or_else(|| source.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();
    let snippet = source
        .get("description")
        .or_else(|| source.get("snippet"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(400).collect());
    Some(WebCitation {
        title,
        url: url.to_string(),
        snippet,
        ..Default::default()
    })
}

/// 从 Responses `output[]` 数组解析内置搜索痕迹：`web_search_call` 项取查询词、
/// `message` 项内容的 `annotations[type=url_citation]` 取来源。任一为空则整体可能为空，
/// 全空时返回 None（视作没发生可见的内置搜索）。解析尽力而为，绝不 panic。
fn web_search_from_responses_output(output: &[Value]) -> Option<BuiltinWebSearch> {
    let mut result = BuiltinWebSearch::default();
    let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    // grok(xAI) 特有:web_search_call.action.sources[](检索命中 URL)。仅当整个输出
    // 没有任何正文 url_citation 时才转正为兜底来源(注解质量更高,优先)。
    let mut sources_fallback: Vec<WebCitation> = Vec::new();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("web_search_call") => {
                // 查询词可能在 action.query 或顶层 query（wire 随版本略有差异，两处都试）。
                let query = item
                    .get("action")
                    .and_then(|action| action.get("query"))
                    .or_else(|| item.get("query"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(query) = query {
                    if !result.queries.iter().any(|existing| existing == query) {
                        result.queries.push(query.to_string());
                    }
                }
                if let Some(sources) = item
                    .get("action")
                    .and_then(|action| action.get("sources"))
                    .and_then(Value::as_array)
                {
                    for source in sources {
                        let Some(citation) = grok_source_citation(source) else {
                            continue;
                        };
                        if !sources_fallback.iter().any(|c| c.url == citation.url) {
                            sources_fallback.push(citation);
                        }
                    }
                }
            }
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        let Some(annotations) = part.get("annotations").and_then(Value::as_array)
                        else {
                            continue;
                        };
                        for annotation in annotations {
                            if annotation.get("type").and_then(Value::as_str)
                                != Some("url_citation")
                            {
                                continue;
                            }
                            let Some(url) = annotation
                                .get("url")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            else {
                                continue;
                            };
                            if !seen_urls.insert(url.to_string()) {
                                continue;
                            }
                            let title = annotation
                                .get("title")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .unwrap_or(url)
                                .to_string();
                            result.citations.push(WebCitation {
                                title,
                                url: url.to_string(),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // 无正文注解时才用 sources 兜底(有注解则丢弃 sources,注解更准)。
    if result.citations.is_empty() && !sources_fallback.is_empty() {
        result.citations = sources_fallback;
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn push_refusal(
    state: &mut ResponsesStreamState,
    item_id: &str,
    index: u64,
    text: &str,
    snapshot: bool,
    sink: &mut (dyn StreamSink + Send),
) -> Result<(), ModelError> {
    let part = state
        .refusal_parts
        .entry((item_id.into(), index))
        .or_default();
    let delta = if snapshot {
        text.strip_prefix(part.as_str()).unwrap_or_default()
    } else {
        text
    };
    if !delta.is_empty() {
        part.push_str(delta);
        state.text.push_str(delta);
        sink.emit(StreamPart::TextDelta {
            delta: delta.into(),
        })?;
    }
    Ok(())
}

fn message_text_parts(item: &Value) -> impl Iterator<Item = &str> {
    item["content"]
        .as_array()
        .filter(|_| item["type"] == "message")
        .into_iter()
        .flatten()
        .filter_map(|part| match part["type"].as_str()? {
            "output_text" => part["text"].as_str(),
            "refusal" => part["refusal"].as_str(),
            _ => None,
        })
}

fn finish_refusal_parts(
    state: &mut ResponsesStreamState,
    item: &Value,
    sink: &mut (dyn StreamSink + Send),
) -> Result<(), ModelError> {
    if item["type"] == "message" {
        for (index, part) in item["content"].as_array().into_iter().flatten().enumerate() {
            if part["type"] == "refusal" {
                push_refusal(
                    state,
                    item["id"].as_str().unwrap_or_default(),
                    index as u64,
                    part["refusal"].as_str().unwrap_or_default(),
                    true,
                    sink,
                )?;
            }
        }
    }
    Ok(())
}

fn responses_terminal_error(response: &Value) -> Option<ModelError> {
    let status = response["status"].as_str()?;
    let reason = response["incomplete_details"]["reason"]
        .as_str()
        .unwrap_or(status);
    if !matches!(status, "incomplete" | "failed" | "cancelled")
        || (status == "incomplete" && reason == "max_output_tokens")
    {
        return None;
    }
    let kind = match (status, reason) {
        (_, "content_filter") => ModelErrorKind::ContentModeration,
        ("cancelled", _) => ModelErrorKind::Cancelled,
        _ => ModelErrorKind::Other,
    };
    let mut error = ModelError::with_kind(format!("Responses {status}: {reason}"), kind);
    error.provider_code = Some(reason.into());
    Some(error)
}

fn responses_finish_reason(status: &str, response: &Value, has_tool_calls: bool) -> FinishReason {
    let detail = response["incomplete_details"]["reason"].as_str();
    let kind = match (status, detail) {
        ("completed", _) if has_tool_calls => FinishKind::ToolCalls,
        ("completed", _) => FinishKind::Stop,
        ("incomplete", Some("max_output_tokens")) => FinishKind::Length,
        _ => FinishReason::from_openai(status).kind,
    };
    let mut reason = FinishReason::new(kind, status);
    if status == "incomplete" {
        reason.detail = detail.map(str::to_string);
        if kind != FinishKind::Length {
            reason = reason.with_legacy_name(status);
        }
    }
    reason
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

/// Extract the most informative human-readable message from a Responses API `error`
/// object. Providers vary: some put the real reason in `message`, some only in `code`
/// or `type`, and some proxies return a bare object. Try `message` → `code` → `type` →
/// the compact JSON of the whole error value; only fall back to a generic string when the
/// error object carries nothing at all. This surfaces the provider's real reason (rate
/// limit, 502, context length, …) instead of a useless "Unknown Responses API error".
fn responses_error_message(error: &Value) -> String {
    for key in ["message", "code", "type"] {
        if let Some(text) = error.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    // No standard field carried a string. If the error value is a non-empty object/array
    // or a non-empty scalar, serialize it compactly so the real payload is visible.
    let serialized = error.to_string();
    if !error.is_null() && serialized != "{}" && serialized != "\"\"" && !serialized.is_empty() {
        return serialized;
    }
    "Unknown Responses API error".to_string()
}

/// `response.failed` 把原因放在 `response.error`；OpenAI 的 `type: error` 事件把
/// `message` 放在顶层；部分中转把 `error` 直接写成字符串。三条都要读到，否则缺终态
/// 的那条文案会变成笼统的 "Responses stream failed"，无法正确识别可重试错误。
fn responses_error_payload(value: &Value) -> Option<&Value> {
    value
        .get("response")
        .and_then(|response| response.get("error"))
        .or_else(|| value.get("error"))
}

fn responses_stream_error_text(value: &Value) -> String {
    if let Some(error) = responses_error_payload(value) {
        if let Some(text) = error.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return text.to_string();
        }
        let message = responses_error_message(error);
        return match error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(Value::as_str)
        {
            Some(code) => format!("{message} ({code})"),
            None => message,
        };
    }
    value
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|message| match value.get("code").and_then(Value::as_str) {
            Some(code) => format!("{message} ({code})"),
            None => message.to_string(),
        })
        .unwrap_or_else(|| "Responses stream failed".to_string())
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GenerateOptions, ModelMessage, ModelRole};

    #[test]
    fn json_summaries_keep_paragraphs_and_optional_status_compatibility() {
        let raw = serde_json::json!({"output":[
            {"type":"reasoning","encrypted_content":"opaque","summary":[
                {"type":"summary_text","text":"first\n"},
                {"type":"summary_text","text":"second"}]},
            {"type":"message","content":[{"type":"output_text","text":"answer"}]}]
        })
        .to_string();
        let output = decode_responses_body(&raw, "fixture", "test-model").unwrap();
        assert_eq!(output.reasoning.as_deref(), Some("first\n\nsecond"));
        assert_eq!(output.text, "answer");
        assert!(output.finish_reason.is_none());
        assert_eq!(output.reasoning_items[0].model, "test-model");
        assert_eq!(
            output.reasoning_items[0].item["encrypted_content"],
            "opaque"
        );
    }

    fn request() -> GenerateRequest {
        GenerateRequest {
            model: "test-model".into(),
            system: String::new(),
            messages: vec![ModelMessage::text(ModelRole::User, "hi")],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        }
    }

    #[test]
    fn grok_stateless_search_preserves_ordered_output_including_stream_only_items() {
        use serde_json::json;
        let items = vec![
            json!({"type":"web_search_call","id":"ws_1","status":"completed","action":{"type":"search","query":"q"},"encrypted_content":"search-state"}),
            json!({"type":"message","role":"assistant","id":"msg_1","content":[{"type":"output_text","text":"found","annotations":[]}]}),
            json!({"type":"reasoning","id":"rs_1","encrypted_content":"reasoning-state","summary":[]}),
            json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup","arguments":"{}","status":"completed"}),
        ];
        let mut stream = String::new();
        for index in [3, 0, 2] {
            stream += &format!(
                "data: {}\n\n",
                json!({"type":"response.output_item.done","output_index":index,"item":items[index]})
            );
        }
        stream += &format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":{"status":"completed","output":[items[1],items[2],items[3]]}})
        );
        let endpoint = "https://api.x.ai/v1/responses".parse().unwrap();
        for raw in [
            json!({"status":"completed","output":items}).to_string(),
            stream,
        ] {
            let mut request = request();
            request.model = "grok-4.7".into();
            let mut output =
                decode_responses_body_at_endpoint(&raw, "xAI", &request.model, Some(&endpoint))
                    .unwrap();
            bind_output_provenance(
                &mut output,
                ResponsesFormat::Xai,
                Some("a"),
                Some(&endpoint),
            );
            let wire =
                super::super::openai_messages_from_model_messages(&[output.replay_message()]);
            request
                .messages
                .extend(super::super::model_messages_from_openai_messages(wire));
            request.messages.push(ModelMessage {
                role: ModelRole::Tool,
                content: vec![MessagePart::ToolResult {
                    tool_call_id: "call_1".into(),
                    content: "receipt".into(),
                    is_error: false,
                    artifacts: vec![],
                }],
            });
            let mut options = OpenAiResponsesOptions {
                format: ResponsesFormat::Xai,
                provider_id: Some("a".into()),
                endpoint: Some(endpoint.clone()),
                ..Default::default()
            };
            let body = request_body(&request, false, &options);
            let input = body["input"].as_array().unwrap();
            assert_eq!(&input[1..input.len() - 1], items);
            assert_eq!(input.last().unwrap()["output"], "receipt");
            options.provider_id = Some("b".into());
            let other = request_body(&request, false, &options);
            assert!(!other.to_string().contains("search-state"));
            assert!(!other.to_string().contains("reasoning-state"));
            assert!(other.to_string().contains("found"));
        }
    }

    #[test]
    fn encoded_call_options_override_defaults_and_provider_options_apply_last() {
        let mut request = request();
        request.options.max_tokens = 321;
        request.options.temperature = Some(0.7);
        request.options.thinking_level = Some("high".into());
        request.options.builtin_web_search = true;
        let options = OpenAiResponsesOptions {
            temperature: Some(0.7),
            extra_body: serde_json::json!({
                "temperature": 0.1,
                "max_output_tokens": 1,
                "reasoning": { "effort": "low" },
                "custom": true,
            }),
            ..Default::default()
        };
        let body = request_body(&request, false, &options);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["max_output_tokens"], 321);
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["custom"], true);
        assert_eq!(body["store"], false);

        request.options.provider_options = serde_json::json!({
            "temperature": 0.9,
            "max_output_tokens": 17,
            "reasoning": { "effort": "medium" },
            "tools": [{ "type": "web_search" }],
            "tool_choice": "required",
        });
        let body = request_body(&request, false, &options);
        assert_eq!(body["temperature"], 0.9);
        assert_eq!(body["max_output_tokens"], 17);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["tool_choice"], "required");

        request.options.tool_choice = super::super::ToolChoice::None;
        let body = request_body(&request, false, &options);
        assert!(body.get("tools").is_none());
        assert_eq!(body["tool_choice"], "none");

        request.options.provider_options = Value::Null;
        request.options.max_tokens = 0;
        assert!(request_body(&request, false, &options)
            .get("max_output_tokens")
            .is_none());
    }

    #[test]
    fn body_decoder_stamps_json_and_sse_reasoning_with_request_model() {
        let response = serde_json::json!({
            "status": "completed",
            "output": [{ "id": "rs_1", "type": "reasoning", "encrypted_content": "cipher" }],
        });
        let bodies = [
            response.to_string(),
            format!(
                "data: {}\n\n",
                serde_json::json!({ "type": "response.completed", "response": response })
            ),
        ];
        for body in bodies {
            let output = decode_responses_body(&body, "test", "test-model").unwrap();
            assert_eq!(output.reasoning_items.len(), 1);
            assert_eq!(output.reasoning_items[0].model, "test-model");
            assert_eq!(
                output.reasoning_items[0].item["encrypted_content"],
                "cipher"
            );
        }
    }

    #[test]
    fn xai_malformed_extra_input_does_not_panic() {
        let mut request = request();
        request.system = "system".into();
        let options = OpenAiResponsesOptions {
            format: ResponsesFormat::Xai,
            extra_body: serde_json::json!({ "input": "custom" }),
            ..Default::default()
        };
        assert_eq!(request_body(&request, false, &options)["input"], "custom");
    }

    #[test]
    fn reasoning_replay_is_ordered_model_bound_and_host_controlled() {
        let mut request = request();
        request.messages.push(ModelMessage {
            role: ModelRole::Assistant,
            content: vec![
                MessagePart::ReasoningItem {
                    model: request.model.clone(),
                    source: None,
                    item: serde_json::json!({
                        "id": "rs_1", "type": "reasoning", "encrypted_content": "cipher",
                    }),
                },
                MessagePart::Text {
                    text: "checking".into(),
                },
                MessagePart::ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                    arguments_raw: "{}".into(),
                    signature: None,
                },
            ],
        });
        request.messages.push(ModelMessage {
            role: ModelRole::Tool,
            content: vec![MessagePart::ToolResult {
                tool_call_id: "call_1".into(),
                content: "result".into(),
                is_error: false,
                artifacts: Vec::new(),
            }],
        });
        let body = request_body(&request, false, &Default::default());
        assert_eq!(body["input"][1]["type"], "reasoning");
        assert_eq!(body["input"][2]["role"], "assistant");
        assert_eq!(body["input"][3]["type"], "function_call");
        assert_eq!(body["input"][4]["type"], "function_call_output");

        for options in [
            OpenAiResponsesOptions {
                replay_reasoning: false,
                ..Default::default()
            },
            OpenAiResponsesOptions {
                format: ResponsesFormat::Xai,
                ..Default::default()
            },
        ] {
            let body = request_body(&request, false, &options);
            assert!(body["input"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["type"] != "reasoning"));
        }
        request.model = "another-model".into();
        let body = request_body(&request, false, &Default::default());
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "reasoning"));
    }

    #[tokio::test]
    async fn standalone_provider_rejects_video_before_sending() {
        let provider = OpenAiResponsesProvider::new(
            reqwest::Client::new(),
            OpenAiResponsesConfig::new("http://127.0.0.1:1/responses".parse().unwrap()),
        );
        let mut request = request();
        request.messages[0].content.push(MessagePart::Video {
            mime_type: "video/mp4".into(),
            data: "AA==".into(),
            path: None,
        });
        let error = LanguageModelProvider::generate(&provider, request)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ModelErrorKind::InvalidRequest);
    }

    #[tokio::test]
    async fn stream_completion_releases_an_unclosed_http_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            let body = concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"done\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":7,\"output_tokens\":1}}}\n\n"
            );
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len() + 1,
            ).as_bytes()).await.unwrap();
            assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
        });
        let response = reqwest::Client::new().get(endpoint).send().await.unwrap();
        let mut parts = Vec::new();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            decode_responses_stream(
                response,
                &mut |part| {
                    parts.push(part);
                    Ok(())
                },
                "test",
                "test-model",
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(output.text, "done");
        assert_eq!(output.usage.unwrap().input_tokens, Some(7));
        assert_eq!(
            parts
                .iter()
                .filter(|part| matches!(part, StreamPart::Finish { .. }))
                .count(),
            1
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    fn xai_body(model: &str, effort: Option<&str>, builtin_search: bool) -> Value {
        xai_body_with(model, effort, true, builtin_search)
    }

    fn xai_body_with(
        model: &str,
        effort: Option<&str>,
        thinking_enabled: bool,
        builtin_search: bool,
    ) -> Value {
        let request = GenerateRequest {
            model: model.into(),
            system: "你是 Kivio".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions {
                thinking_enabled,
                thinking_level: effort.map(str::to_string),
                builtin_web_search: builtin_search,
                ..Default::default()
            },
            metadata: crate::model::RequestMetadata {
                conversation_id: Some("conv_abc".into()),
                ..Default::default()
            },
        };
        request_body(
            &request,
            false,
            &OpenAiResponsesOptions {
                format: ResponsesFormat::Xai,
                ..Default::default()
            },
        )
    }

    #[test]
    fn xai_body_omits_what_is_useless_and_opts_out_of_storage() {
        let body = xai_body("grok-4.3", Some("high"), false);
        // `instructions` 搬进了 input（见下一条测试）；`prompt_cache_key` 对 xAI 没用
        // ——它的缓存靠 previous_response_id 自动做，不认这个键。
        for field in ["instructions", "prompt_cache_key"] {
            assert!(
                body.get(field).is_none(),
                "{field} must not be sent: {body}"
            );
        }
        // **必须显式关存储**：xAI 的 Responses `store` 默认 true、服务端留 30 天，
        // 而我们从不用 previous_response_id，那份副本纯属白存用户的对话。
        assert_eq!(
            body["store"], false,
            "must opt out of xAI server-side storage: {body}"
        );
    }

    #[test]
    fn xai_moves_the_system_prompt_into_input_instead_of_dropping_it() {
        let body = xai_body("grok-4.3", None, false);
        // 直接删 instructions 会让系统提示词凭空消失——必须搬进 input 首位。
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input[0]["role"], "system");
        assert_eq!(input[0]["content"][0]["text"], "你是 Kivio");
        assert_eq!(input[1]["role"], "user");
    }

    #[test]
    fn xai_maps_effort_onto_its_own_ladder() {
        assert_eq!(
            xai_body("grok-4.3", Some("minimal"), false)["reasoning"]["effort"],
            "low"
        );
        assert_eq!(
            xai_body("grok-4.3", Some("max"), false)["reasoning"]["effort"],
            "high"
        );
        assert_eq!(
            xai_body("grok-4.3", Some("xhigh"), false)["reasoning"]["effort"],
            "xhigh"
        );
        assert!(
            xai_body("grok-4.3", Some("high"), false)["reasoning"]
                .get("summary")
                .is_none(),
            "xAI must not get OpenAI reasoning.summary",
        );
        // 开思考但未设档 → 不发 reasoning（不擅自兜底）。
        assert!(xai_body("grok-4.3", None, false).get("reasoning").is_none());
        // Thinking disabled → 显式 effort:"none"（thinking_enabled=false，level=None）。
        let off = xai_body_with("grok-4.3", None, false, false);
        assert_eq!(off["reasoning"]["effort"], "none", "body: {off}");
    }

    #[test]
    fn xai_always_reasoning_models_use_lowest_effort_when_disabled() {
        for model in ["grok-4.5", "grok-4.6-latest", "grok-4.7"] {
            assert_eq!(
                xai_body_with(model, None, false, false)["reasoning"]["effort"],
                "low",
                "{model}"
            );
            assert_eq!(
                xai_body(model, Some("none"), false)["reasoning"]["effort"],
                "low",
                "{model}"
            );
        }
        assert_eq!(
            xai_body_with("grok-4.3", None, false, false)["reasoning"]["effort"],
            "none"
        );
    }

    #[test]
    fn xai_includes_server_tool_outputs() {
        // No include field is needed without hosted tools. Grok 4.7 returns
        // encrypted reasoning by default.
        assert!(xai_body("grok-4.3", None, false).get("include").is_none());
        // 开了内置搜索才发，且必须含来源字段——不 include 就拿不到 grok 的搜索来源。
        let searching = xai_body("grok-4.3", None, true);
        let include = searching["include"].as_array().expect("include");
        assert!(
            include.contains(&Value::String("web_search_call.action.sources".to_string())),
            "{searching}"
        );
    }

    #[test]
    fn openai_responses_is_untouched_by_the_xai_branch() {
        // 同一个适配器伺候两种协议，OpenAI 那条路必须逐字节照旧。
        let request = GenerateRequest {
            model: "gpt-5.5".into(),
            system: "你是 Kivio".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions {
                thinking_level: Some("high".into()),
                ..Default::default()
            },
            metadata: crate::model::RequestMetadata {
                conversation_id: Some("conv_abc".into()),
                ..Default::default()
            },
        };
        let body = request_body(
            &request,
            false,
            &OpenAiResponsesOptions {
                prompt_cache_key: Some("conv_abc".into()),
                ..Default::default()
            },
        );
        assert_eq!(body["instructions"], "你是 Kivio");
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert_eq!(body["prompt_cache_key"], "conv_abc");
        assert!(body.get("prompt_cache_retention").is_none());
        assert_eq!(body["input"][0]["role"], "user");
    }

    #[test]
    fn builtin_web_search_appends_responses_tool() {
        let base = GenerateRequest {
            model: "gpt-5.5".into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        };
        let encode =
            |request: &GenerateRequest, stream| request_body(request, stream, &Default::default());
        // off ⇒ 无 tools。
        let off = encode(&base, false);
        assert!(off.get("tools").is_none(), "body: {off}");
        // on ⇒ tools 含 {"type":"web_search"} + tool_choice=auto。
        let mut req = base.clone();
        req.options.builtin_web_search = true;
        let on = encode(&req, false);
        let tools = on["tools"].as_array().expect("tools array present");
        assert!(
            tools.iter().any(|t| t["type"] == "web_search"),
            "body: {on}"
        );
        assert_eq!(on["tool_choice"], "auto");
        // 开思考但无档位 ⇒ 不编造 effort，但仍要 summary + 无状态 encrypted replay。
        assert!(on["reasoning"].get("effort").is_none(), "body: {on}");
        assert_eq!(on["reasoning"]["summary"], "auto", "body: {on}");
        assert_eq!(on["store"], false, "body: {on}");
        assert_eq!(
            on["include"],
            serde_json::json!(["reasoning.encrypted_content"]),
            "body: {on}"
        );
        // 显式设 high ⇒ reasoning.effort=high。
        let mut req_high = base.clone();
        req_high.options.builtin_web_search = true;
        req_high.options.thinking_level = Some("high".into());
        let high = encode(&req_high, false);
        assert_eq!(high["reasoning"]["effort"], "high", "body: {high}");
        assert_eq!(high["reasoning"]["summary"], "auto", "body: {high}");
        // xhigh 原样下发（gpt-5.1-codex-max 起支持；哪些模型认由模型库 reasoningEfforts 门控，
        // 适配器不再按协议收敛成 high）。
        let mut req_x = base.clone();
        req_x.options.thinking_level = Some("xhigh".into());
        let xh = encode(&req_x, false);
        assert_eq!(xh["reasoning"]["effort"], "xhigh", "body: {xh}");
        assert_eq!(xh["reasoning"]["summary"], "auto", "body: {xh}");
        // 纯对话（开思考、无内置、无档）⇒ 同样要 summary / store / include，不编造 effort。
        assert!(off["reasoning"].get("effort").is_none(), "body: {off}");
        assert_eq!(off["reasoning"]["summary"], "auto", "body: {off}");
        assert_eq!(off["store"], false, "body: {off}");
        assert_eq!(
            off["include"],
            serde_json::json!(["reasoning.encrypted_content"]),
            "body: {off}"
        );
        // Thinking disabled → 显式 none（OpenAI / DeepSeek Responses 文档；省略会默认 high）。
        let mut req_off = base.clone();
        req_off.options.thinking_enabled = false;
        let thinking_off = encode(&req_off, false);
        assert_eq!(
            thinking_off["reasoning"]["effort"], "none",
            "body: {thinking_off}"
        );
        assert_eq!(thinking_off["store"], false, "body: {thinking_off}");
        // 关思考不需要 encrypted reasoning content。
        assert!(
            thinking_off.get("include").is_none(),
            "body: {thinking_off}"
        );

        // 发 reasoning 档位时同时发 store:false + include（与 Codex 官方客户端一致）：
        // 我们每轮自带完整 input，不依赖服务端会话状态，思考链走 encrypted_content。
        assert_eq!(high["store"], false, "body: {high}");
        assert_eq!(
            high["include"],
            serde_json::json!(["reasoning.encrypted_content"]),
            "body: {high}"
        );
        // 开思考但无档时同样走无状态 encrypted replay（与有档路径一致，只是不编造 effort）。
        assert_eq!(off["store"], false, "body: {off}");
        assert_eq!(
            off["include"],
            serde_json::json!(["reasoning.encrypted_content"]),
            "body: {off}"
        );
    }

    #[test]
    fn web_search_parsed_from_responses_output() {
        // web_search_call → query；message.content.annotations[url_citation] → 来源（去重）。
        let output = serde_json::json!([
            { "type": "web_search_call", "action": { "type": "search", "query": "kivio release" } },
            {
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "见来源。",
                    "annotations": [
                        { "type": "url_citation", "url": "https://a.com", "title": "A 站" },
                        { "type": "url_citation", "url": "https://a.com", "title": "重复应去掉" },
                        { "type": "url_citation", "url": "https://b.com" }
                    ]
                }]
            }
        ]);
        let parsed = web_search_from_responses_output(output.as_array().unwrap())
            .expect("web_search present");
        assert_eq!(parsed.queries, vec!["kivio release".to_string()]);
        assert_eq!(parsed.citations.len(), 2);
        assert_eq!(parsed.citations[0].title, "A 站");
        assert_eq!(parsed.citations[0].url, "https://a.com");
        // 缺 title 时回退用 url。
        assert_eq!(parsed.citations[1].title, "https://b.com");
    }

    #[test]
    fn web_search_none_without_search_traces() {
        let output = serde_json::json!([
            { "type": "message", "content": [{ "type": "output_text", "text": "无搜索" }] }
        ]);
        assert!(web_search_from_responses_output(output.as_array().unwrap()).is_none());
    }

    #[test]
    fn web_search_sources_fallback_only_without_citations() {
        // grok(xAI):action.sources[] 是检索命中 URL。无正文 url_citation 时兜底转正;
        // 有注解时以注解为准、sources 丢弃。
        let sources_only = serde_json::json!([
            { "type": "web_search_call", "action": { "type": "search", "query": "nvda close",
                "sources": [
                    { "type": "url", "url": "https://finance.yahoo.com/quote/NVDA/" },
                    { "type": "url", "url": "https://finance.yahoo.com/quote/NVDA/" },
                    { "type": "url", "url": "https://www.wsj.com/market-data/quotes/NVDA" }
                ] } },
            { "type": "message", "content": [{ "type": "output_text", "text": "无注解正文" }] }
        ]);
        let parsed = web_search_from_responses_output(sources_only.as_array().unwrap())
            .expect("web_search present");
        assert_eq!(parsed.citations.len(), 2, "sources 去重后兜底转正");
        assert_eq!(
            parsed.citations[0].url,
            "https://finance.yahoo.com/quote/NVDA/"
        );

        // 有 url_citation ⇒ 注解优先,sources 不混入。
        let with_annotation = serde_json::json!([
            { "type": "web_search_call", "action": { "type": "search", "query": "q",
                "sources": [ { "type": "url", "url": "https://ignored.example.com" } ] } },
            { "type": "message", "content": [{ "type": "output_text", "text": "见[1]",
                "annotations": [ { "type": "url_citation", "url": "https://cited.example.com", "title": "Cited" } ] }] }
        ]);
        let parsed = web_search_from_responses_output(with_annotation.as_array().unwrap())
            .expect("web_search present");
        assert_eq!(parsed.citations.len(), 1);
        assert_eq!(parsed.citations[0].url, "https://cited.example.com");
    }

    #[test]
    fn sse_stream_hosted_image_generation_is_captured_with_empty_text() {
        // 真机形态（xb1520 网关 + gpt-5.6）：模型自行走 hosted image_generation_call 回答，
        // message 的 output_text 是空串（图即答案），且没有任何 output_text.delta。
        // 不收 image_generation_call.result 会让整轮变成「空助手响应」而报错。
        let body = concat!(
            "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"ig_1\",\"type\":\"image_generation_call\",\"status\":\"completed\",\"action\":\"generate\",\"output_format\":\"png\",\"result\":\"aGVsbG8=\"}}\n\n",
            "data: {\"type\":\"response.output_text.done\",\"output_index\":2,\"text\":\"\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"\"}]}]}}\n\n",
            "data: [DONE]\n\n",
        );
        let output = output_from_sse_body(body, false, "").expect("sse output");
        assert!(output.text.trim().is_empty(), "正文确实是空的");
        assert_eq!(output.images.len(), 1, "hosted 出图必须被收下");
        assert_eq!(output.images[0].mime_type, "image/png");
        assert_eq!(output.images[0].data, "aGVsbG8=");
    }

    #[test]
    fn nonstream_hosted_image_generation_is_captured() {
        // 非流式同形态：output[] 里的 image_generation_call 也要出图，mime 按 output_format。
        let value = serde_json::json!({
            "status": "completed",
            "output": [
                { "type": "image_generation_call", "output_format": "jpeg", "result": "aGVsbG8=" },
                { "type": "message", "content": [{ "type": "output_text", "text": "" }] }
            ]
        });
        let output = output_from_responses(&value, "{}", "test").expect("output");
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].mime_type, "image/jpeg");
    }

    #[test]
    fn stream_reasoning_items_with_encrypted_content_are_captured() {
        // 带密文的 reasoning item 要整只收下（回放思维链）；没有密文的不收——
        // store:false 下回放无密文 item 会被服务端当引用查找而报错。completed 里
        // 重现的同 id item 按 id 去重，不重复收。
        let (_parts, output) = run_events(&[
            serde_json::json!({
                "type": "response.output_item.done",
                "item": { "id": "rs_1", "type": "reasoning", "summary": [], "encrypted_content": "gAAA" }
            }),
            serde_json::json!({
                "type": "response.output_item.done",
                "item": { "id": "rs_2", "type": "reasoning", "summary": [] }
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": { "status": "completed", "output": [
                    { "id": "rs_1", "type": "reasoning", "summary": [], "encrypted_content": "gAAA" },
                    { "type": "message", "content": [{ "type": "output_text", "text": "ok" }] }
                ] }
            }),
        ]);
        assert_eq!(
            output.reasoning_items.len(),
            1,
            "只收带密文的，且按 id 去重"
        );
        assert_eq!(output.reasoning_items[0].item["id"], "rs_1");
        assert_eq!(output.reasoning_items[0].item["encrypted_content"], "gAAA");
    }

    #[test]
    fn nonstream_reasoning_items_are_captured_and_model_stamped() {
        let value = serde_json::json!({
            "status": "completed",
            "output": [
                { "id": "rs_1", "type": "reasoning", "summary": [], "encrypted_content": "gAAA" },
                { "type": "message", "content": [{ "type": "output_text", "text": "hi" }] }
            ]
        });
        let mut output = output_from_responses(&value, "{}", "test").expect("output");
        assert_eq!(output.reasoning_items.len(), 1);
        assert_eq!(output.reasoning_items[0].model, "", "解析层拿不到请求模型");
        stamp_reasoning_item_model(&mut output, "gpt-5.6");
        assert_eq!(output.reasoning_items[0].model, "gpt-5.6");
    }

    #[test]
    fn sse_stream_sources_fallback_when_no_citation_arrives() {
        // 流式:web_search_call(带 sources)到达但整轮无 url_citation(grok 岔去客户端
        // fetch 的典型形态)⇒ finish 时 sources 兜底进 citations。
        let body = concat!(
            "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"nvda close\",\"sources\":[{\"type\":\"url\",\"url\":\"https://finance.yahoo.com/quote/NVDA/\"},{\"type\":\"url\",\"url\":\"https://www.cnbc.com/quotes/NVDA\"}]}}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer\"}]}]}}\n\n",
            "data: [DONE]\n\n",
        );
        let output = output_from_sse_body(body, false, "").expect("sse output");
        let ws = output.web_search.expect("web_search present");
        assert_eq!(ws.queries, vec!["nvda close".to_string()]);
        assert_eq!(ws.citations.len(), 2, "无注解时 sources 兜底");
        assert_eq!(ws.citations[0].url, "https://finance.yahoo.com/quote/NVDA/");
    }

    fn run_events(events: &[Value]) -> (Vec<StreamPart>, GenerateOutput) {
        let mut parts = Vec::new();
        let mut state = ResponsesStreamState::default();
        let mut sink = |part: StreamPart| {
            parts.push(part);
            Ok(())
        };
        for event in events {
            handle_responses_stream_event(event, &mut state, &mut sink).expect("event");
        }
        let output = state.finish(&mut sink).expect("finish");
        (parts, output)
    }

    #[test]
    fn stream_function_call_arguments_are_captured() {
        let events = vec![
            serde_json::json!({
                "type": "response.output_item.added",
                "output_index": 1,
                "item": { "id": "fc_1", "type": "function_call", "status": "in_progress", "arguments": "", "call_id": "call_abc", "name": "web_search" }
            }),
            serde_json::json!({
                "type": "response.function_call_arguments.done",
                "arguments": "{\"query\":\"吉林市 明天 天气\"}",
                "item_id": "fc_1",
                "output_index": 1
            }),
            serde_json::json!({
                "type": "response.output_item.done",
                "output_index": 1,
                "item": { "id": "fc_1", "type": "function_call", "status": "completed", "arguments": "{\"query\":\"吉林市 明天 天气\"}", "call_id": "call_abc", "name": "web_search" }
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": { "status": "completed", "usage": { "input_tokens": 10, "output_tokens": 5, "total_tokens": 15 } }
            }),
        ];
        let (parts, output) = run_events(&events);

        assert_eq!(output.tool_calls.len(), 1);
        let call = &output.tool_calls[0];
        assert_eq!(call.id, "call_abc");
        assert_eq!(call.function_name, "web_search");
        assert_eq!(call.arguments["query"], "吉林市 明天 天气");
        assert!(call.arguments_parse_error.is_none());
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("tool_calls")
        );
        assert_eq!(output.usage.and_then(|u| u.total_tokens), Some(15));
        assert!(parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolCallStart { .. })));
        assert!(parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolCallDone { .. })));
    }

    #[test]
    fn stream_text_deltas_accumulate() {
        let events = vec![
            serde_json::json!({ "type": "response.output_text.delta", "delta": "你好" }),
            serde_json::json!({ "type": "response.output_text.delta", "delta": "，世界" }),
            serde_json::json!({ "type": "response.completed", "response": { "status": "completed" } }),
        ];
        let (parts, output) = run_events(&events);
        assert_eq!(output.text, "你好，世界");
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("stop")
        );
        assert_eq!(
            parts
                .iter()
                .filter(|p| matches!(p, StreamPart::TextDelta { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn stream_reasoning_summary_parts_are_separated() {
        // GPT-5 / gpt-5.6-sol 的 reasoning summary 是 `summary[]` 里多条
        // `summary_text`。官方流是 part.added → text.delta* → part.done，换 part
        // 时 `summary_index` 递增。每条 concise 标题通常没有尾换行，如果只把
        // delta 首尾相接，界面上就会变成「constraintsAnalyzing」这种粘连段。
        let (_parts, output) = run_events(&[
            serde_json::json!({
                "type": "response.reasoning_summary_part.added",
                "item_id": "rs_1",
                "output_index": 0,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": "" }
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": "rs_1",
                "output_index": 0,
                "summary_index": 0,
                "delta": "Parsing table and defining drawing constraints"
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_part.added",
                "item_id": "rs_1",
                "output_index": 0,
                "summary_index": 1,
                "part": { "type": "summary_text", "text": "" }
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": "rs_1",
                "output_index": 0,
                "summary_index": 1,
                "delta": "Analyzing shape-based drawing strategy for guarantee"
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": "rs_1",
                "output_index": 0,
                "summary_index": 1,
                "delta": " by feel"
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": { "status": "completed" }
            }),
        ]);
        assert_eq!(
            output.reasoning.as_deref(),
            Some(
                "Parsing table and defining drawing constraints\n\n\
                 Analyzing shape-based drawing strategy for guarantee by feel"
            )
        );
    }

    #[test]
    fn stream_reasoning_summary_index_separates_without_part_added() {
        // 有些中转只推 text.delta + summary_index，不发 part.added。
        let (_parts, output) = run_events(&[
            serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "summary_index": 0,
                "delta": "Establishing 21 as minimal guarantee"
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "summary_index": 1,
                "delta": "Confirming rounds guarantee A and P"
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": { "status": "completed" }
            }),
        ]);
        assert_eq!(
            output.reasoning.as_deref(),
            Some("Establishing 21 as minimal guarantee\n\nConfirming rounds guarantee A and P")
        );
    }

    #[test]
    fn non_stream_output_parses_text_and_tool_call() {
        let value = serde_json::json!({
            "status": "completed",
            "output": [
                { "type": "function_call", "call_id": "call_x", "name": "web_search", "arguments": "{\"query\":\"a\"}" }
            ],
            "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
        });
        let output = output_from_responses(&value, "{}", "test").expect("output");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].arguments["query"], "a");
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("tool_calls")
        );

        let text_value = serde_json::json!({
            "status": "completed",
            "output": [ { "type": "message", "content": [ { "type": "output_text", "text": "hi" } ] } ]
        });
        let out = output_from_responses(&text_value, "{}", "test").expect("output");
        assert_eq!(out.text, "hi");
        assert_eq!(
            out.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("stop")
        );
    }

    #[test]
    fn sse_body_fallback_does_not_commit_unterminated_completion() {
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n",
        );
        let error =
            output_from_sse_body(body, false, "").expect_err("blank line is required to commit");
        assert!(error.is_retryable_stream_error());
    }

    #[test]
    fn sse_body_fallback_preserves_multiline_event_data() {
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\r\n",
            "data: \"delta\":\"你好\"}\r\n\r\n",
            "data: {\"type\":\"response.completed\",\r",
            "data: \"response\":{\"status\":\"completed\"}}\r\r",
        );
        let output = output_from_sse_body(body, false, "").expect("complete multiline SSE");
        assert_eq!(output.text, "你好");
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("stop")
        );
    }

    /// A provider that streams an SSE body despite `stream:false`. `output_from_sse_body`
    /// must reuse the streaming accumulation and yield the same `GenerateOutput` the live
    /// stream path produces — text/tool-call args/usage — instead of a JSON parse error.
    #[test]
    fn sse_body_fallback_parses_tool_call_and_usage() {
        let body = concat!(
            "event: response.created\r\n",
            "data: {\"type\":\"response.created\",\"response\":{\"status\":\"in_progress\"}}\r\n",
            "\r\n",
            "event: response.output_item.added\r\n",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"status\":\"in_progress\",\"arguments\":\"\",\"call_id\":\"call_abc\",\"name\":\"web_search\"}}\r\n",
            "\r\n",
            "event: response.function_call_arguments.done\r\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"arguments\":\"{\\\"query\\\":\\\"吉林市 明天 天气\\\"}\",\"item_id\":\"fc_1\",\"output_index\":1}\r\n",
            "\r\n",
            "event: response.output_item.done\r\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"status\":\"completed\",\"arguments\":\"{\\\"query\\\":\\\"吉林市 明天 天气\\\"}\",\"call_id\":\"call_abc\",\"name\":\"web_search\"}}\r\n",
            "\r\n",
            "event: response.completed\r\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"total_tokens\":15}}}\r\n",
            "\r\n",
            "data: [DONE]\r\n",
        );
        assert!(looks_like_sse(body));
        let output = output_from_sse_body(body, false, "").expect("sse output");
        assert_eq!(output.tool_calls.len(), 1);
        let call = &output.tool_calls[0];
        assert_eq!(call.id, "call_abc");
        assert_eq!(call.function_name, "web_search");
        assert_eq!(call.arguments["query"], "吉林市 明天 天气");
        assert!(call.arguments_parse_error.is_none());
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("tool_calls")
        );
        assert_eq!(output.usage.and_then(|u| u.total_tokens), Some(15));
    }

    /// 内置搜索(hosted web_search)流式捕获:web_search_call 走 output_item.done、
    /// 来源走 output_text.annotation.added——都不重现在 response.completed 的 output 里
    /// (completed 只剩 message),故必须边流边收。此测试钉住这条真实 wire 行为。
    #[test]
    fn sse_stream_captures_builtin_web_search_from_events() {
        let body = concat!(
            "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"queries\":[\"TSLA latest close\"],\"query\":\"TSLA latest close\"}}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n",
            "data: {\"type\":\"response.output_text.annotation.added\",\"annotation\":{\"type\":\"url_citation\",\"title\":\"ChartExchange\",\"url\":\"https://chartexchange.com/x\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer\"}]}]}}\n\n",
            "data: [DONE]\n\n",
        );
        let output = output_from_sse_body(body, false, "").expect("sse output");
        let ws = output
            .web_search
            .expect("web_search captured from stream events");
        assert_eq!(ws.queries, vec!["TSLA latest close".to_string()]);
        assert_eq!(ws.citations.len(), 1);
        assert_eq!(ws.citations[0].url, "https://chartexchange.com/x");
        assert_eq!(ws.citations[0].title, "ChartExchange");
    }

    #[test]
    fn sse_stream_emits_web_search_parts_for_realtime_card() {
        #[derive(Default)]
        struct CapturingSink {
            parts: Vec<StreamPart>,
        }
        impl StreamSink for CapturingSink {
            fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
                self.parts.push(part);
                Ok(())
            }
        }

        let body = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"status\":\"in_progress\"}}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"queries\":[\"TSLA latest close\"],\"query\":\"TSLA latest close\"}}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n",
            "data: {\"type\":\"response.output_text.annotation.added\",\"annotation\":{\"type\":\"url_citation\",\"title\":\"ChartExchange\",\"url\":\"https://chartexchange.com/x\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer\"}]}]}}\n\n",
            "data: [DONE]\n\n",
        );
        let mut state = ResponsesStreamState::default();
        let mut sink = CapturingSink::default();
        for data in crate::model::sse::SseDecoder::default()
            .push(body.as_bytes())
            .unwrap()
        {
            process_sse_data(&data, &mut state, &mut sink).expect("process event");
        }
        state.finish(&mut sink).expect("finish");

        let web_search_parts: Vec<(Vec<String>, Vec<WebCitation>)> = sink
            .parts
            .iter()
            .filter_map(|part| match part {
                StreamPart::WebSearch { queries, citations } => {
                    Some((queries.clone(), citations.clone()))
                }
                _ => None,
            })
            .collect();
        // 开牌 + 查询 + 来源 + finish 终帧同步 = 4 帧(completed 合并不发,finish 无条件发终帧)。
        assert_eq!(web_search_parts.len(), 4);
        // 首帧空快照 = 开牌。
        assert!(web_search_parts[0].0.is_empty() && web_search_parts[0].1.is_empty());
        // 末帧累加了查询与来源。
        let last = web_search_parts.last().expect("last frame");
        assert_eq!(last.0, vec!["TSLA latest close".to_string()]);
        assert_eq!(last.1.len(), 1);
        assert_eq!(last.1[0].url, "https://chartexchange.com/x");
    }

    /// SSE body carrying only text deltas accumulates the same as the live stream path.
    #[test]
    fn sse_body_fallback_accumulates_text() {
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"，世界\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
        );
        let output = output_from_sse_body(body, false, "").expect("sse output");
        assert_eq!(output.text, "你好，世界");
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("stop")
        );
    }

    /// Both official top-level error codes and nested relay codes preserve
    /// their retryability; unknown errors remain failures without auto-replay.
    #[test]
    fn sse_body_fallback_surfaces_provider_error() {
        for (event, retryable) in [
            (
                serde_json::json!({"type":"response.failed","response":{"error":{"message":"boom"}}}),
                false,
            ),
            (
                serde_json::json!({"type":"error","code":"server_error","message":"boom"}),
                true,
            ),
            (
                serde_json::json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"boom"}}}),
                true,
            ),
            (
                serde_json::json!({"type":"error","code":"invalid_api_key","message":"boom"}),
                false,
            ),
        ] {
            let err = output_from_sse_body(&format!("data: {event}\n\n"), false, "")
                .expect_err("provider failure");
            assert!(err.to_string().contains("boom"));
            assert_eq!(err.is_retryable_stream_error(), retryable, "{event}");
        }
    }

    /// Partial text cannot prove completion; relay missing-terminal errors must retry.
    #[test]
    fn missing_completed_event_after_text_is_interrupted() {
        for error in [
            serde_json::json!({"type":"error","error":{"message":"stream ended without terminal event or completed response"}}),
            serde_json::json!({"type":"error","message":"stream ended without terminal event or completed response"}),
            serde_json::json!({"type":"response.failed","response":{"error":{"message":"stream ended without terminal event or completed response"}}}),
        ] {
            let body = format!("data: {{\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}}\n\ndata: {error}\n\n");
            let err = output_from_sse_body(&body, false, "")
                .expect_err("partial stream must not count as success");
            assert!(err.is_retryable_stream_error(), "{err}");
        }
    }

    /// 没有可用输出时，同一条缺终态 error 仍是失败——不能把空流当成成功。
    #[test]
    fn missing_completed_event_without_output_is_still_error() {
        let body = concat!(
            "data: {\"type\":\"error\",\"message\":\"stream ended without terminal event or completed response\"}\n\n",
        );
        let err = output_from_sse_body(body, false, "").expect_err("empty stream should fail");
        assert!(err.to_string().contains("without terminal"), "got {err}");
    }

    /// A plain JSON body is not mistaken for SSE; the happy path stays unchanged.
    #[test]
    fn plain_json_body_is_not_sse() {
        let body = r#"{"status":"completed","output":[]}"#;
        assert!(!looks_like_sse(body));
    }

    /// Gap 4: a non-stream Responses error object surfaces the provider's real reason.
    #[test]
    fn output_from_responses_surfaces_real_error_message() {
        let value: Value = serde_json::from_str(r#"{"error":{"message":"boom"}}"#).unwrap();
        let err = output_from_responses(&value, "{}", "Chat planning").expect_err("error");
        assert!(err.to_string().contains("boom"), "got {err}");
        assert!(!err.to_string().contains("Unknown"));
    }

    #[test]
    fn output_from_responses_ignores_null_error_on_success() {
        let value = serde_json::json!({
            "status": "completed",
            "error": serde_json::Value::Null,
            "output": [{
                "type": "message",
                "content": [{ "type": "output_text", "text": "ok" }]
            }]
        });
        let output = output_from_responses(&value, "{}", "test").expect("output");
        assert_eq!(output.text, "ok");
    }

    /// Gap 4: error message extraction falls through message → code → type → JSON, and
    /// only uses the generic fallback when the error object is truly empty.
    #[test]
    fn responses_error_message_falls_through_fields() {
        assert_eq!(
            responses_error_message(&serde_json::json!({"message": "rate limited"})),
            "rate limited"
        );
        assert_eq!(
            responses_error_message(&serde_json::json!({"code": "context_length_exceeded"})),
            "context_length_exceeded"
        );
        assert_eq!(
            responses_error_message(&serde_json::json!({"type": "server_error"})),
            "server_error"
        );
        // No standard field but a non-empty object → compact JSON is surfaced.
        let json_only = responses_error_message(&serde_json::json!({"detail": "502 bad gateway"}));
        assert!(json_only.contains("502 bad gateway"), "got {json_only}");
        // Truly empty error object → generic fallback (last resort only).
        assert_eq!(
            responses_error_message(&serde_json::json!({})),
            "Unknown Responses API error"
        );
    }

    /// Responses 的输出截断（status=incomplete + reason=max_output_tokens）必须归一化为
    /// "length"，否则「finish_reason==length ⇒ 整批工具调用作废」防护对 Responses 系失效。
    #[test]
    fn incomplete_max_output_tokens_normalizes_to_length() {
        let has_tool_calls = false;
        let truncated = serde_json::json!({
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" }
        });
        assert_eq!(
            responses_finish_reason("incomplete", &truncated, has_tool_calls).legacy_name(),
            "length"
        );
        // 其它 incomplete 原因（如 content_filter）原样透传，不冒充截断。
        let filtered = serde_json::json!({
            "status": "incomplete",
            "incomplete_details": { "reason": "content_filter" }
        });
        assert_eq!(
            responses_finish_reason("incomplete", &filtered, has_tool_calls).legacy_name(),
            "incomplete"
        );
        // completed 的两条既有映射不受影响。
        let done = serde_json::json!({ "status": "completed" });
        assert_eq!(
            responses_finish_reason("completed", &done, has_tool_calls).legacy_name(),
            "stop"
        );
    }
}
