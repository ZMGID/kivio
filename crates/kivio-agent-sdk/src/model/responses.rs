//! Shared Responses wire protocol and host-configured HTTP provider.
use serde_json::Value;

use super::error::provider_error_code;
use super::usage::model_usage_from_openai_value;
use super::{
    parse_tool_arguments, responses_input_from_model_messages, stream_read_error, BuiltinWebSearch,
    FinishKind, FinishReason, GenerateOutput, GenerateRequest, GeneratedImageData,
    LanguageModelProvider, MessagePart, ModelError, ModelErrorKind, ModelFuture, ModelUsage,
    PendingToolCall, ProviderReasoningItem, StreamPart, StreamSink, WebCitation,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResponsesFormat {
    #[default]
    OpenAi,
    Xai,
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
}

impl OpenAiResponsesProvider {
    pub fn new(client: reqwest::Client, config: OpenAiResponsesConfig) -> Self {
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
                "Responses does not support video input",
                ModelErrorKind::InvalidRequest,
            ));
        }
        let options = OpenAiResponsesOptions {
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
        super::transport::send(http, &request_label(request, "Responses API")).await
    }
}

impl LanguageModelProvider for OpenAiResponsesProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            let label = request_label(&request, "Responses API");
            let raw = self
                .post(&request, false)
                .await?
                .text()
                .await
                .map_err(|error| stream_read_error(&label, &error))?;
            decode_responses_body(&raw, &label, &request.model)
        })
    }

    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            let label = request_label(&request, "Responses stream");
            decode_responses_stream(
                self.post(&request, true).await?,
                sink,
                &label,
                &request.model,
            )
            .await
        })
    }
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

/// Decode a successful JSON or SSE-only response and bind replay data to its model.
pub fn decode_responses_body(
    raw: &str,
    label: &str,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    let mut output = match serde_json::from_str::<Value>(raw) {
        Ok(value) => output_from_responses(&value, raw, label)?,
        Err(error) if !looks_like_sse(raw) => {
            return Err(ModelError::new(format!(
                "{label} parse JSON: {error} (body: {})",
                raw.chars().take(500).collect::<String>()
            )));
        }
        Err(_) => output_from_sse_body(raw)?,
    };
    stamp_reasoning_item_model(&mut output, model);
    Ok(output)
}

/// Completion commits the response even when a proxy keeps the HTTP body open.
pub async fn decode_responses_stream(
    mut response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
    model: &str,
) -> Result<GenerateOutput, ModelError> {
    let mut sse = super::sse::SseDecoder::default();
    let mut state = ResponsesStreamState {
        model: model.to_string(),
        ..Default::default()
    };
    'stream: loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| stream_read_error(label, &error))?;
        let Some(chunk) = chunk else {
            break;
        };
        for data in sse.push(&chunk) {
            if let Some(error) = process_sse_data(&data, &mut state, sink)? {
                return Err(error);
            }
            if state.finish_reason.is_some() {
                break 'stream;
            }
        }
    }
    if state.finish_reason.is_none() {
        return Err(ModelError::stream_ended_early(label));
    }
    state.finish(sink)
}

