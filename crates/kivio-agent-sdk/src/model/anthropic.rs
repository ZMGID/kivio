//! Shared Anthropic Messages wire protocol and host-configured HTTP provider.
use super::error::provider_error_code;
use super::usage::model_usage_from_anthropic_value;
use super::{
    parse_tool_arguments, stream_read_error, BuiltinWebSearch, FinishKind, FinishReason,
    GenerateOutput, GenerateRequest, LanguageModelProvider, MessagePart, ModelError,
    ModelErrorKind, ModelFuture, ModelMessage, ModelRole, ModelTool, ModelUsage, PendingToolCall,
    StreamPart, StreamSink, WebCitation,
};
use serde_json::Value;

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const EXTENDED_CACHE_TTL_BETA: &str = "extended-cache-ttl-2025-04-11";
pub const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnthropicThinkingMode {
    #[default]
    Adaptive,
    Extended {
        output_effort: bool,
    },
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnthropicCacheTtl {
    #[default]
    Default,
    OneHour,
}

/// Host-resolved wire defaults. Encoded call options override extra_body, then
/// per-call provider_options apply; ToolChoice::None has final precedence.
#[derive(Clone, Debug)]
pub struct AnthropicMessagesOptions {
    pub temperature: Option<f64>,
    pub default_max_tokens: u32,
    pub thinking_mode: AnthropicThinkingMode,
    pub off_thinking_type: Option<String>,
    pub allow_unsigned_thinking: bool,
    /// Opaque host identity for native replay isolation. None preserves legacy unscoped behavior.
    pub provider_id: Option<String>,
    pub system_prefix: Option<String>,
    pub cache: Option<AnthropicCacheTtl>,
    pub extra_body: Value,
}

impl Default for AnthropicMessagesOptions {
    fn default() -> Self {
        Self {
            temperature: None,
            default_max_tokens: 16_384,
            thinking_mode: Default::default(),
            off_thinking_type: None,
            allow_unsigned_thinking: false,
            provider_id: None,
            system_prefix: None,
            cache: None,
            extra_body: Value::Null,
        }
    }
}

/// Explicit transport configuration with a complete Messages endpoint URL.
pub struct AnthropicMessagesConfig {
    pub endpoint: reqwest::Url,
    pub api_key: Option<String>,
    pub headers: reqwest::header::HeaderMap,
    pub options: AnthropicMessagesOptions,
    /// Applies to each buffered response. Configure stream idle timeouts on the client.
    pub request_timeout: Option<std::time::Duration>,
}

impl AnthropicMessagesConfig {
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

/// Completes bounded native server-tool continuations. Each HTTP attempt sends once,
/// without transport retry, credential rotation, or host accounting.
pub struct AnthropicMessagesProvider {
    client: reqwest::Client,
    config: AnthropicMessagesConfig,
}

impl AnthropicMessagesProvider {
    pub fn new(client: reqwest::Client, config: AnthropicMessagesConfig) -> Self {
        Self { client, config }
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
                "Anthropic Messages does not support video input",
                ModelErrorKind::InvalidRequest,
            ));
        }
        let options = AnthropicMessagesOptions {
            temperature: request
                .options
                .temperature
                .or(self.config.options.temperature),
            ..self.config.options.clone()
        };
        let mut headers = self.config.headers.clone();
        let betas = feature_betas(request, &options);
        if !betas.is_empty() {
            let mut merged: Vec<String> = headers
                .get("anthropic-beta")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
            for beta in betas {
                if !merged.iter().any(|value| value == beta) {
                    merged.push(beta.to_string());
                }
            }
            headers.insert(
                "anthropic-beta",
                merged.join(",").parse().map_err(|error| {
                    ModelError::with_kind(
                        format!("Invalid Anthropic beta header: {error}"),
                        ModelErrorKind::InvalidRequest,
                    )
                })?,
            );
        }
        let mut http = self
            .client
            .post(self.config.endpoint.clone())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .headers(headers)
            .json(&request_body(request, stream, &options));
        if let Some(key) = &self.config.api_key {
            http = http.header("x-api-key", key);
        }
        if !stream {
            if let Some(timeout) = self.config.request_timeout {
                http = http.timeout(timeout);
            }
        }
        super::transport::send(http, &request_label(request, "Anthropic Messages API")).await
    }

    async fn generate_inner(&self, request: GenerateRequest) -> Result<GenerateOutput, ModelError> {
        let label = request_label(&request, "Anthropic Messages API");
        let raw = self
            .post(&request, false)
            .await?
            .text()
            .await
            .map_err(|error| stream_read_error(&label, &error))?;
        decode_anthropic_body(
            &raw,
            &label,
            &request.model,
            self.config.options.provider_id.as_deref(),
            None,
        )
    }

    async fn stream_inner(
        &self,
        request: GenerateRequest,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<GenerateOutput, ModelError> {
        let label = request_label(&request, "Anthropic stream");
        decode_anthropic_stream(
            self.post(&request, true).await?,
            sink,
            &label,
            &request.model,
            self.config.options.provider_id.as_deref(),
            None,
        )
        .await
    }
}

struct SingleRequest<'a>(&'a AnthropicMessagesProvider);
impl LanguageModelProvider for SingleRequest<'_> {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(self.0.generate_inner(request))
    }
    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(self.0.stream_inner(request, sink))
    }
}

impl LanguageModelProvider for AnthropicMessagesProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move { complete_anthropic_turn(&SingleRequest(self), request, None).await })
    }
    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(
            async move { complete_anthropic_turn(&SingleRequest(self), request, Some(sink)).await },
        )
    }
}

impl crate::agent::provider_runtime::ModelRuntime for AnthropicMessagesProvider {
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

pub fn request_body(
    request: &GenerateRequest,
    stream: bool,
    options: &AnthropicMessagesOptions,
) -> Value {
    let max_tokens = required_max_tokens(request.options.max_tokens, options.default_max_tokens);
    let mut body = serde_json::json!({
        "model": request.model,
        "messages": anthropic_messages_with_unsigned_thinking(request, options.allow_unsigned_thinking, options.provider_id.as_deref()),
    });
    if let Some(extra) = options.extra_body.as_object() {
        for (key, value) in extra {
            body[key] = value.clone();
        }
    }
    body["max_tokens"] = Value::from(max_tokens);
    if let Some(temperature) = options.temperature {
        body["temperature"] = serde_json::json!(temperature);
    }
    let mut system = Vec::new();
    if let Some(prefix) = &options.system_prefix {
        system.push(serde_json::json!({ "type":"text", "text":prefix }));
    }
    if !request.system.trim().is_empty() {
        system.push(serde_json::json!({ "type":"text", "text":request.system }));
    }
    if !system.is_empty() {
        body["system"] = Value::Array(system);
    }
    if stream {
        body["stream"] = Value::Bool(true);
    }
    let mut tools = anthropic_tools_from_model_tools(&request.tools);
    if request.options.builtin_web_search {
        tools.push(serde_json::json!({ "type":"web_search_20250305", "name":"web_search" }));
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    apply_anthropic_thinking(&mut body, request, options);
    if body.get("thinking").is_some() {
        body.as_object_mut().unwrap().remove("temperature");
    }
    if let Some(overrides) = request.options.provider_options.as_object() {
        for (key, value) in overrides {
            body[key] = value.clone();
        }
    }
    if request.options.tool_choice == super::ToolChoice::None {
        body.as_object_mut().unwrap().remove("tools");
        body["tool_choice"] = serde_json::json!({ "type":"none" });
    }
    if let Some(cache) = options.cache {
        let control = match cache {
            AnthropicCacheTtl::Default => serde_json::json!({ "type":"ephemeral" }),
            AnthropicCacheTtl::OneHour => serde_json::json!({ "type":"ephemeral", "ttl":"1h" }),
        };
        apply_prompt_cache_breakpoints(&mut body, &control);
    }
    body
}

pub fn feature_betas(
    request: &GenerateRequest,
    options: &AnthropicMessagesOptions,
) -> Vec<&'static str> {
    let mut body = serde_json::json!({});
    apply_anthropic_thinking(&mut body, request, options);
    let mut betas = Vec::new();
    if body["thinking"]["type"] == "enabled" {
        betas.push(INTERLEAVED_THINKING_BETA);
    }
    if options.cache == Some(AnthropicCacheTtl::OneHour) {
        betas.push(EXTENDED_CACHE_TTL_BETA);
    }
    betas
}

fn required_max_tokens(requested: u32, default: u32) -> u32 {
    if requested > 0 {
        requested
    } else {
        default
    }
}

fn apply_anthropic_thinking(
    body: &mut Value,
    request: &GenerateRequest,
    options: &AnthropicMessagesOptions,
) {
    if !request.options.thinking_enabled {
        if let Some(kind) = &options.off_thinking_type {
            body["thinking"] = serde_json::json!({ "type":kind });
        }
        return;
    }
    let Some(level) = request.options.thinking_level.as_deref() else {
        return;
    };
    match options.thinking_mode {
        AnthropicThinkingMode::Unsupported => {}
        AnthropicThinkingMode::Adaptive => {
            body["thinking"] = serde_json::json!({ "type":"adaptive", "display":"summarized" });
            body["output_config"]["effort"] = serde_json::json!(level);
        }
        AnthropicThinkingMode::Extended { output_effort } => {
            let Some(budget) = clamp_extended_thinking_budget(
                extended_thinking_budget_for_level(level),
                required_max_tokens(request.options.max_tokens, options.default_max_tokens),
            ) else {
                return;
            };
            body["thinking"] = serde_json::json!({ "type":"enabled", "budget_tokens":budget, "display":"summarized" });
            if output_effort && matches!(level, "low" | "medium" | "high") {
                body["output_config"]["effort"] = serde_json::json!(level);
            }
        }
    }
}

fn mapped_tool_name(name: &str, map: Option<&(dyn Fn(&str) -> String + Sync)>) -> String {
    map.map_or_else(|| name.to_string(), |map| map(name))
}

fn bind_output_provenance(output: &mut GenerateOutput, model: &str, provider_id: Option<&str>) {
    if let Some(message) = &mut output.canonical_message {
        for part in &mut message.content {
            match part {
                MessagePart::AnthropicThinking {
                    source_model,
                    source_provider_id,
                    ..
                }
                | MessagePart::AnthropicRedactedThinking {
                    source_model,
                    source_provider_id,
                    ..
                }
                | MessagePart::AnthropicNativeBlock {
                    source_model,
                    source_provider_id,
                    ..
                } => {
                    *source_model = Some(model.to_string());
                    *source_provider_id = provider_id.map(str::to_string);
                }
                _ => {}
            }
        }
    }
}

pub fn decode_anthropic_body(
    raw: &str,
    label: &str,
    model: &str,
    provider_id: Option<&str>,
    map_tool_name: Option<&(dyn Fn(&str) -> String + Sync)>,
) -> Result<GenerateOutput, ModelError> {
    let mut value: Value = serde_json::from_str(raw).map_err(|error| {
        ModelError::new(format!(
            "{label} parse JSON: {error} (body: {})",
            raw.chars().take(500).collect::<String>()
        ))
    })?;
    if let Some(blocks) = value.get_mut("content").and_then(Value::as_array_mut) {
        for block in blocks
            .iter_mut()
            .filter(|block| block["type"] == "tool_use")
        {
            if let Some(Value::String(name)) = block.get_mut("name") {
                *name = mapped_tool_name(name, map_tool_name);
            }
        }
    }
    let mut output = output_from_anthropic_message(&value, label)?;
    bind_output_provenance(&mut output, model, provider_id);
    Ok(output)
}

pub async fn decode_anthropic_stream(
    mut response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
    model: &str,
    provider_id: Option<&str>,
    map_tool_name: Option<&(dyn Fn(&str) -> String + Sync)>,
) -> Result<GenerateOutput, ModelError> {
    let mut sse = super::sse::SseDecoder::default();
    let mut content_blocks = AnthropicContentAccumulator::default();
    let mut full = String::new();
    let mut reasoning_full = String::new();
    let mut tool_calls = Vec::new();
    let mut current_tool_id = String::new();
    let mut current_tool_name = String::new();
    let mut current_tool_input_parts: Vec<String> = Vec::new();
    let mut current_tool_initial_input = serde_json::json!({});
    let mut finish_reason = FinishReason::new(FinishKind::Stop, "");
    let mut usage: Option<ModelUsage> = None;
    // 内置搜索引用（服务端块尽力收集，随 content_block_start 完整到达）。
    let mut web_search: Option<BuiltinWebSearch> = None;

    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|err| stream_read_error(label, &err))?;
        let Some(chunk) = chunk else {
            break;
        };
        for data in sse.push(&chunk) {
            content_blocks.push(&data);
            match parse_anthropic_sse_event(&format!("data: {data}")) {
                Some(AnthropicSseEvent::Usage(next)) => {
                    merge_stream_usage(&mut usage, next, sink)?;
                }
                Some(AnthropicSseEvent::TextDelta(text)) => {
                    full.push_str(&text);
                    sink.emit(StreamPart::TextDelta { delta: text })?;
                }
                Some(AnthropicSseEvent::ThinkingDelta(thinking)) => {
                    reasoning_full.push_str(&thinking);
                    sink.emit(StreamPart::ReasoningDelta { delta: thinking })?;
                }
                Some(AnthropicSseEvent::ToolUseStart { id, name, input }) => {
                    let name = mapped_tool_name(&name, map_tool_name);
                    sink.emit(StreamPart::ToolCallStart {
                        id: id.clone(),
                        name: name.clone(),
                    })?;
                    current_tool_id = id;
                    current_tool_name = name;
                    current_tool_initial_input = input;
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
                            &current_tool_initial_input,
                        );
                        sink.emit(StreamPart::ToolCallDone { call: call.clone() })?;
                        tool_calls.push(call);
                    }
                    current_tool_id.clear();
                    current_tool_name.clear();
                    current_tool_input_parts.clear();
                }
                Some(AnthropicSseEvent::WebSearch(ws)) => {
                    sink.emit(StreamPart::WebSearch {
                        queries: ws.queries.clone(),
                        citations: ws.citations.clone(),
                    })?;
                    merge_anthropic_web_search(&mut web_search, ws);
                }
                Some(AnthropicSseEvent::MessageStop) => {
                    sink.emit(StreamPart::Finish {
                        reason: finish_reason.clone(),
                        full: full.clone(),
                    })?;
                    let mut output =
                        stream_output(full, reasoning_full, tool_calls, finish_reason, usage);
                    let mut message = content_blocks.message();
                    for part in &mut message.content {
                        if let MessagePart::ToolCall { name, .. } = part {
                            *name = mapped_tool_name(name, map_tool_name);
                        }
                    }
                    if !message.content.is_empty() {
                        output.canonical_message = Some(message);
                    }
                    bind_output_provenance(&mut output, model, provider_id);
                    output.web_search = web_search;
                    return Ok(output);
                }
                Some(AnthropicSseEvent::MessageStopWithReason {
                    reason,
                    usage: next_usage,
                }) => {
                    if let Some(next) = next_usage {
                        merge_stream_usage(&mut usage, next, sink)?;
                    }
                    if reason.is_empty() {
                        continue;
                    }
                    finish_reason = finish_reason_from_anthropic_stop_reason(&reason);
                    sink.emit(StreamPart::Finish {
                        reason: finish_reason.clone(),
                        full: full.clone(),
                    })?;
                    let mut output =
                        stream_output(full, reasoning_full, tool_calls, finish_reason, usage);
                    let mut message = content_blocks.message();
                    for part in &mut message.content {
                        if let MessagePart::ToolCall { name, .. } = part {
                            *name = mapped_tool_name(name, map_tool_name);
                        }
                    }
                    if !message.content.is_empty() {
                        output.canonical_message = Some(message);
                    }
                    bind_output_provenance(&mut output, model, provider_id);
                    output.web_search = web_search;
                    return Ok(output);
                }
                Some(AnthropicSseEvent::Error(error)) => return Err(error),
                None => {}
            }
        }
    }

    let err = ModelError::stream_ended_early(label);
    Err(err)
}