fn xai_reasoning_effort(effort: &str) -> Option<&'static str> {
    match effort.trim().to_ascii_lowercase().as_str() {
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

pub fn request_body(
    request: &GenerateRequest,
    stream: bool,
    options: &OpenAiResponsesOptions,
) -> Value {
    let is_xai = options.format == ResponsesFormat::Xai;
    // Encrypted reasoning is model-bound; xAI does not support its replay.
    let reasoning_replay = (!is_xai && options.replay_reasoning).then_some(request.model.as_str());
    let mut body = serde_json::json!({
        "model": request.model,
        "input": responses_input_from_model_messages(&request.messages, reasoning_replay),
        "store": false,
    });
    if let Some(extra) = options.extra_body.as_object() {
        for (key, value) in extra {
            body[key] = value.clone();
        }
    }
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
        .map(|tool| tool.to_openai_responses_tool())
        .collect();
    if request.options.builtin_web_search {
        tools.push(serde_json::json!({ "type": "web_search" }));
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = Value::String("auto".to_string());
    }
    if !request.options.thinking_enabled {
        // Omitting effort leaves thinking enabled on some compatible endpoints.
        body["reasoning"] = serde_json::json!({ "effort": "none" });
    } else if let Some(effort) = request.options.thinking_level.as_deref() {
        if is_xai {
            if let Some(mapped) = xai_reasoning_effort(effort) {
                body["reasoning"] = serde_json::json!({ "effort": mapped });
            }
        } else {
            body["reasoning"] = serde_json::json!({ "effort": effort, "summary": "auto" });
        }
    } else if !is_xai {
        body["reasoning"] = serde_json::json!({ "summary": "auto" });
    }
    if !is_xai && request.options.thinking_enabled {
        body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
    }
    if is_xai {
        let include = xai_include_values(body.get("tools"));
        if !include.is_empty() {
            body["include"] = Value::Array(include);
        }
    } else if let Some(key) = options
        .prompt_cache_key
        .as_deref()
        .filter(|key| !key.is_empty())
    {
        body["prompt_cache_key"] = Value::String(key.to_string());
        if let Some(retention) = &options.prompt_cache_retention {
            body["prompt_cache_retention"] = Value::String(retention.clone());
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
    text: String,
    reasoning: String,
    /// 本次请求的模型名，随 reasoning item 落进历史（回放前校验同模型，密文模型键控）。
    model: String,
    /// 原生 reasoning items（带 encrypted_content 才收）。回放它们是 gpt-5 系在
    /// 工具循环里保持思维链连续的关键（Codex 官方客户端同款做法）。
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

    fn finalize_tool_call(
        &mut self,
        item_id: &str,
        sink: &mut (dyn StreamSink + Send),
    ) -> Result<(), ModelError> {
        if let Some(partial) = self.tool_calls.iter_mut().find(|p| p.item_id == item_id) {
            if partial.done {
                return Ok(());
            }
            partial.done = true;
            let call = pending_tool_call_from_partial(partial);
            sink.emit(StreamPart::ToolCallDone { call })?;
        }
        Ok(())
    }

    fn finish(mut self, sink: &mut (dyn StreamSink + Send)) -> Result<GenerateOutput, ModelError> {
        // Flush any function calls that never received an explicit output_item.done.
        let pending_ids: Vec<String> = self
            .tool_calls
            .iter()
            .filter(|partial| !partial.done)
            .map(|partial| partial.item_id.clone())
            .collect();
        for item_id in pending_ids {
            self.finalize_tool_call(&item_id, sink)?;
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
        Ok(GenerateOutput {
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
                })
                .collect(),
        })
    }
}

/// 一只 reasoning item 是否值得存下来回放：必须带非空 `encrypted_content`。
fn reasoning_item_replayable(item: &Value) -> bool {
    item.get("encrypted_content")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
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
                    Some("function_call") => {
                        let item_id = item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or(&item_id)
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        sink.emit(StreamPart::ToolCallStart {
                            id: call_id.clone(),
                            name: name.clone(),
                        })?;
                        state.tool_calls.push(ResponsesToolPartial {
                            item_id,
                            call_id,
                            name,
                            arguments: String::new(),
                            done: false,
                        });
                    }
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
                if let Some(partial) = state.partial_mut(item_id) {
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
                if let Some(partial) = state.partial_mut(item_id) {
                    // The `done` event carries the full argument string — prefer it over
                    // the accumulated deltas to avoid any drift.
                    partial.arguments = arguments.to_string();
                }
            }
        }
        "response.output_item.done" => {
            if let Some(item) = value.get("item") {
                match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => {
                        let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
                        if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                            if let Some(partial) = state.partial_mut(item_id) {
                                if !arguments.is_empty() {
                                    partial.arguments = arguments.to_string();
                                }
                            }
                        }
                        state.finalize_tool_call(item_id, sink)?;
                    }
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
                    // 原生 reasoning item：带 encrypted_content 才值得收（无密文的 item
                    // 在 store:false 下回放会被当服务端引用查找而 404）。整只原样保存，
                    // 回放时随 input 送回 —— 思维链跨工具轮延续的唯一载体。
                    Some("reasoning") => {
                        if reasoning_item_replayable(item) {
                            state.reasoning_items.push(item.clone());
                        }
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
        "response.completed" | "response.incomplete" => {
            if let Some(response) = value.get("response") {
                if let Some(usage) = model_usage_from_openai_value(response) {
                    if let Some(input_tokens) = usage.input_tokens {
                        sink.emit(StreamPart::ContextUsage {
                            input_tokens,
                            output_tokens: usage.output_tokens.unwrap_or(0),
                        })?;
                    }
                    state.usage = Some(usage);
                }
                if let Some(status) = response.get("status").and_then(Value::as_str) {
                    state.finish_reason = Some(responses_finish_reason(
                        status,
                        response,
                        !state.tool_calls.is_empty(),
                    ));
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
                    // reasoning items 兜底：有些网关只在 completed 的完整 output 里带
                    // encrypted_content（output_item.done 里缺）。按 id 去重合并。
                    for item in output {
                        if item.get("type").and_then(Value::as_str) == Some("reasoning")
                            && reasoning_item_replayable(item)
                        {
                            let id = item.get("id").and_then(Value::as_str);
                            let seen = id.is_some_and(|id| {
                                state.reasoning_items.iter().any(|existing| {
                                    existing.get("id").and_then(Value::as_str) == Some(id)
                                })
                            });
                            if !seen {
                                state.reasoning_items.push(item.clone());
                            }
                        }
                    }
                }
            }
        }
        "response.failed" | "error" => {
            let code = responses_error_payload(value)
                .and_then(provider_error_code)
                .or_else(|| value["code"].as_str().map(str::to_string));
            let message = responses_stream_error_text(value);
            return Ok(Some(ModelError::from_provider_stream(
                code.as_deref(),
                message,
            )));
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
fn output_from_sse_body(body: &str) -> Result<GenerateOutput, ModelError> {
    let mut state = ResponsesStreamState::default();
    let mut sink = DiscardSink;
    for data in super::sse::SseDecoder::default().push(body.as_bytes()) {
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

fn pending_tool_call_from_partial(partial: &ResponsesToolPartial) -> PendingToolCall {
    let raw = if partial.arguments.trim().is_empty() {
        "{}".to_string()
    } else {
        partial.arguments.clone()
    };
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
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        if part.get("type").and_then(Value::as_str) == Some("output_text") {
                            if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                                text.push_str(part_text);
                            }
                        }
                    }
                }
            }
            Some("function_call") => {
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
                let raw_args = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("{}")
                    .to_string();
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
                if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                    for text in summary
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .filter(|text| !text.is_empty())
                    {
                        reasoning.push_str(summary_separator(&reasoning));
                        reasoning.push_str(text);
                    }
                }
                if reasoning_item_replayable(item) {
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
    fn xai_includes_server_tool_outputs() {
        // 没有服务端工具时不发这个字段（尤其别带 reasoning.encrypted_content：
        // xAI 的回放会丢弃推理项，要来只是让每次响应多背一份没人消费的密文）。
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
        let output = output_from_sse_body(body).expect("sse output");
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
        let output = output_from_sse_body(body).expect("sse output");
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
        let error = output_from_sse_body(body).expect_err("blank line is required to commit");
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
        let output = output_from_sse_body(body).expect("complete multiline SSE");
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
        let output = output_from_sse_body(body).expect("sse output");
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
        let output = output_from_sse_body(body).expect("sse output");
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
        for data in crate::model::sse::SseDecoder::default().push(body.as_bytes()) {
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
        let output = output_from_sse_body(body).expect("sse output");
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
            let err =
                output_from_sse_body(&format!("data: {event}\n\n")).expect_err("provider failure");
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
            let err =
                output_from_sse_body(&body).expect_err("partial stream must not count as success");
            assert!(err.is_retryable_stream_error(), "{err}");
        }
    }

    /// 没有可用输出时，同一条缺终态 error 仍是失败——不能把空流当成成功。
    #[test]
    fn missing_completed_event_without_output_is_still_error() {
        let body = concat!(
            "data: {\"type\":\"error\",\"message\":\"stream ended without terminal event or completed response\"}\n\n",
        );
        let err = output_from_sse_body(body).expect_err("empty stream should fail");
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