struct AnthropicContinuationSink<'a> {
    sink: &'a mut (dyn StreamSink + Send),
    prefix: &'a str,
}

impl StreamSink for AnthropicContinuationSink<'_> {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        match part {
            StreamPart::Finish { reason, .. } if reason.kind == FinishKind::PauseTurn => Ok(()),
            StreamPart::Finish { reason, full } => self.sink.emit(StreamPart::Finish {
                reason,
                full: format!("{}{full}", self.prefix),
            }),
            other => self.sink.emit(other),
        }
    }
}

fn merge_anthropic_continuation(
    mut previous: GenerateOutput,
    mut next: GenerateOutput,
) -> GenerateOutput {
    previous.text.push_str(&next.text);
    if let Some(reasoning) = next.reasoning.take() {
        previous
            .reasoning
            .get_or_insert_with(String::new)
            .push_str(&reasoning);
    }
    let next_message = next
        .canonical_message
        .take()
        .unwrap_or_else(|| ModelMessage::text(ModelRole::Assistant, &next.text));
    if let Some(message) = &mut previous.canonical_message {
        message.content.extend(next_message.content);
    } else {
        previous.canonical_message = Some(next_message);
    }
    previous.tool_calls.extend(next.tool_calls);
    previous.billing_usage = previous.billing_usage.take().or(previous.usage.take());
    if let Some(next_usage) = next.billing_usage.take().or_else(|| next.usage.clone()) {
        let usage = previous
            .billing_usage
            .get_or_insert_with(ModelUsage::default);
        let add = |target: &mut Option<u64>, value: Option<u64>| {
            if let Some(value) = value {
                *target = Some(target.unwrap_or(0).saturating_add(value));
            }
        };
        add(&mut usage.input_tokens, next_usage.input_tokens);
        add(&mut usage.output_tokens, next_usage.output_tokens);
        add(&mut usage.total_tokens, next_usage.total_tokens);
        add(
            &mut usage.cached_input_tokens,
            next_usage.cached_input_tokens,
        );
        add(
            &mut usage.cache_creation_input_tokens,
            next_usage.cache_creation_input_tokens,
        );
        add(&mut usage.reasoning_tokens, next_usage.reasoning_tokens);
        usage.context_window_tokens = next_usage
            .context_window_tokens
            .or(usage.context_window_tokens);
    }
    previous.usage = next.usage;
    if let Some(search) = next.web_search {
        merge_anthropic_web_search(&mut previous.web_search, search);
    }
    previous.images.extend(next.images);
    previous.finish_reason = next.finish_reason;
    previous.cancelled = next.cancelled;
    previous
}
pub async fn complete_anthropic_turn(
    provider: &(dyn LanguageModelProvider + Sync),
    mut request: GenerateRequest,
    mut sink: Option<&mut (dyn StreamSink + Send)>,
) -> Result<GenerateOutput, ModelError> {
    const MAX_SERVER_TOOL_CONTINUATIONS: usize = 8;
    let mut accumulated: Option<GenerateOutput> = None;
    for continuation in 0..=MAX_SERVER_TOOL_CONTINUATIONS {
        let output = if let Some(sink) = sink.as_mut() {
            let mut continuation_sink = AnthropicContinuationSink {
                sink: &mut **sink,
                prefix: accumulated
                    .as_ref()
                    .map_or("", |output| output.text.as_str()),
            };
            provider
                .stream(request.clone(), &mut continuation_sink)
                .await?
        } else {
            provider.generate(request.clone()).await?
        };
        let paused = output
            .finish_reason
            .as_ref()
            .is_some_and(|reason| reason.kind == FinishKind::PauseTurn);
        if paused {
            if !output.tool_calls.is_empty() {
                return Err(ModelError::new(
                    "Anthropic pause_turn contains unresolved client tool calls",
                ));
            }
            let native = output
                .canonical_message
                .as_ref()
                .filter(|message| !message.content.is_empty())
                .ok_or_else(|| {
                    ModelError::new(
                        "Anthropic pause_turn has no complete native content to continue",
                    )
                })?;
            // Continue the same assistant turn; do not execute server tools locally
            // or manufacture a new user prompt/result for completed server work.
            request.messages.push(native.clone());
        }
        let output = match accumulated.take() {
            Some(previous) => merge_anthropic_continuation(previous, output),
            None => output,
        };
        if !paused {
            return Ok(output);
        }
        accumulated = Some(output);
        if continuation == MAX_SERVER_TOOL_CONTINUATIONS {
            return Err(ModelError::new(
                "Anthropic server tools exceeded the pause_turn continuation limit (8)",
            ));
        }
    }
    unreachable!("bounded continuation loop always returns")
}

/// 在请求体上打 prompt 缓存断点。用文档化的**显式断点**写法（给内容块加 `cache_control`），
/// 而不是顶层 `cache_control`——后者只在官方域名成立，兼容网关的行为没法保证。
///
/// 断点位置按前缀稳定性从前往后排：tools → system → 最后一条消息的最后一个可缓存块。
/// 前两个是每轮不变的长前缀（省钱的大头），第三个把本轮的对话历史也一起缓存进去，
/// 下一轮就能直接命中。Anthropic 上限 4 个断点，这里最多用 3 个。
fn apply_prompt_cache_breakpoints(body: &mut Value, cache_control: &Value) {
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if let Some(last) = tools.last_mut().and_then(Value::as_object_mut) {
            last.insert("cache_control".to_string(), cache_control.clone());
        }
    }

    // system 由 request_body 写成块数组；provider_options 覆盖成裸字符串时这里补成块结构。
    if let Some(system) = body.get("system").and_then(Value::as_str) {
        body["system"] = serde_json::json!([{
            "type": "text",
            "text": system,
            "cache_control": cache_control.clone(),
        }]);
    } else if let Some(blocks) = body.get_mut("system").and_then(Value::as_array_mut) {
        mark_last_cacheable_block(blocks, cache_control);
    }

    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        if let Some(content) = messages
            .last_mut()
            .and_then(|m| m.get_mut("content"))
            .and_then(Value::as_array_mut)
        {
            mark_last_cacheable_block(content, cache_control);
        }
    }
}

/// 给最后一个可缓存的块加断点。`thinking` 块与空 text 块不能带 cache_control。
fn mark_last_cacheable_block(blocks: &mut [Value], cache_control: &Value) {
    for block in blocks.iter_mut().rev() {
        let Some(obj) = block.as_object_mut() else {
            continue;
        };
        match obj.get("type").and_then(Value::as_str) {
            Some("thinking") | Some("redacted_thinking") => continue,
            Some("text")
                if obj
                    .get("text")
                    .and_then(Value::as_str)
                    .is_none_or(|t| t.trim().is_empty()) =>
            {
                continue
            }
            _ => {}
        }
        obj.insert("cache_control".to_string(), cache_control.clone());
        return;
    }
}

/// 官方 budget 起点，不是 Anthropic 定义的 effort→token 表：
/// 最小 1024；文档示例 10000；复杂任务从 16000 起。
/// https://platform.claude.com/docs/en/build-with-claude/extended-thinking#budget-rules-and-tuning
const EXTENDED_THINKING_BUDGET_MIN: u32 = 1024;
const EXTENDED_THINKING_BUDGET_EXAMPLE: u32 = 10_000;
const EXTENDED_THINKING_BUDGET_COMPLEX: u32 = 16_000;

fn extended_thinking_budget_for_level(level: &str) -> u32 {
    match level {
        "low" => EXTENDED_THINKING_BUDGET_MIN,
        "medium" => EXTENDED_THINKING_BUDGET_EXAMPLE,
        _ => EXTENDED_THINKING_BUDGET_COMPLEX,
    }
}

/// `budget_tokens` 必须 ≥1024 且 < `max_tokens`，并给最终回答留空。
fn clamp_extended_thinking_budget(budget: u32, max_tokens: u32) -> Option<u32> {
    if max_tokens <= EXTENDED_THINKING_BUDGET_MIN {
        return None;
    }
    let cap = (max_tokens - 1).min(max_tokens.saturating_sub(EXTENDED_THINKING_BUDGET_MIN));
    Some(budget.min(cap).max(EXTENDED_THINKING_BUDGET_MIN))
}

pub fn anthropic_messages_from_generate_request(request: &GenerateRequest) -> Vec<Value> {
    anthropic_messages_with_unsigned_thinking(request, false, None)
}

fn anthropic_messages_with_unsigned_thinking(
    request: &GenerateRequest,
    allow_unsigned: bool,
    provider_id: Option<&str>,
) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for message in super::types::paired_model_messages(&request.messages).iter() {
        match message.role {
            ModelRole::User | ModelRole::Developer => messages.push(serde_json::json!({
                "role": "user",
                "content": anthropic_content_blocks(message, ModelRole::User, allow_unsigned, &request.model, provider_id),
            })),
            ModelRole::Assistant => messages.push(serde_json::json!({
                "role": "assistant",
                "content": anthropic_content_blocks(message, ModelRole::Assistant, allow_unsigned, &request.model, provider_id),
            })),
            ModelRole::Tool => messages.push(serde_json::json!({
                "role": "user",
                "content": anthropic_content_blocks(message, ModelRole::Tool, allow_unsigned, &request.model, provider_id),
            })),
        }
    }
    messages.retain(|m| {
        m["content"]
            .as_array()
            .is_some_and(|parts| !parts.is_empty())
    });
    normalize_anthropic_tool_ids(&mut messages);
    merge_consecutive_anthropic_roles(&mut messages);
    messages
}

/// Client ids must match ^[a-zA-Z0-9_-]+$. Reserve every valid id before
/// encoding foreign ids so a literal id cannot collide with an encoded one.
fn normalize_anthropic_tool_ids(messages: &mut [Value]) {
    let mut used = std::collections::HashSet::new();
    let valid = |id: &str| {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    let ids: Vec<String> = messages
        .iter()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_use" || b["type"] == "tool_result")
        .filter_map(|b| {
            b.get("id")
                .or_else(|| b.get("tool_use_id"))?
                .as_str()
                .map(str::to_string)
        })
        .collect();
    for id in &ids {
        if valid(id) {
            used.insert(id.clone());
        }
    }
    let mut mapping = std::collections::HashMap::new();
    for id in ids {
        if valid(&id) || mapping.contains_key(&id) {
            continue;
        }
        let encoded = format!(
            "toolu_kivio_{}",
            id.as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let mut candidate = encoded.clone();
        let mut suffix = 0;
        while !used.insert(candidate.clone()) {
            suffix += 1;
            candidate = format!("{encoded}_{suffix}");
        }
        mapping.insert(id, candidate);
    }
    for message in messages {
        if let Some(blocks) = message["content"].as_array_mut() {
            for block in blocks {
                let key = match block["type"].as_str() {
                    Some("tool_use") => "id",
                    Some("tool_result") => "tool_use_id",
                    _ => continue,
                };
                if let Some(mapped) = block[key].as_str().and_then(|id| mapping.get(id)) {
                    block[key] = Value::String(mapped.clone());
                }
            }
        }
    }
}

pub fn anthropic_tools_from_model_tools(tools: &[ModelTool]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tool in tools {
        let name = &tool.name;
        if name.is_empty() || !seen.insert(name) {
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
        let code = provider_error_code(error);
        return Err(ModelError::from_provider_payload(
            code.as_deref(),
            format!("{label}: {msg}"),
        ));
    }

    let parsed = parse_anthropic_response(value);
    let finish_reason = parsed.finish_reason.clone();
    let usage = anthropic_usage(value);
    Ok(GenerateOutput {
        billing_usage: None,
        canonical_message: Some(ModelMessage {
            role: ModelRole::Assistant,
            content: anthropic_model_parts(
                value
                    .get("content")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            ),
        }),
        text: parsed.content,
        reasoning: parsed.reasoning,
        tool_calls: parsed.tool_calls,
        usage,
        finish_reason: Some(finish_reason),
        cancelled: false,
        web_search: parsed.web_search,
        images: Vec::new(),
        reasoning_items: Vec::new(),
    })
}

fn anthropic_model_parts(blocks: &[Value]) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    for block in blocks {
        match block["type"].as_str().unwrap_or_default() {
            "thinking" => {
                if let Some(thinking) = block["thinking"].as_str() {
                    parts.push(MessagePart::AnthropicThinking {
                        thinking: thinking.to_string(),
                        source_model: None,
                        source_provider_id: None,
                        signature: block["signature"]
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                    });
                }
            }
            "redacted_thinking" => {
                if let Some(data) = block["data"].as_str() {
                    parts.push(MessagePart::AnthropicRedactedThinking {
                        data: data.to_string(),
                        source_model: None,
                        source_provider_id: None,
                    });
                }
            }
            "text" if block.get("citations").is_some() => {
                parts.push(MessagePart::AnthropicNativeBlock {
                    block: block.clone(),
                    source_model: None,
                    source_provider_id: None,
                })
            }
            "server_tool_use"
            | "web_search_tool_result"
            | "web_fetch_tool_result"
            | "code_execution_tool_result"
            | "bash_code_execution_tool_result"
            | "text_editor_code_execution_tool_result"
            | "tool_search_tool_result"
            | "container_upload" => parts.push(MessagePart::AnthropicNativeBlock {
                block: block.clone(),
                source_model: None,
                source_provider_id: None,
            }),
            "text" => {
                if let Some(text) = block["text"].as_str() {
                    parts.push(MessagePart::Text {
                        text: text.to_string(),
                    });
                }
            }
            "tool_use" => {
                let input = block
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                parts.push(MessagePart::ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    name: block["name"].as_str().unwrap_or_default().to_string(),
                    arguments_raw: input.to_string(),
                    arguments: input,
                    signature: None,
                });
            }
            _ => {}
        }
    }
    parts
}

struct AnthropicParsedResponse {
    content: String,
    reasoning: Option<String>,
    tool_calls: Vec<PendingToolCall>,
    finish_reason: FinishReason,
    web_search: Option<BuiltinWebSearch>,
}

/// 从一个 `web_search_tool_result` block 的 `content[]` 收集来源到累积值（去重按 url）。
/// 每项形如 `{type:"web_search_result", url, title}`。尽力而为，缺字段即跳过。
fn collect_anthropic_web_search_results(block: &Value, acc: &mut BuiltinWebSearch) {
    let Some(results) = block.get("content").and_then(Value::as_array) else {
        return;
    };
    for item in results {
        let Some(url) = item
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if acc.citations.iter().any(|c| c.url == url) {
            continue;
        }
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(url)
            .to_string();
        acc.citations.push(WebCitation {
            title,
            url: url.to_string(),
            ..Default::default()
        });
    }
}

fn parse_anthropic_response(response: &Value) -> AnthropicParsedResponse {
    let mut content_parts = Vec::new();
    let mut reasoning_parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut web_search = BuiltinWebSearch::default();

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
                // 内置搜索：服务端工具调用（`server_tool_use`）带查询词，结果块
                // （`web_search_tool_result`）带来源。二者都由 Anthropic 服务端产生，
                // 不进 tool_calls（那是客户端函数工具），只收进 web_search 供卡片可视化。
                "server_tool_use" => {
                    if block.get("name").and_then(Value::as_str) == Some("web_search") {
                        if let Some(query) = block
                            .get("input")
                            .and_then(|input| input.get("query"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                        {
                            if !web_search.queries.iter().any(|q| q == query) {
                                web_search.queries.push(query.to_string());
                            }
                        }
                    }
                }
                "web_search_tool_result" => {
                    collect_anthropic_web_search_results(block, &mut web_search);
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
        web_search: if web_search.is_empty() {
            None
        } else {
            Some(web_search)
        },
    }
}

/// Owns wire block assembly. Only stopped blocks can become immutable replay content.
#[derive(Default)]
struct AnthropicContentAccumulator {
    blocks: std::collections::BTreeMap<u64, Value>,
    completed: std::collections::BTreeSet<u64>,
    tool_input: std::collections::BTreeMap<u64, String>,
}

impl AnthropicContentAccumulator {
    fn push(&mut self, data: &str) {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let Some(index) = value["index"].as_u64() else {
            return;
        };
        match value["type"].as_str().unwrap_or_default() {
            "content_block_start" => {
                if let Some(block) = value.get("content_block") {
                    self.blocks.insert(index, block.clone());
                }
            }
            "content_block_delta" => {
                let Some(block) = self.blocks.get_mut(&index) else {
                    return;
                };
                let delta = &value["delta"];
                let key = match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => "text",
                    "thinking_delta" => "thinking",
                    "signature_delta" => "signature",
                    "citations_delta" => {
                        if let Some(citation) = delta.get("citation") {
                            if !block["citations"].is_array() {
                                block["citations"] = serde_json::json!([]);
                            }
                            block["citations"]
                                .as_array_mut()
                                .unwrap()
                                .push(citation.clone());
                        }
                        return;
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta["partial_json"].as_str() {
                            self.tool_input.entry(index).or_default().push_str(partial);
                        }
                        return;
                    }
                    _ => return,
                };
                if let Some(partial) = delta[key].as_str() {
                    let mut text = block[key].as_str().unwrap_or_default().to_string();
                    text.push_str(partial);
                    block[key] = Value::String(text);
                }
            }
            "content_block_stop" => {
                if let Some(raw) = self.tool_input.remove(&index) {
                    if let Some(block) = self.blocks.get_mut(&index) {
                        block["input"] = serde_json::from_str(&raw).unwrap_or(Value::Null);
                    }
                }
                self.completed.insert(index);
            }
            _ => {}
        }
    }

    fn message(&self) -> ModelMessage {
        let blocks: Vec<Value> = self
            .blocks
            .iter()
            .filter(|(index, _)| self.completed.contains(index))
            .map(|(_, block)| block.clone())
            .collect();
        ModelMessage {
            role: ModelRole::Assistant,
            content: anthropic_model_parts(&blocks),
        }
    }
}

enum AnthropicSseEvent {
    Usage(ModelUsage),
    TextDelta(String),
    ThinkingDelta(String),
    ToolUseStart {
        id: String,
        name: String,
        input: Value,
    },
    ToolInputDelta(String),
    ContentBlockStop,
    /// 内置搜索的服务端块（`server_tool_use` 查询 / `web_search_tool_result` 结果），
    /// 完整随 `content_block_start` 到达，尽力收集来源；解析不到即无此事件。
    WebSearch(BuiltinWebSearch),
    MessageStop,
    MessageStopWithReason {
        reason: String,
        usage: Option<ModelUsage>,
    },
    Error(ModelError),
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
        "message_start" => {
            model_usage_from_anthropic_value(value.get("message")?).map(AnthropicSseEvent::Usage)
        }
        "content_block_start" => {
            let block = value.get("content_block")?;
            match block.get("type").and_then(|value| value.as_str()) {
                Some("tool_use") => Some(AnthropicSseEvent::ToolUseStart {
                    input: block
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
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
                }),
                // 内置搜索服务端块：查询词（server_tool_use.input.query）或结果（web_search_tool_result）。
                Some("server_tool_use") => {
                    if block.get("name").and_then(Value::as_str) != Some("web_search") {
                        return None;
                    }
                    let query = block
                        .get("input")
                        .and_then(|input| input.get("query"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    Some(AnthropicSseEvent::WebSearch(BuiltinWebSearch {
                        queries: vec![query.to_string()],
                        citations: Vec::new(),
                    }))
                }
                Some("web_search_tool_result") => {
                    let mut web_search = BuiltinWebSearch::default();
                    collect_anthropic_web_search_results(block, &mut web_search);
                    if web_search.is_empty() {
                        None
                    } else {
                        Some(AnthropicSseEvent::WebSearch(web_search))
                    }
                }
                _ => None,
            }
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
                .unwrap_or("")
                .to_string();
            Some(AnthropicSseEvent::MessageStopWithReason {
                reason,
                usage: model_usage_from_anthropic_value(&value),
            })
        }
        "message_stop" => Some(AnthropicSseEvent::MessageStop),
        "error" => Some(AnthropicSseEvent::Error({
            let error = &value["error"];
            let message = error["message"]
                .as_str()
                .unwrap_or("Unknown Anthropic error");
            let diagnostic = match error["type"].as_str() {
                Some(kind) => format!("{message} ({kind})"),
                None => message.to_string(),
            };
            let code = provider_error_code(error);
            ModelError::from_provider_stream(code.as_deref(), diagnostic)
        })),
        _ => None,
    }
}

/// Delta fields are cumulative snapshots, not increments. Omitted fields retain
/// the input/cache counts received at message_start.
fn merge_stream_usage(
    usage: &mut Option<ModelUsage>,
    next: ModelUsage,
    sink: &mut dyn StreamSink,
) -> Result<(), ModelError> {
    let current = usage.get_or_insert_with(ModelUsage::default);
    current.input_tokens = next.input_tokens.or(current.input_tokens);
    current.output_tokens = next.output_tokens.or(current.output_tokens);
    current.cached_input_tokens = next.cached_input_tokens.or(current.cached_input_tokens);
    current.cache_creation_input_tokens = next
        .cache_creation_input_tokens
        .or(current.cache_creation_input_tokens);
    current.total_tokens = current
        .input_tokens
        .zip(current.output_tokens)
        .map(|(input, output)| input.saturating_add(output));
    if let Some(input_tokens) = current.input_tokens {
        let input_tokens = input_tokens
            .saturating_add(current.cached_input_tokens.unwrap_or(0))
            .saturating_add(current.cache_creation_input_tokens.unwrap_or(0));
        sink.emit(StreamPart::ContextUsage {
            input_tokens,
            output_tokens: current.output_tokens.unwrap_or(0),
        })?;
    }
    Ok(())
}

fn assemble_tool_call_from_stream(
    id: &str,
    name: &str,
    input_json_parts: &[String],
    initial_input: &Value,
) -> PendingToolCall {
    let raw = input_json_parts.join("");
    let arguments_raw = if input_json_parts.is_empty() {
        initial_input.to_string()
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

fn anthropic_content_blocks(
    message: &ModelMessage,
    role: ModelRole,
    allow_unsigned: bool,
    model: &str,
    provider_id: Option<&str>,
) -> Vec<Value> {
    let mut blocks = Vec::new();
    for part in &message.content {
        let origin = match part {
            MessagePart::AnthropicThinking {
                source_model,
                source_provider_id,
                ..
            }
            | MessagePart::AnthropicNativeBlock {
                source_model,
                source_provider_id,
                ..
            }
            | MessagePart::AnthropicRedactedThinking {
                source_model,
                source_provider_id,
                ..
            } => {
                source_model
                    .as_deref()
                    .map_or(true, |source| source == model)
                    && provider_id.map_or(true, |provider| {
                        source_provider_id
                            .as_deref()
                            .map_or(true, |source| source == provider)
                    })
            }
            _ => true,
        };
        if !origin {
            if let MessagePart::AnthropicNativeBlock { block, .. } = part {
                if block["type"] == "text" {
                    if let Some(text) = block["text"]
                        .as_str()
                        .filter(|text| !text.trim().is_empty())
                    {
                        blocks.push(serde_json::json!({"type":"text", "text":text}));
                    }
                }
            }
            continue;
        }
        match part {
            MessagePart::Video { .. } => {
                blocks.push(serde_json::json!({"type":"text", "text":"[视频输入不受此协议支持]"}))
            }
            MessagePart::Text { text } => {
                if !text.trim().is_empty() {
                    blocks.push(serde_json::json!({"type":"text", "text":text}));
                }
            }
            MessagePart::Image {
                mime_type, data, ..
            } => {
                if matches!(role, ModelRole::User) {
                    if data.is_empty() {
                        blocks.push(serde_json::json!({
                            "type": "text",
                            "text": super::MISSING_IMAGE_PLACEHOLDER,
                        }));
                    } else {
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
            }
            MessagePart::ImageUrl { url, .. } => {
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
            MessagePart::AnthropicThinking {
                thinking,
                signature,
                ..
            } => {
                if role == ModelRole::Assistant && (signature.is_some() || allow_unsigned) {
                    let mut block = serde_json::json!({"type":"thinking", "thinking":thinking});
                    if let Some(signature) = signature {
                        block["signature"] = Value::String(signature.clone());
                    }
                    blocks.push(block);
                }
            }
            MessagePart::AnthropicNativeBlock { block, .. } => {
                if block["type"] != "text"
                    || block["text"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty())
                {
                    blocks.push(block.clone());
                }
            }
            MessagePart::AnthropicRedactedThinking { data, .. } => {
                if role == ModelRole::Assistant {
                    blocks.push(serde_json::json!({"type":"redacted_thinking", "data":data}));
                }
            }
            // Display reasoning from old history/other protocols cannot become a signed block.
            MessagePart::Reasoning { text } => {
                if role == ModelRole::Assistant && allow_unsigned {
                    blocks.push(serde_json::json!({"type":"thinking", "thinking":text}));
                }
            }
            MessagePart::ReasoningItem { .. } => {}
        }
    }
    if allow_unsigned
        && message
            .content
            .iter()
            .any(|part| matches!(part, MessagePart::Reasoning { .. }))
    {
        blocks.sort_by_key(|block| usize::from(block["type"] != "thinking"));
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
            obj.entry("properties")
                .or_insert_with(|| serde_json::json!({}));
        }
        return result;
    }

    schema
}

fn anthropic_usage(value: &Value) -> Option<ModelUsage> {
    model_usage_from_anthropic_value(value)
}

fn stream_output(
    text: String,
    reasoning: String,
    tool_calls: Vec<PendingToolCall>,
    finish_reason: FinishReason,
    usage: Option<ModelUsage>,
) -> GenerateOutput {
    let reasoning = non_empty(reasoning);
    GenerateOutput {
        canonical_message: None,
        billing_usage: None,
        text,
        reasoning,
        tool_calls,
        usage,
        finish_reason: Some(finish_reason),
        cancelled: false,
        web_search: None,
        images: Vec::new(),
        reasoning_items: Vec::new(),
    }
}

/// 把流式逐块解析出的内置搜索片段合并进累积值（去重按 url / query 文本）。
fn merge_anthropic_web_search(acc: &mut Option<BuiltinWebSearch>, next: BuiltinWebSearch) {
    let target = acc.get_or_insert_with(BuiltinWebSearch::default);
    for query in next.queries {
        if !target.queries.iter().any(|existing| *existing == query) {
            target.queries.push(query);
        }
    }
    for citation in next.citations {
        if !target.citations.iter().any(|c| c.url == citation.url) {
            target.citations.push(citation);
        }
    }
}

fn finish_reason_from_anthropic_stop_reason(reason: &str) -> FinishReason {
    let kind = match reason {
        "tool_use" => FinishKind::ToolCalls,
        "max_tokens" => FinishKind::Length,
        "pause_turn" => FinishKind::PauseTurn,
        _ => FinishKind::Stop,
    };
    FinishReason::new(kind, reason)
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
    use crate::model::GenerateOptions;

    async fn streamed(
        events: Vec<Value>,
        provider_id: Option<&str>,
        map_tool_name: Option<&(dyn Fn(&str) -> String + Sync)>,
    ) -> (Vec<StreamPart>, Result<GenerateOutput, ModelError>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let body = events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len(),
            ).as_bytes()).await.unwrap();
        });
        let response = reqwest::Client::new().get(endpoint).send().await.unwrap();
        let mut parts = Vec::new();
        let output = decode_anthropic_stream(
            response,
            &mut |part| {
                parts.push(part);
                Ok(())
            },
            "fixture",
            "test-model",
            provider_id,
            map_tool_name,
        )
        .await;
        server.await.unwrap();
        (parts, output)
    }

    #[tokio::test]
    async fn json_and_stream_preserve_native_blocks_provenance_and_mapped_tool_names() {
        let blocks = serde_json::json!([
            {"type":"thinking", "thinking":"plan", "signature":"sig"},
            {"type":"redacted_thinking", "data":"secret"},
            {"type":"server_tool_use", "id":"server_1", "name":"web_search", "input":{"query":"fixture"}},
            {"type":"text", "text":"Answer", "citations":[{"type":"web_search_result_location", "url":"https://example.org"}]},
            {"type":"tool_use", "id":"tool_1", "name":"Read", "input":{"path":"a"}}
        ]);
        let response = serde_json::json!({
            "content":blocks, "stop_reason":"tool_use", "usage":{"input_tokens":3,"output_tokens":5}
        });
        let map_tool_name = |name: &str| format!("tool/{name}");
        let json = decode_anthropic_body(
            &response.to_string(),
            "fixture",
            "test-model",
            Some("provider-a"),
            Some(&map_tool_name),
        )
        .unwrap();
        let mut events = vec![
            serde_json::json!({"type":"message_start", "message":{"usage":{"input_tokens":3,"output_tokens":0}}}),
        ];
        for (index, block) in blocks.as_array().unwrap().iter().enumerate() {
            let mut initial = block.clone();
            let delta = match index {
                0 => {
                    initial["thinking"] = serde_json::json!("");
                    initial["signature"] = serde_json::json!("");
                    Some(serde_json::json!({"type":"thinking_delta", "thinking":"plan"}))
                }
                3 => {
                    initial["text"] = serde_json::json!("");
                    Some(serde_json::json!({"type":"text_delta", "text":"Answer"}))
                }
                4 => {
                    initial["input"] = serde_json::json!({});
                    Some(
                        serde_json::json!({"type":"input_json_delta", "partial_json":"{\"path\":\"a\"}"}),
                    )
                }
                _ => None,
            };
            events.push(serde_json::json!({"type":"content_block_start", "index":index, "content_block":initial}));
            if let Some(delta) = delta {
                events.push(
                    serde_json::json!({"type":"content_block_delta", "index":index, "delta":delta}),
                );
            }
            if index == 0 {
                events.push(serde_json::json!({"type":"content_block_delta", "index":index, "delta":{"type":"signature_delta", "signature":"sig"}}));
            }
            events.push(serde_json::json!({"type":"content_block_stop", "index":index}));
        }
        events.push(serde_json::json!({"type":"message_delta", "delta":{"stop_reason":"tool_use"}, "usage":{"output_tokens":5}}));
        let (parts, streamed) = streamed(events, Some("provider-a"), Some(&map_tool_name)).await;
        let streamed = streamed.unwrap();
        assert_eq!(
            serde_json::to_value(streamed.replay_message()).unwrap(),
            serde_json::to_value(json.replay_message()).unwrap()
        );
        assert_eq!(streamed.text, "Answer");
        assert_eq!(streamed.reasoning.as_deref(), Some("plan"));
        assert_eq!(streamed.tool_calls[0].function_name, "tool/Read");
        assert!(parts.iter().any(
            |part| matches!(part, StreamPart::ToolCallStart { name, .. } if name == "tool/Read")
        ));
        assert!(parts.iter().any(|part| matches!(part, StreamPart::ToolCallDone { call } if call.function_name == "tool/Read")));
        for part in &streamed.canonical_message.as_ref().unwrap().content {
            match part {
                MessagePart::AnthropicThinking {
                    source_model,
                    source_provider_id,
                    ..
                }
                | MessagePart::AnthropicRedactedThinking {
                    source_model,
                    source_provider_id,
                    ..
                }
                | MessagePart::AnthropicNativeBlock {
                    source_model,
                    source_provider_id,
                    ..
                } => {
                    assert_eq!(source_model.as_deref(), Some("test-model"));
                    assert_eq!(source_provider_id.as_deref(), Some("provider-a"));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn replay_filters_known_origins_and_preserves_legacy_unscoped_blocks() {
        let response = serde_json::json!({"content":[
            {"type":"thinking", "thinking":"plan", "signature":"sig"},
            {"type":"redacted_thinking", "data":"secret"},
            {"type":"text", "text":"Answer", "citations":[{"url":"https://example.org"}]}
        ]});
        let output = decode_anthropic_body(
            &response.to_string(),
            "fixture",
            "test-model",
            Some("provider-a"),
            None,
        )
        .unwrap();
        let mut request = cache_test_request();
        request.messages = vec![output.replay_message()];
        let mut options = AnthropicMessagesOptions::default();
        assert!(request_body(&request, false, &options)
            .to_string()
            .contains("secret"));
        options.provider_id = Some("provider-a".into());
        assert!(request_body(&request, false, &options)
            .to_string()
            .contains("secret"));
        options.provider_id = Some("provider-b".into());
        let foreign = request_body(&request, false, &options);
        assert!(!foreign.to_string().contains("secret"));
        assert_eq!(
            foreign["messages"][0]["content"],
            serde_json::json!([{"type":"text", "text":"Answer"}])
        );

        options.provider_id = Some("provider-a".into());
        request.model = "another-model".into();
        assert!(!request_body(&request, false, &options)
            .to_string()
            .contains("secret"));
        request.messages = vec![output_from_anthropic_message(&response, "legacy")
            .unwrap()
            .replay_message()];
        assert!(request_body(&request, false, &options)
            .to_string()
            .contains("secret"));
    }

    #[tokio::test]
    async fn incomplete_blocks_do_not_enter_replay_and_missing_terminal_is_retryable() {
        let start = serde_json::json!({"type":"content_block_start", "index":0, "content_block":{"type":"thinking", "thinking":"partial", "signature":"partial-signature"}});
        let (_, output) = streamed(
            vec![
                start.clone(),
                serde_json::json!({"type":"message_delta", "delta":{"stop_reason":"end_turn"}}),
            ],
            None,
            None,
        )
        .await;
        assert!(output.unwrap().canonical_message.is_none());
        let (_, output) = streamed(vec![start], None, None).await;
        assert!(output.unwrap_err().is_retryable_stream_error());
    }

    #[tokio::test]
    async fn standalone_provider_rejects_video_before_http() {
        let provider = AnthropicMessagesProvider::new(
            reqwest::Client::new(),
            AnthropicMessagesConfig::new("http://127.0.0.1:1/messages".parse().unwrap()),
        );
        let mut request = cache_test_request();
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

    #[test]
    fn cache_marks_final_override_blocks_and_tool_prohibition_wins() {
        let mut request = cache_test_request();
        request.options.max_tokens = 0;
        request.options.provider_options = serde_json::json!({
            "system":"replaced", "tools":[{"name":"override"}],
            "messages":[{"role":"user", "content":[{"type":"text", "text":"replaced"}]}],
        });
        let options = AnthropicMessagesOptions {
            default_max_tokens: 4096,
            cache: Some(AnthropicCacheTtl::OneHour),
            ..Default::default()
        };
        let body = request_body(&request, false, &options);
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["system"][0]["text"], "replaced");
        assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
        assert_eq!(body["tools"][0]["cache_control"]["ttl"], "1h");
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["ttl"],
            "1h"
        );
        request.options.tool_choice = super::super::ToolChoice::None;
        let body = request_body(&request, false, &options);
        assert!(body.get("tools").is_none());
        assert_eq!(body["tool_choice"], serde_json::json!({"type":"none"}));
    }
    fn cache_test_request() -> GenerateRequest {
        GenerateRequest {
            model: "test-model".into(),
            system: "system".into(),
            messages: vec![ModelMessage::text(ModelRole::User, "hi")],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        }
    }

    #[test]
    fn signed_thinking_survives_tool_round_and_storage_reload() {
        let blocks = serde_json::json!([
            {"type":"thinking", "thinking":" plan ", "signature":"sig-one"},
            {"type":"redacted_thinking", "data":"opaque-redacted"},
            {"type":"thinking", "thinking":"", "signature":"sig-empty"},
            {"type":"server_tool_use", "id":"srvtoolu_search", "name":"web_search", "input":{"query":"fixture"}},
            {"type":"web_search_tool_result", "tool_use_id":"srvtoolu_search", "content":[{"type":"web_search_result", "url":"https://example.org", "title":"Evidence", "encrypted_content":"opaque"}]},
            {"type":"text", "text":"Checking", "citations":[{"type":"web_search_result_location", "url":"https://example.org", "title":"Evidence", "encrypted_index":"opaque-index", "cited_text":"Evidence"}]},
            {"type":"tool_use", "id":"toolu_1", "name":"read", "input":{"path":"a"}}
        ]);
        let output = output_from_anthropic_message(
            &serde_json::json!({
                "content": blocks, "stop_reason":"tool_use"
            }),
            "fixture",
        )
        .unwrap();
        assert_eq!(
            output.finish_reason.as_ref().unwrap().kind,
            FinishKind::ToolCalls
        );
        let mut history = vec![
            output.replay_message(),
            ModelMessage {
                role: ModelRole::Tool,
                content: vec![MessagePart::ToolResult {
                    tool_call_id: "toolu_1".into(),
                    content: "found".into(),
                    is_error: false,
                    artifacts: Vec::new(),
                }],
            },
        ];
        // Real storage format is serde canonical messages; reload must preserve opaque bytes.
        history = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        let mut request = cache_test_request();
        request.messages = history;
        let replay = anthropic_messages_from_generate_request(&request);
        assert_eq!(replay[0]["content"], blocks);
        assert_eq!(replay[1]["content"][0]["tool_use_id"], "toolu_1");
        let truncated = output_from_anthropic_message(
            &serde_json::json!({
                "content":blocks, "stop_reason":"max_tokens"
            }),
            "fixture",
        )
        .unwrap();
        assert_eq!(
            truncated.finish_reason.as_ref().unwrap().kind,
            FinishKind::Length
        );
    }

    #[test]
    fn unsigned_legacy_reasoning_is_display_only_and_native_blocks_stay_provider_local() {
        let legacy = crate::model::model_messages_from_openai_messages(vec![serde_json::json!({
            "role":"assistant", "content":"answer", "reasoning_content":"unsigned legacy plan"
        })]);
        let mut request = cache_test_request();
        request.messages = legacy;
        assert_eq!(
            anthropic_messages_from_generate_request(&request)[0]["content"],
            serde_json::json!([{"type":"text", "text":"answer"}])
        );
        let compatible = anthropic_messages_with_unsigned_thinking(&request, true, None);
        assert_eq!(
            compatible[0]["content"][0],
            serde_json::json!({"type":"thinking", "thinking":"unsigned legacy plan"})
        );
        let unsigned_native = output_from_anthropic_message(
            &serde_json::json!({"content":[
                {"type":"thinking", "thinking":"gateway plan"}, {"type":"text", "text":"answer"}
            ]}),
            "fixture",
        )
        .unwrap();
        let mut unsigned_request = cache_test_request();
        unsigned_request.messages = vec![unsigned_native.canonical_message.unwrap()];
        assert_eq!(
            anthropic_messages_with_unsigned_thinking(&unsigned_request, true, None)[0]["content"]
                [0],
            serde_json::json!({"type":"thinking", "thinking":"gateway plan"})
        );
        assert_eq!(
            anthropic_messages_from_generate_request(&unsigned_request)[0]["content"],
            serde_json::json!([{"type":"text", "text":"answer"}])
        );
        let output = output_from_anthropic_message(
            &serde_json::json!({"content":[
                {"type":"thinking", "thinking":"native", "signature":"opaque"},
                {"type":"redacted_thinking", "data":"redacted"},
                {"type":"text", "text":"answer"}
            ]}),
            "fixture",
        )
        .unwrap();
        request.messages = vec![output.canonical_message.unwrap()];
        let openai = crate::model::openai_messages_from_generate_request(&request);
        assert!(openai
            .iter()
            .all(|message| message.get("anthropic_content").is_none()));
        let responses = crate::model::responses_input_from_model_messages(&request.messages, None);
        assert!(!serde_json::to_string(&responses)
            .unwrap()
            .contains("opaque"));
        assert!(!serde_json::to_string(&responses)
            .unwrap()
            .contains("redacted"));
    }

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
        assert!(
            out.get("anyOf").is_none(),
            "top-level anyOf must be dropped"
        );
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

    #[test]
    fn cache_breakpoint_skips_thinking_and_empty_text_blocks() {
        let mut blocks = vec![
            serde_json::json!({ "type": "text", "text": "real content" }),
            serde_json::json!({ "type": "text", "text": "   " }),
            serde_json::json!({ "type": "thinking", "thinking": "hmm" }),
        ];
        mark_last_cacheable_block(&mut blocks, &serde_json::json!({ "type": "ephemeral" }));
        // thinking 块和空 text 块不能带 cache_control，断点要往前退到第一块。
        assert!(blocks[0].get("cache_control").is_some());
        assert!(blocks[1].get("cache_control").is_none());
        assert!(blocks[2].get("cache_control").is_none());
    }

    #[test]
    fn web_search_parsed_from_content_blocks() {
        // server_tool_use(web_search) → 查询；web_search_tool_result.content[] → 来源（去重）。
        let response = serde_json::json!({
            "content": [
                { "type": "server_tool_use", "name": "web_search", "input": { "query": "kivio" } },
                {
                    "type": "web_search_tool_result",
                    "content": [
                        { "type": "web_search_result", "url": "https://a.com", "title": "A 站" },
                        { "type": "web_search_result", "url": "https://a.com", "title": "dup" },
                        { "type": "web_search_result", "url": "https://b.com" }
                    ]
                },
                { "type": "text", "text": "见来源。" }
            ],
            "stop_reason": "end_turn"
        });
        let parsed = parse_anthropic_response(&response);
        let web_search = parsed.web_search.expect("web_search present");
        assert_eq!(web_search.queries, vec!["kivio".to_string()]);
        assert_eq!(web_search.citations.len(), 2);
        assert_eq!(web_search.citations[0].title, "A 站");
        assert_eq!(web_search.citations[1].title, "https://b.com");
        assert_eq!(parsed.content, "见来源。");
    }

    #[test]
    fn web_search_none_without_server_blocks() {
        let response = serde_json::json!({
            "content": [{ "type": "text", "text": "无搜索" }],
            "stop_reason": "end_turn"
        });
        assert!(parse_anthropic_response(&response).web_search.is_none());
    }

    #[test]
    fn stream_web_search_events_parse_query_then_results_increments() {
        let query_line = "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"server_tool_use\",\"name\":\"web_search\",\"input\":{\"query\":\"kivio\"}}}";
        match parse_anthropic_sse_event(query_line) {
            Some(AnthropicSseEvent::WebSearch(ws)) => {
                assert_eq!(ws.queries, vec!["kivio".to_string()]);
                assert!(ws.citations.is_empty());
            }
            _ => panic!("expected WebSearch query increment"),
        }
        let result_line = "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"web_search_tool_result\",\"content\":[{\"type\":\"web_search_result\",\"url\":\"https://a.com\",\"title\":\"A 站\"}]}}";
        match parse_anthropic_sse_event(result_line) {
            Some(AnthropicSseEvent::WebSearch(ws)) => {
                assert!(ws.queries.is_empty());
                assert_eq!(ws.citations.len(), 1);
                assert_eq!(ws.citations[0].url, "https://a.com");
            }
            _ => panic!("expected WebSearch result increment"),
        }
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
                            path: None,
                            detail: None,
                        },
                    ],
                },
                ModelMessage {
                    role: ModelRole::Assistant,
                    content: vec![MessagePart::ToolCall {
                        id: "toolu_1".to_string(),
                        name: "read".to_string(),
                        arguments: serde_json::json!({}),
                        arguments_raw: "{}".to_string(),
                        signature: None,
                    }],
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

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        let content = messages[0]["content"].as_array().expect("content");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(messages[2]["content"][0]["content"], "done");
        assert_eq!(messages[2]["content"][0]["is_error"], false);
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
        assert_eq!(
            output.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("tool_calls")
        );
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
            Some(AnthropicSseEvent::ToolUseStart { id, name, .. })
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
        let call = assemble_tool_call_from_stream(
            "toolu_123",
            "web_search",
            &input_parts,
            &serde_json::json!({}),
        );

        assert_eq!(call.function_name, "web_search");
        assert_eq!(call.arguments["query"], "kivio");
        assert!(call.arguments_parse_error.is_none());
    }
}
