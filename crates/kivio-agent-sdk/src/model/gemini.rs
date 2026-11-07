//! Shared Gemini Generate Content wire protocol and host-configured HTTP provider.
use super::error::provider_error_code;
use super::{
    parse_tool_arguments, stream_read_error, BuiltinWebSearch, FinishKind, FinishReason,
    GenerateOutput, GenerateRequest, GeneratedImageData, LanguageModelProvider, MessagePart,
    ModelError, ModelFuture, ModelMessage, ModelRole, ModelTool, ModelUsage, PendingToolCall,
    ProviderSignature, StreamPart, StreamSink, WebCitation,
};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GeminiThinkingMode {
    #[default]
    IncludeThoughts,
    Budget(i64),
    Level,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GeminiResponseFormat {
    #[default]
    Native,
    ResponseEnvelope,
}

/// Host-resolved wire defaults. Call options override extra_body; disabling tools wins last.
#[derive(Clone, Debug, Default)]
pub struct GeminiOptions {
    pub temperature: Option<f64>,
    pub thinking: GeminiThinkingMode,
    pub zero_budget_when_disabled: bool,
    pub image_output: bool,
    pub include_tool_ids: bool,
    /// Opaque host identity for native signature replay isolation.
    pub provider_id: Option<String>,
    pub response_format: GeminiResponseFormat,
    pub extra_body: Value,
}

/// Explicit transport configuration. base_url is the native API root, including its base path.
pub struct GeminiConfig {
    pub base_url: reqwest::Url,
    pub api_key: Option<String>,
    pub headers: reqwest::header::HeaderMap,
    pub options: GeminiOptions,
    /// Applies to buffered generation only. Configure stream idle timeouts on the client.
    pub request_timeout: Option<std::time::Duration>,
}

impl GeminiConfig {
    pub fn new(base_url: reqwest::Url) -> Self {
        Self {
            base_url,
            api_key: None,
            headers: Default::default(),
            options: Default::default(),
            request_timeout: None,
        }
    }
}

/// Sends once per invocation. Hosts own retries, credentials, accounting, and model capabilities.
pub struct GeminiProvider {
    client: reqwest::Client,
    config: GeminiConfig,
}

impl GeminiProvider {
    pub fn new(client: reqwest::Client, config: GeminiConfig) -> Self {
        Self { client, config }
    }

    async fn post(
        &self,
        request: &GenerateRequest,
        stream: bool,
    ) -> Result<reqwest::Response, ModelError> {
        let options = GeminiOptions {
            temperature: request
                .options
                .temperature
                .or(self.config.options.temperature),
            ..self.config.options.clone()
        };
        let body = request_body(request, &options);
        if request.messages.iter().any(|message| {
            message
                .content
                .iter()
                .any(|part| matches!(part, MessagePart::Video { .. }))
        }) {
            super::validate_video_body(&body)?;
        }
        let mut http = self
            .client
            .post(gemini_url(
                self.config.base_url.as_str(),
                &request.model,
                stream,
            ))
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .headers(self.config.headers.clone())
            .json(&body);
        if let Some(key) = &self.config.api_key {
            http = http.header("x-goog-api-key", key);
        }
        if !stream {
            if let Some(timeout) = self.config.request_timeout {
                http = http.timeout(timeout);
            }
        }
        super::transport::send(http, &request_label(request, "Gemini API")).await
    }
}

impl LanguageModelProvider for GeminiProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            let label = request_label(&request, "Gemini generateContent");
            let raw = self
                .post(&request, false)
                .await?
                .text()
                .await
                .map_err(|error| stream_read_error(&label, &error))?;
            decode_gemini_body(
                &raw,
                &label,
                &request.model,
                self.config.options.provider_id.as_deref(),
                self.config.options.response_format,
            )
        })
    }
    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            let label = request_label(&request, "Gemini stream");
            decode_gemini_stream(
                self.post(&request, true).await?,
                sink,
                &label,
                &request.model,
                self.config.options.provider_id.as_deref(),
                self.config.options.response_format,
            )
            .await
        })
    }
}

impl crate::agent::provider_runtime::ModelRuntime for GeminiProvider {
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

pub fn request_body(request: &GenerateRequest, options: &GeminiOptions) -> Value {
    let mut body = serde_json::json!({});
    if let Some(extra) = options.extra_body.as_object() {
        body.as_object_mut().unwrap().extend(extra.clone());
    }
    body["contents"] = Value::Array(contents_with_tool_ids(
        request,
        options.include_tool_ids,
        options.provider_id.as_deref(),
    ));
    if !body["generationConfig"].is_object() {
        body["generationConfig"] = serde_json::json!({});
    }
    if request.options.max_tokens > 0 {
        body["generationConfig"]["maxOutputTokens"] = Value::from(request.options.max_tokens);
    }
    if let Some(temperature) = options.temperature {
        body["generationConfig"]["temperature"] = serde_json::json!(temperature);
    }
    if !request.system.trim().is_empty() {
        body["systemInstruction"] = serde_json::json!({"parts":[{"text":request.system}]});
    }
    if options.image_output {
        body["generationConfig"]["responseModalities"] = serde_json::json!(["TEXT", "IMAGE"]);
    }
    let declarations = gemini_function_declarations(&request.tools);
    let mut tools = Vec::new();
    if !declarations.is_empty() {
        tools.push(serde_json::json!({"functionDeclarations":declarations}));
        body["toolConfig"] = serde_json::json!({"functionCallingConfig":{"mode":"AUTO"}});
    }
    if request.options.builtin_web_search {
        tools.push(serde_json::json!({"google_search":{}}));
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if request.options.thinking_enabled {
        let mut thinking = serde_json::json!({"includeThoughts":true});
        match options.thinking {
            GeminiThinkingMode::IncludeThoughts => {}
            GeminiThinkingMode::Budget(budget) => thinking["thinkingBudget"] = Value::from(budget),
            GeminiThinkingMode::Level => {
                if let Some(level) = &request.options.thinking_level {
                    thinking["thinkingLevel"] = Value::String(level.clone());
                }
            }
        }
        body["generationConfig"]["thinkingConfig"] = thinking;
    } else if options.zero_budget_when_disabled {
        body["generationConfig"]["thinkingConfig"] = serde_json::json!({"thinkingBudget":0});
    }
    if let Some(overrides) = request.options.provider_options.as_object() {
        body.as_object_mut().unwrap().extend(overrides.clone());
    }
    if request.options.tool_choice == super::ToolChoice::None {
        body.as_object_mut().unwrap().remove("tools");
        body["toolConfig"] = serde_json::json!({"functionCallingConfig":{"mode":"NONE"}});
    }
    body
}

pub fn decode_gemini_body(
    raw: &str,
    label: &str,
    model: &str,
    provider_id: Option<&str>,
    response_format: GeminiResponseFormat,
) -> Result<GenerateOutput, ModelError> {
    let value: Value = serde_json::from_str(raw).map_err(|error| {
        ModelError::new(format!(
            "{label} parse JSON: {error} (body: {})",
            raw.chars().take(500).collect::<String>()
        ))
    })?;
    let value = unwrap_response(value, response_format);
    let mut output = output_from_gemini_response(&value, label)?;
    for call in &mut output.tool_calls {
        scope_signature(call, model, provider_id);
    }
    Ok(output)
}

fn unwrap_response(mut value: Value, format: GeminiResponseFormat) -> Value {
    if format == GeminiResponseFormat::ResponseEnvelope
        && value.get("error").is_none()
        && value.get("response").is_some()
    {
        value["response"].take()
    } else {
        value
    }
}

fn scope_signature(call: &mut PendingToolCall, model: &str, provider_id: Option<&str>) {
    if let Some(signature) = call.signature.take() {
        call.signature = Some(ProviderSignature::Scoped {
            value: signature.value().to_string(),
            model: model.to_string(),
            provider_id: provider_id.map(str::to_string),
        });
    }
}

pub async fn decode_gemini_stream(
    mut response: reqwest::Response,
    sink: &mut (dyn StreamSink + Send),
    label: &str,
    model: &str,
    provider_id: Option<&str>,
    response_format: GeminiResponseFormat,
) -> Result<GenerateOutput, ModelError> {
    let mut sse = super::sse::SseDecoder::default();
    let mut full = String::new();
    let mut reasoning_full = String::new();
    let mut tool_calls: Vec<PendingToolCall> = Vec::new();
    // thoughtSignature 兜底（跨 chunk 记住，functionCall 自身没有时回退用它）。
    let mut carry_sig: Option<String> = None;
    let mut finish_reason = None;
    let mut usage: Option<ModelUsage> = None;
    let mut saw_terminal = false;
    // 内置搜索引用：groundingMetadata 通常在末段 chunk，逐段合并兜底。
    let mut web_search: Option<BuiltinWebSearch> = None;
    // 模型生成的图片：逐 chunk 的 inlineData part 累积，finish 后并入 output。
    let mut images: Vec<GeneratedImageData> = Vec::new();

    'stream: loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| stream_read_error(label, &error))?;
        let Some(chunk) = chunk else {
            break;
        };
        for data in sse.push(&chunk) {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            let Ok(mut value) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            value = unwrap_response(value, response_format);
            if let Some(err) = gemini_error_message(&value) {
                let code = provider_error_code(&value["error"]);
                return Err(ModelError::from_provider_stream(code.as_deref(), err));
            }
            // 逐 part：text/thought → 增量；functionCall → 完整工具调用。
            // 先整块预扫一个候选签名（thoughtSignature 可能在 functionCall 兄弟 part 上、
            // 且顺序不定），functionCall 自身无签名时回退用它。
            if let Some(sig) = gemini_candidate_signature(&value) {
                carry_sig = Some(sig);
            }
            for part in gemini_response_parts(&value) {
                if let Some(mut call) = gemini_tool_call_from_part(part) {
                    if call.signature.is_none() {
                        call.signature = carry_sig.clone().map(Into::into);
                    }
                    scope_signature(&mut call, model, provider_id);
                    sink.emit(StreamPart::ToolCallStart {
                        id: call.id.clone(),
                        name: call.function_name.clone(),
                    })?;
                    sink.emit(StreamPart::ToolCallDelta {
                        id: call.id.clone(),
                        delta: call.arguments_raw.clone(),
                    })?;
                    sink.emit(StreamPart::ToolCallDone { call: call.clone() })?;
                    tool_calls.push(call);
                } else if let Some(image) = gemini_image_from_part(part) {
                    sink.emit(StreamPart::ImageData {
                        mime_type: image.mime_type.clone(),
                        data: image.data.clone(),
                    })?;
                    images.push(image);
                } else if let Some((text, is_thought)) = gemini_text_from_part(part) {
                    if is_thought {
                        reasoning_full.push_str(&text);
                        sink.emit(StreamPart::ReasoningDelta { delta: text })?;
                    } else {
                        full.push_str(&text);
                        sink.emit(StreamPart::TextDelta { delta: text })?;
                    }
                }
            }
            if let Some(reason) = gemini_finish_reason_str(&value) {
                saw_terminal = true;
                finish_reason = Some(reason);
            }
            if let Some(next_usage) = gemini_usage(&value) {
                if let Some(input_tokens) = next_usage.input_tokens {
                    sink.emit(StreamPart::ContextUsage {
                        input_tokens,
                        output_tokens: next_usage.output_tokens.unwrap_or(0),
                    })?;
                }
                usage = Some(next_usage);
            }
            // 实时卡（任务 07-23）：grounding 通常在末段到达，实时性打折但答案前定位
            // 收益照拿；仅当本 chunk 真有新数据才发（避免空帧），随后并入累加值。
            let chunk_ws = web_search_from_gemini_value(&value);
            if let Some(ws) = chunk_ws.as_ref() {
                if !ws.is_empty() {
                    sink.emit(StreamPart::WebSearch {
                        queries: ws.queries.clone(),
                        citations: ws.citations.clone(),
                    })?;
                }
            }
            merge_gemini_web_search(&mut web_search, chunk_ws);
            if saw_terminal {
                break 'stream;
            }
        }
    }

    if !saw_terminal {
        return Err(ModelError::stream_ended_early(label));
    }
    // 有工具调用则结束原因归一为 tool_calls（Gemini 常仍返回 STOP）。
    let finish_reason = normalize_finish_reason(finish_reason.as_deref(), !tool_calls.is_empty());
    sink.emit(StreamPart::Finish {
        reason: finish_reason.clone(),
        full: full.clone(),
    })?;
    let mut output = stream_output(full, reasoning_full, tool_calls, finish_reason, usage);
    output.web_search = web_search;
    output.images = images;
    Ok(output)
}

/// `{base}/models/{model}:generateContent` 或 `:streamGenerateContent?alt=sse`。
/// base_url 用户配 Gemini 原生根（如 `https://generativelanguage.googleapis.com/v1beta`）。
pub fn gemini_url(base_url: &str, model: &str, stream: bool) -> String {
    let base = base_url.trim_end_matches('/');
    // model 可能已带 "models/" 前缀（如 "models/gemini-3.1-flash-lite"），去重。
    let model = model.trim_start_matches("models/");
    if stream {
        format!("{base}/models/{model}:streamGenerateContent?alt=sse")
    } else {
        format!("{base}/models/{model}:generateContent")
    }
}

/// canonical messages → Gemini `contents[]`。Tool 结果按函数名关联（Gemini 无 call id），
/// 先扫 assistant 的 functionCall 建 `tool_call_id → name` 映射。
pub fn gemini_contents_from_generate_request(request: &GenerateRequest) -> Vec<Value> {
    contents_with_tool_ids(request, false, None)
}

fn contents_with_tool_ids(
    request: &GenerateRequest,
    include_ids: bool,
    provider_id: Option<&str>,
) -> Vec<Value> {
    let id_to_name = tool_call_id_to_name(&request.messages);
    let mut contents: Vec<Value> = Vec::new();
    for message in super::types::paired_model_messages(&request.messages).iter() {
        let role = match message.role {
            ModelRole::Assistant => "model",
            // Gemini contents only support user/model; tool and developer messages use user.
            ModelRole::User | ModelRole::Developer | ModelRole::Tool => "user",
        };
        let parts = gemini_parts_from_message(
            message,
            &id_to_name,
            include_ids,
            &request.model,
            provider_id,
        );
        if parts.is_empty() {
            continue;
        }
        contents.push(serde_json::json!({ "role": role, "parts": parts }));
    }
    merge_consecutive_gemini_roles(&mut contents);
    contents
}

fn tool_call_id_to_name(messages: &[ModelMessage]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for message in messages {
        for part in &message.content {
            if let MessagePart::ToolCall { id, name, .. } = part {
                if !id.is_empty() {
                    map.insert(id.clone(), name.clone());
                }
            }
        }
    }
    map
}

fn gemini_parts_from_message(
    message: &ModelMessage,
    id_to_name: &std::collections::HashMap<String, String>,
    include_ids: bool,
    model: &str,
    provider_id: Option<&str>,
) -> Vec<Value> {
    let mut parts = Vec::new();
    for part in &message.content {
        match part {
            MessagePart::Video {
                mime_type, data, ..
            } => {
                if data.is_empty() {
                    parts.push(serde_json::json!({"text":"[视频附件不可用，请重新添加]"}));
                } else {
                    // Generate Content documents QuickTime; retain legacy stored MIME
                    // and other providers' video/mov wire format outside this adapter.
                    let mime_type = if mime_type == "video/mov" {
                        "video/quicktime"
                    } else {
                        mime_type.as_str()
                    };
                    parts
                        .push(serde_json::json!({"inlineData":{"mimeType":mime_type,"data":data}}));
                }
            }
            MessagePart::Text { text } => {
                if !text.is_empty() {
                    parts.push(serde_json::json!({ "text": text }));
                }
            }
            MessagePart::Image {
                mime_type, data, ..
            } => {
                if matches!(message.role, ModelRole::User | ModelRole::Developer) {
                    if data.is_empty() {
                        parts.push(serde_json::json!({
                            "text": super::MISSING_IMAGE_PLACEHOLDER
                        }));
                    } else {
                        parts.push(serde_json::json!({
                            "inlineData": { "mimeType": mime_type, "data": data }
                        }));
                    }
                }
            }
            MessagePart::ImageUrl { url, .. } => {
                if matches!(message.role, ModelRole::User | ModelRole::Developer) {
                    parts.push(serde_json::json!({ "fileData": { "fileUri": url } }));
                }
            }
            MessagePart::ToolCall {
                id,
                name,
                arguments,
                arguments_raw,
                signature,
                ..
            } => {
                if matches!(message.role, ModelRole::Assistant) {
                    let args = if arguments.is_null() {
                        serde_json::from_str(arguments_raw).unwrap_or(serde_json::json!({}))
                    } else {
                        arguments.clone()
                    };
                    let mut part = serde_json::json!({
                        "functionCall": { "name": name, "args": args }
                    });
                    if include_ids {
                        part["functionCall"]["id"] = Value::String(id.clone());
                    }
                    // Gemini 3.x：回放 functionCall 必须带回响应给的 thoughtSignature，否则 400。
                    if let Some(sig) = signature
                        .as_ref()
                        .and_then(|signature| signature.replay_for(model, provider_id))
                    {
                        part["thoughtSignature"] = Value::String(sig.to_string());
                    }
                    parts.push(part);
                }
            }
            MessagePart::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                // Gemini 按函数名关联；用 id→name 映射还原，回退用 id 本身。
                let name = id_to_name
                    .get(tool_call_id)
                    .cloned()
                    .unwrap_or_else(|| tool_call_id.clone());
                // response 必须是 JSON 对象；字符串输出包进 { output }。
                let response = serde_json::from_str::<Value>(content)
                    .ok()
                    .filter(|v| v.is_object())
                    .unwrap_or_else(|| serde_json::json!({ "output": content }));
                let mut part = serde_json::json!({
                    "functionResponse": { "name": name, "response": response }
                });
                if include_ids {
                    part["functionResponse"]["id"] = Value::String(tool_call_id.clone());
                }
                parts.push(part);
            }
            MessagePart::Reasoning { .. } => {
                // 思维文本回放时丢弃（thoughtSignature 未保存；对连续性影响可接受）。
            }
            MessagePart::AnthropicNativeBlock { block, .. } => {
                if block["type"] == "text" {
                    if let Some(text) = block["text"].as_str() {
                        parts.push(serde_json::json!({"text":text}));
                    }
                }
            }
            // OpenAI Responses 专属的原生推理项：其它协议不识别，回放时忽略。
            MessagePart::ReasoningItem { .. }
            | MessagePart::AnthropicThinking { .. }
            | MessagePart::AnthropicRedactedThinking { .. } => {}
        }
    }
    parts
}

fn merge_consecutive_gemini_roles(contents: &mut Vec<Value>) {
    if contents.len() < 2 {
        return;
    }
    let mut i = 1;
    while i < contents.len() {
        let prev_role = contents[i - 1]
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let curr_role = contents[i]
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if prev_role == curr_role {
            let curr_parts = contents[i]
                .get("parts")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if let Some(prev) = contents[i - 1]
                .get_mut("parts")
                .and_then(|v| v.as_array_mut())
            {
                prev.extend(curr_parts);
            }
            contents.remove(i);
        } else {
            i += 1;
        }
    }
}

pub fn gemini_function_declarations(tools: &[ModelTool]) -> Vec<Value> {
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
            "parameters": normalize_gemini_schema(tool.input_schema.clone()),
        }));
    }
    out
}

/// Gemini 只收 OpenAPI 子集：剥 JSON Schema 专有键（`$schema`/`additionalProperties`/
/// `$defs` 等），nullable `anyOf` 折成非空分支。递归处理 properties/items/组合子。
/// `required` 只允许出现在 `type: object` 上，否则 Gemini 400
///（`required: only allowed for OBJECT type`，issue #33）。
fn normalize_gemini_schema(schema: Value) -> Value {
    match schema {
        Value::Object(mut map) => {
            // nullable anyOf（[T, null]）→ 取非空分支。
            if let Some(any_of) = map.get("anyOf").and_then(|v| v.as_array()).cloned() {
                if any_of.len() == 2
                    && any_of
                        .iter()
                        .any(|it| it.get("type").and_then(|v| v.as_str()) == Some("null"))
                {
                    if let Some(non_null) = any_of
                        .iter()
                        .find(|it| it.get("type").and_then(|v| v.as_str()) != Some("null"))
                    {
                        let mut result = normalize_gemini_schema(non_null.clone());
                        if let (Some(obj), Some(desc)) =
                            (result.as_object_mut(), map.get("description"))
                        {
                            obj.insert("description".into(), desc.clone());
                        }
                        return result;
                    }
                }
            }
            // 约束型组合子（所有分支都无 type，只有 required 等纯约束）：Vertex 要求
            // anyOf 分支是带 type 的完整 schema，此形态直接剥掉，其余字段保留。
            for key in ["anyOf", "oneOf", "allOf"] {
                let constraint_only = map
                    .get(key)
                    .and_then(|v| v.as_array())
                    .is_some_and(|branches| branches.iter().all(|it| it.get("type").is_none()));
                if constraint_only {
                    map.remove(key);
                }
            }
            for key in [
                "$schema",
                "additionalProperties",
                "$defs",
                "definitions",
                "$id",
                "$ref",
                "title",
            ] {
                map.remove(key);
            }
            if let Some(props) = map.get_mut("properties").and_then(|v| v.as_object_mut()) {
                for value in props.values_mut() {
                    *value = normalize_gemini_schema(value.clone());
                }
            }
            if let Some(items) = map.get("items").cloned() {
                map.insert("items".into(), normalize_gemini_schema(items));
            }
            // 留下的 typed anyOf/oneOf/allOf 也要递归，否则分支里的非 object
            // `required` 会原样透传给 Gemini。
            for key in ["anyOf", "oneOf", "allOf"] {
                if let Some(branches) = map.get(key).and_then(|v| v.as_array()).cloned() {
                    let normalized: Vec<Value> =
                        branches.into_iter().map(normalize_gemini_schema).collect();
                    map.insert(key.into(), Value::Array(normalized));
                }
            }
            // 有 properties 却没写 type 的 MCP schema：补成 object，required 才合法。
            if map.contains_key("properties") && map.get("type").is_none() {
                map.insert("type".into(), Value::String("object".into()));
            }
            let is_object = map.get("type").and_then(|v| v.as_str()) == Some("object");
            if !is_object {
                map.remove("required");
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn output_from_gemini_response(value: &Value, label: &str) -> Result<GenerateOutput, ModelError> {
    if let Some(msg) = gemini_error_message(value) {
        let code = provider_error_code(&value["error"]);
        return Err(ModelError::from_provider_payload(
            code.as_deref(),
            format!("{label}: {msg}"),
        ));
    }
    let mut content_parts: Vec<String> = Vec::new();
    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<PendingToolCall> = Vec::new();
    let mut images: Vec<GeneratedImageData> = Vec::new();
    // thoughtSignature 可能在 functionCall 兄弟 part 上：先扫全 parts 取一个候选签名兜底。
    let carry_sig = gemini_candidate_signature(value);
    for part in gemini_response_parts(value) {
        if let Some(mut call) = gemini_tool_call_from_part(part) {
            if call.signature.is_none() {
                call.signature = carry_sig.clone().map(Into::into);
            }
            tool_calls.push(call);
        } else if let Some(image) = gemini_image_from_part(part) {
            images.push(image);
        } else if let Some((text, is_thought)) = gemini_text_from_part(part) {
            if is_thought {
                reasoning_parts.push(text);
            } else {
                content_parts.push(text);
            }
        }
    }
    let finish_reason = normalize_finish_reason(
        gemini_finish_reason_str(value).as_deref(),
        !tool_calls.is_empty(),
    );
    let content = content_parts.join("");
    let reasoning = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join(""))
    };
    let usage = gemini_usage(value);
    Ok(GenerateOutput {
        text: content,
        reasoning,
        tool_calls,
        usage,
        finish_reason: Some(finish_reason),
        cancelled: false,
        web_search: web_search_from_gemini_value(value),
        images,
        reasoning_items: Vec::new(),
        canonical_message: None,
        billing_usage: None,
    })
}

/// 从 Gemini 响应 `candidates[0].groundingMetadata` 解析内置搜索痕迹：
/// `webSearchQueries[]` → 查询词；`groundingChunks[].web.{uri,title}` → 来源。
/// 全空则 None。片段响应亦同结构（末段带 groundingMetadata），故流式逐段调用累积。
fn web_search_from_gemini_value(value: &Value) -> Option<BuiltinWebSearch> {
    let grounding = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|cand| cand.get("groundingMetadata"))?;
    let mut result = BuiltinWebSearch::default();
    if let Some(queries) = grounding.get("webSearchQueries").and_then(Value::as_array) {
        for query in queries {
            if let Some(query) = query
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !result.queries.iter().any(|existing| existing == query) {
                    result.queries.push(query.to_string());
                }
            }
        }
    }
    // 块下标 → citations 下标（groundingSupports.groundingChunkIndices 引的是
    // groundingChunks 数组下标，非 web 块或重复 url 记 None）。
    let mut chunk_to_citation: Vec<Option<usize>> = Vec::new();
    if let Some(chunks) = grounding.get("groundingChunks").and_then(Value::as_array) {
        for chunk in chunks {
            let Some(web) = chunk.get("web") else {
                chunk_to_citation.push(None);
                continue;
            };
            let Some(url) = web
                .get("uri")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                chunk_to_citation.push(None);
                continue;
            };
            if result.citations.iter().any(|c| c.url == url) {
                chunk_to_citation.push(None);
                continue;
            }
            let title = web
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
            chunk_to_citation.push(Some(result.citations.len() - 1));
        }
    }
    // groundingSupports[]:segment.text 是「答案正文里支持某来源」的片段，
    // groundingChunkIndices 指向它支撑的块。把它映射成对应来源的 snippet（首个命中即止）。
    if let Some(supports) = grounding.get("groundingSupports").and_then(Value::as_array) {
        for support in supports {
            let Some(text) = support
                .get("segment")
                .and_then(|segment| segment.get("text"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(indices) = support
                .get("groundingChunkIndices")
                .and_then(Value::as_array)
            else {
                continue;
            };
            for index in indices.iter().filter_map(Value::as_u64) {
                let Some(Some(citation_index)) = chunk_to_citation.get(index as usize).copied()
                else {
                    continue;
                };
                if result.citations[citation_index].snippet.is_none() {
                    result.citations[citation_index].snippet =
                        Some(text.chars().take(400).collect());
                }
            }
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// 把新解析出的内置搜索片段合并进累积值（流式逐段调用；去重按 url / query 文本）。
/// 同一 url 的后到片段若带了 snippet/published_date 而累积值没有，则补全
/// （groundingSupports 可能随后续流式段才到，而 chunk 首段就建立了引用）。
fn merge_gemini_web_search(acc: &mut Option<BuiltinWebSearch>, next: Option<BuiltinWebSearch>) {
    let Some(next) = next else {
        return;
    };
    let target = acc.get_or_insert_with(BuiltinWebSearch::default);
    for query in next.queries {
        if !target.queries.iter().any(|existing| *existing == query) {
            target.queries.push(query);
        }
    }
    for citation in next.citations {
        if let Some(existing) = target.citations.iter_mut().find(|c| c.url == citation.url) {
            if existing.snippet.is_none() {
                existing.snippet = citation.snippet;
            }
            if existing.published_date.is_none() {
                existing.published_date = citation.published_date;
            }
            continue;
        }
        target.citations.push(citation);
    }
}

/// 取 `candidates[0].content.parts[]`（片段响应亦同）。
fn gemini_response_parts(value: &Value) -> impl Iterator<Item = &Value> {
    value
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|cand| cand.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|p| p.as_array())
        .map(|v| v.iter())
        .unwrap_or_else(|| [].iter())
}

/// part 里的 functionCall → PendingToolCall（合成 id；args 是对象；捕获同 part 上的 thoughtSignature）。
fn gemini_tool_call_from_part(part: &Value) -> Option<PendingToolCall> {
    let call = part.get("functionCall")?;
    let name = call.get("name").and_then(|v| v.as_str())?.to_string();
    let args = call.get("args").cloned().unwrap_or(serde_json::json!({}));
    let arguments_raw = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string());
    let (arguments, arguments_parse_error) = parse_tool_arguments(&arguments_raw);
    Some(PendingToolCall {
        id: call
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4())),
        function_name: name,
        arguments,
        arguments_raw,
        arguments_parse_error,
        signature: gemini_part_thought_signature(part).map(Into::into),
    })
}

/// part 里的 text → (文本, 是否思维)。空文本返回 None（如仅带 thoughtSignature 的占位 part）。
fn gemini_text_from_part(part: &Value) -> Option<(String, bool)> {
    let text = part.get("text").and_then(|v| v.as_str())?;
    if text.is_empty() {
        return None;
    }
    let is_thought = part
        .get("thought")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some((text.to_string(), is_thought))
}

/// part 里的 inlineData → 模型生成的图片（base64 + mime）。非图片 part 返回 None。
/// wire 形状：`{"inlineData": {"mimeType": "image/png", "data": "<base64>"}}`。
fn gemini_image_from_part(part: &Value) -> Option<GeneratedImageData> {
    let inline = part.get("inlineData")?;
    let data = inline
        .get("data")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    let mime_type = inline
        .get("mimeType")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("image/png")
        .to_string();
    Some(GeneratedImageData {
        mime_type,
        data: data.to_string(),
    })
}

/// part 上的 thoughtSignature（Gemini 3.x 思维签名，回放 functionCall 时须带回）。
fn gemini_part_thought_signature(part: &Value) -> Option<String> {
    part.get("thoughtSignature")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 扫一个候选（或流片段）里所有 part，取第一个 thoughtSignature——签名可能不在
/// functionCall part 自身上，而在同轮兄弟 part 上，且顺序不定。
fn gemini_candidate_signature(value: &Value) -> Option<String> {
    gemini_response_parts(value).find_map(gemini_part_thought_signature)
}

fn gemini_finish_reason_str(value: &Value) -> Option<String> {
    value
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|cand| cand.get("finishReason"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Gemini finishReason → canonical。有工具调用时恒为 tool_calls（Gemini 常仍返回 STOP）。
fn normalize_finish_reason(reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
    if has_tool_calls {
        return FinishReason::new(FinishKind::ToolCalls, reason.unwrap_or_default());
    }
    let Some(reason) = reason else {
        return FinishReason::new(FinishKind::Stop, "");
    };
    if reason.eq_ignore_ascii_case("stop") {
        return FinishReason::new(FinishKind::Stop, reason);
    }
    if reason == "MAX_TOKENS" {
        return FinishReason::new(FinishKind::Length, reason);
    }
    let legacy = reason.to_ascii_lowercase();
    FinishReason::new(FinishReason::from_openai(&legacy).kind, reason).with_legacy_name(&legacy)
}

fn gemini_usage(value: &Value) -> Option<ModelUsage> {
    let meta = value.get("usageMetadata")?;
    let get = |key: &str| meta.get(key).and_then(|v| v.as_u64());
    Some(ModelUsage {
        input_tokens: get("promptTokenCount"),
        output_tokens: get("candidatesTokenCount"),
        total_tokens: get("totalTokenCount"),
        cached_input_tokens: get("cachedContentTokenCount"),
        cache_creation_input_tokens: None,
        reasoning_tokens: get("thoughtsTokenCount"),
        // 内置 provider 路径：窗口来自 model_metadata，不由响应携带。
        context_window_tokens: None,
        context_tokens: None,
    })
}

fn gemini_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    let message = error.get("message")?.as_str()?;
    Some(match error.get("status").and_then(Value::as_str) {
        Some(status) => format!("{message} ({status})"),
        None => message.to_string(),
    })
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
        text,
        reasoning,
        tool_calls,
        usage,
        finish_reason: Some(finish_reason),
        cancelled: false,
        web_search: None,
        images: Vec::new(),
        reasoning_items: Vec::new(),
        canonical_message: None,
        billing_usage: None,
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
    use super::super::GenerateOptions;
    use super::*;
    fn body_for(request: &GenerateRequest, _stream: bool) -> Value {
        request_body(request, &GeminiOptions::default())
    }

    #[test]
    fn signed_calls_preserve_specific_signatures_and_isolate_replay_after_storage() {
        let raw = serde_json::json!({"candidates":[{"content":{"parts":[
            {"text":"", "thoughtSignature":"fallback"},
            {"functionCall":{"id":"first", "name":"echo", "args":{}}, "thoughtSignature":"one"},
            {"functionCall":{"id":"second", "name":"echo", "args":{}}, "thoughtSignature":"two"},
            {"functionCall":{"id":"third", "name":"echo", "args":{}}}
        ]}, "finishReason":"STOP"}]})
        .to_string();
        let output = decode_gemini_body(
            &raw,
            "fixture",
            "custom-model",
            Some("account-a"),
            GeminiResponseFormat::Native,
        )
        .unwrap();
        assert_eq!(
            output
                .tool_calls
                .iter()
                .map(|call| call.signature.as_ref().unwrap().value())
                .collect::<Vec<_>>(),
            ["one", "two", "fallback"]
        );
        let saved = serde_json::to_vec(&output.replay_message()).unwrap();
        let mut request = GenerateRequest {
            model: "custom-model".into(),
            system: String::new(),
            messages: vec![serde_json::from_slice(&saved).unwrap()],
            tools: vec![],
            options: Default::default(),
            metadata: Default::default(),
        };
        for (model, provider, expected) in [
            ("custom-model", Some("account-a"), Some("one")),
            ("another-model", Some("account-a"), None),
            ("custom-model", Some("account-b"), None),
            ("custom-model", None, Some("one")),
        ] {
            request.model = model.into();
            let body = request_body(
                &request,
                &GeminiOptions {
                    provider_id: provider.map(str::to_string),
                    ..Default::default()
                },
            );
            let call = &body["contents"][0]["parts"][0];
            assert_eq!(call["thoughtSignature"].as_str(), expected);
            assert_eq!(call["functionCall"]["name"], "echo");
        }
    }

    #[test]
    fn explicit_thinking_modes_and_tool_disable_override_extensions() {
        let mut request = GenerateRequest {
            model: "custom-model".into(),
            system: String::new(),
            messages: vec![],
            tools: vec![],
            options: GenerateOptions {
                thinking_enabled: true,
                thinking_level: Some("high".into()),
                builtin_web_search: true,
                ..Default::default()
            },
            metadata: Default::default(),
        };
        for (mode, field, expected) in [
            (
                GeminiThinkingMode::Budget(2048),
                "thinkingBudget",
                serde_json::json!(2048),
            ),
            (
                GeminiThinkingMode::Level,
                "thinkingLevel",
                serde_json::json!("high"),
            ),
        ] {
            let body = request_body(
                &request,
                &GeminiOptions {
                    thinking: mode,
                    ..Default::default()
                },
            );
            let thinking = &body["generationConfig"]["thinkingConfig"];
            assert_eq!(thinking[field], expected);
            assert_eq!(thinking["includeThoughts"], true);
            assert_eq!(thinking.as_object().unwrap().len(), 2);
        }
        request.options.thinking_enabled = false;
        let body = request_body(
            &request,
            &GeminiOptions {
                zero_budget_when_disabled: true,
                ..Default::default()
            },
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"],
            serde_json::json!({"thinkingBudget":0})
        );
        request.options.tool_choice = super::super::ToolChoice::None;
        request.options.provider_options = serde_json::json!({"tools":[{"google_search":{}}], "toolConfig":{"functionCallingConfig":{"mode":"ANY"}}});
        let body = request_body(
            &request,
            &GeminiOptions {
                extra_body: serde_json::json!({"tools":[{"functionDeclarations":[]}]}),
                ..Default::default()
            },
        );
        assert!(body.get("tools").is_none());
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "NONE");
    }

    async fn streamed(
        events: Vec<Value>,
        format: GeminiResponseFormat,
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
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let response = reqwest::Client::new().get(endpoint).send().await.unwrap();
        let mut parts = Vec::new();
        let output = decode_gemini_stream(
            response,
            &mut |part| {
                parts.push(part);
                Ok(())
            },
            "fixture",
            "custom-model",
            Some("account-a"),
            format,
        )
        .await;
        server.await.unwrap();
        (parts, output)
    }

    #[tokio::test]
    async fn stream_carries_signatures_and_preserves_images_reasoning_and_usage() {
        let (parts, output) = streamed(vec![
            serde_json::json!({"response":{"candidates":[{"content":{"parts":[{"text":"plan", "thought":true, "thoughtSignature":"previous"}]}}]}}),
            serde_json::json!({"response":{"candidates":[{"content":{"parts":[
                {"functionCall":{"id":"first", "name":"echo", "args":{}}},
                {"inlineData":{"mimeType":"image/png", "data":"AAAA"}},
                {"text":"answer"}
            ]}, "finishReason":"STOP"}], "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":7,"thoughtsTokenCount":2,"cachedContentTokenCount":1,"totalTokenCount":12}}}),
        ], GeminiResponseFormat::ResponseEnvelope).await;
        let output = output.unwrap();
        assert_eq!(output.text, "answer");
        assert_eq!(output.reasoning.as_deref(), Some("plan"));
        assert_eq!(output.images[0].data, "AAAA");
        let usage = output.usage.unwrap();
        assert_eq!(usage.cached_input_tokens, Some(1));
        assert_eq!(usage.reasoning_tokens, Some(2));
        assert_eq!(usage.output_tokens, Some(7));
        let call = parts
            .iter()
            .find_map(|part| {
                if let StreamPart::ToolCallDone { call } = part {
                    Some(call)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(
            call.signature
                .as_ref()
                .unwrap()
                .replay_for("custom-model", Some("account-a")),
            Some("previous")
        );
        assert_eq!(
            parts
                .iter()
                .filter(|part| matches!(part, StreamPart::Finish { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn envelope_errors_win_and_missing_terminal_is_rejected() {
        let error = serde_json::json!({"error":{"status":"RESOURCE_EXHAUSTED", "message":"slow down"}, "response":{"candidates":[{"content":{"parts":[{"text":"ignored"}]}, "finishReason":"STOP"}]}});
        assert_eq!(
            decode_gemini_body(
                &error.to_string(),
                "fixture",
                "m",
                None,
                GeminiResponseFormat::ResponseEnvelope
            )
            .unwrap_err()
            .kind,
            super::super::ModelErrorKind::RateLimited
        );
        let (parts, output) = streamed(vec![error], GeminiResponseFormat::ResponseEnvelope).await;
        assert!(output.is_err());
        assert!(parts.is_empty());
        let (parts, output) = streamed(
            vec![serde_json::json!({"candidates":[{"content":{"parts":[{"text":"partial"}]}}]})],
            GeminiResponseFormat::Native,
        )
        .await;
        assert!(super::super::is_missing_stream_terminal_error(
            &output.unwrap_err().message
        ));
        assert!(!parts
            .iter()
            .any(|part| matches!(part, StreamPart::Finish { .. })));
    }
    #[test]
    fn builtin_web_search_appends_google_search_tool() {
        let base = GenerateRequest {
            model: "gemini-3.1-flash-lite".into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        };
        // off ⇒ 无 tools。
        let off = body_for(&base, false);
        assert!(off.get("tools").is_none(), "body: {off}");
        // on ⇒ tools 数组含 {"google_search":{}}。
        let mut req = base.clone();
        req.options.builtin_web_search = true;
        let on = body_for(&req, false);
        let tools = on["tools"].as_array().expect("tools array present");
        assert!(
            tools.iter().any(|t| t.get("google_search").is_some()),
            "body: {on}"
        );
    }

    #[test]
    fn web_search_parsed_from_grounding_metadata() {
        // groundingMetadata.webSearchQueries → 查询；groundingChunks[].web → 来源（去重）；
        // groundingSupports[].segment.text → 按 groundingChunkIndices 映射成对应来源的 snippet。
        let value = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "见来源" }] },
                "groundingMetadata": {
                    "webSearchQueries": ["吉林 天气", "吉林 天气"],
                    "groundingChunks": [
                        { "web": { "uri": "https://a.com", "title": "A 站" } },
                        { "web": { "uri": "https://a.com", "title": "dup" } },
                        { "web": { "uri": "https://b.com" } }
                    ],
                    "groundingSupports": [
                        {
                            "segment": { "text": "长春今天晴。" },
                            "groundingChunkIndices": [0, 5]
                        },
                        {
                            "segment": { "text": "来源见气象台。" },
                            "groundingChunkIndices": [2]
                        },
                        {
                            "segment": { "text": "" },
                            "groundingChunkIndices": [0]
                        }
                    ]
                }
            }]
        });
        let parsed = web_search_from_gemini_value(&value).expect("web_search present");
        assert_eq!(parsed.queries, vec!["吉林 天气".to_string()]);
        assert_eq!(parsed.citations.len(), 2);
        assert_eq!(parsed.citations[0].url, "https://a.com");
        assert_eq!(parsed.citations[0].snippet.as_deref(), Some("长春今天晴。"));
        assert_eq!(parsed.citations[1].title, "https://b.com");
        assert_eq!(
            parsed.citations[1].snippet.as_deref(),
            Some("来源见气象台。")
        );
        assert!(parsed.citations[0].published_date.is_none());
    }

    #[test]
    fn web_search_none_without_grounding() {
        let value = serde_json::json!({
            "candidates": [{ "content": { "parts": [{ "text": "无搜索" }] } }]
        });
        assert!(web_search_from_gemini_value(&value).is_none());
    }

    #[test]
    fn merge_gemini_web_search_accumulates_and_dedups_across_chunks() {
        // 流式逐段调用 merge：query/url 去重，跨片段累积。
        let mut acc: Option<BuiltinWebSearch> = None;
        merge_gemini_web_search(&mut acc, None); // 无 grounding 片段不建结构。
        assert!(acc.is_none());
        merge_gemini_web_search(
            &mut acc,
            Some(BuiltinWebSearch {
                queries: vec!["q1".into()],
                citations: vec![WebCitation {
                    title: "A".into(),
                    url: "https://a.com".into(),
                    ..Default::default()
                }],
            }),
        );
        // 第二段带重复 url + 重复 query + 新来源；重复 url 带 snippet 应补全进首条。
        merge_gemini_web_search(
            &mut acc,
            Some(BuiltinWebSearch {
                queries: vec!["q1".into(), "q2".into()],
                citations: vec![
                    WebCitation {
                        title: "A-dup".into(),
                        url: "https://a.com".into(),
                        snippet: Some("天气不错".into()),
                        ..Default::default()
                    },
                    WebCitation {
                        title: "B".into(),
                        url: "https://b.com".into(),
                        ..Default::default()
                    },
                ],
            }),
        );
        let merged = acc.expect("accumulated");
        assert_eq!(merged.queries, vec!["q1".to_string(), "q2".to_string()]);
        assert_eq!(merged.citations.len(), 2);
        assert_eq!(merged.citations[0].url, "https://a.com");
        assert_eq!(merged.citations[0].snippet.as_deref(), Some("天气不错"));
        assert_eq!(merged.citations[1].url, "https://b.com");
    }

    #[test]
    fn url_builds_generate_and_stream_and_dedupes_models_prefix() {
        let base = "https://generativelanguage.googleapis.com/v1beta/";
        assert_eq!(
            gemini_url(base, "gemini-3.1-flash-lite", false),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-lite:generateContent"
        );
        assert_eq!(
            gemini_url(base, "models/gemini-3.1-flash-lite", true),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-lite:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn video_request_uses_inline_blob_and_normalizes_legacy_mov() {
        let request = GenerateRequest {
            model: "gemini-3.8-flash".into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Video {
                    mime_type: "video/mov".into(),
                    data: "AA==".into(),
                    path: None,
                }],
            }],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        };
        for stream in [false, true] {
            let body = body_for(&request, stream);
            assert_eq!(
                body["contents"][0]["parts"][0],
                serde_json::json!({
                    "inlineData": {"mimeType": "video/quicktime", "data": "AA=="}
                })
            );
        }
    }

    #[test]
    fn request_body_shape_and_no_openai_specific_fields() {
        let request = GenerateRequest {
            model: "gemini-3.1-flash-lite".into(),
            system: "sys".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: vec![ModelTool {
                name: "glob".into(),
                description: "find files".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "additionalProperties": false,
                    "properties": { "pattern": { "type": "string" } },
                    "required": ["pattern"]
                }),
            }],
            options: GenerateOptions::default(),
            metadata: Default::default(),
        };
        let body = body_for(&request, true);
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "hi");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 8192);
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
        let decl = &body["tools"][0]["functionDeclarations"][0];
        assert_eq!(decl["name"], "glob");
        // schema 归一化：JSON Schema 专有键被剥掉。
        assert!(decl["parameters"].get("$schema").is_none());
        assert!(decl["parameters"].get("additionalProperties").is_none());
        assert_eq!(
            decl["parameters"]["properties"]["pattern"]["type"],
            "string"
        );
        // 绝不含 OpenAI 专有字段（撞 Gemini 400 的元凶）。
        assert!(body.get("promptCacheKey").is_none());
        assert!(body.get("prompt_cache_key").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("stream_options").is_none());
        assert!(body.get("model").is_none()); // model 在 URL，不在 body
    }

    #[test]
    fn request_body_omits_max_output_tokens_when_zero() {
        let mut request = GenerateRequest {
            model: "gemini-3.1-flash-lite".into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions {
                max_tokens: 0,
                ..Default::default()
            },
            metadata: Default::default(),
        };
        let body = body_for(&request, false);
        assert!(
            body["generationConfig"].get("maxOutputTokens").is_none(),
            "unknown models must not invent a cap: {body}"
        );
        request.options.max_tokens = 8192;
        let capped = body_for(&request, false);
        assert_eq!(capped["generationConfig"]["maxOutputTokens"], 8192);
    }

    #[test]
    fn tool_round_trip_maps_functioncall_and_functionresponse_by_name() {
        let request = GenerateRequest {
            model: "gemini".into(),
            system: String::new(),
            messages: vec![
                ModelMessage {
                    role: ModelRole::Assistant,
                    content: vec![MessagePart::ToolCall {
                        id: "call_abc".into(),
                        name: "glob".into(),
                        arguments: serde_json::json!({ "pattern": "*.rs" }),
                        arguments_raw: "{\"pattern\":\"*.rs\"}".into(),
                        signature: Some("SIG123".into()),
                    }],
                },
                ModelMessage {
                    role: ModelRole::Tool,
                    content: vec![MessagePart::ToolResult {
                        tool_call_id: "call_abc".into(),
                        content: "found 5 files".into(),
                        is_error: false,
                        artifacts: Vec::new(),
                    }],
                },
            ],
            tools: Vec::new(),
            options: Default::default(),
            metadata: Default::default(),
        };
        let contents = gemini_contents_from_generate_request(&request);
        // assistant functionCall（args 为对象）
        assert_eq!(contents[0]["role"], "model");
        assert_eq!(contents[0]["parts"][0]["functionCall"]["name"], "glob");
        assert_eq!(
            contents[0]["parts"][0]["functionCall"]["args"]["pattern"],
            "*.rs"
        );
        // thoughtSignature 回放时带回（Gemini 3.x 必需）。
        assert_eq!(contents[0]["parts"][0]["thoughtSignature"], "SIG123");
        // tool functionResponse：按 call id → name 还原为 "glob"；字符串输出包成 { output }
        assert_eq!(contents[1]["role"], "user");
        let fr = &contents[1]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "glob");
        assert_eq!(fr["response"]["output"], "found 5 files");
        let antigravity_contents = contents_with_tool_ids(&request, true, None);
        assert_eq!(
            antigravity_contents[0]["parts"][0]["functionCall"]["id"],
            "call_abc"
        );
        assert_eq!(
            antigravity_contents[1]["parts"][0]["functionResponse"]["id"],
            "call_abc"
        );
        assert_eq!(
            antigravity_contents[0]["parts"][0]["thoughtSignature"],
            "SIG123"
        );
    }

    #[test]
    fn parses_gemini_response_text_and_finish() {
        let response = serde_json::json!({
            "candidates": [{
                "content": { "role": "model", "parts": [
                    { "text": "收到" },
                    { "text": "", "thoughtSignature": "abc" }
                ] },
                "finishReason": "STOP"
            }],
            "usageMetadata": { "promptTokenCount": 9449, "candidatesTokenCount": 7, "totalTokenCount": 9456 }
        });
        let out = output_from_gemini_response(&response, "test").expect("output");
        assert_eq!(out.text, "收到");
        assert_eq!(
            out.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("stop")
        );
        assert!(out.tool_calls.is_empty());
        assert_eq!(out.usage.as_ref().and_then(|u| u.total_tokens), Some(9456));
        assert_eq!(out.usage.as_ref().and_then(|u| u.input_tokens), Some(9449));
    }

    #[test]
    fn parses_gemini_response_functioncall_forces_tool_calls_finish() {
        let response = serde_json::json!({
            "candidates": [{
                "content": { "role": "model", "parts": [
                    { "functionCall": { "name": "glob", "args": { "pattern": "*.rs" } } }
                ] },
                "finishReason": "STOP"  // Gemini 有工具调用仍常返回 STOP
            }]
        });
        let out = output_from_gemini_response(&response, "test").expect("output");
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].function_name, "glob");
        assert_eq!(out.tool_calls[0].arguments["pattern"], "*.rs");
        assert!(out.tool_calls[0].id.starts_with("call_"));
        // 由 functionCall 存在推导 tool_calls（而非 STOP→stop）
        assert_eq!(
            out.finish_reason.as_ref().map(FinishReason::legacy_name),
            Some("tool_calls")
        );
    }

    #[test]
    fn normalize_strips_constraint_only_anyof() {
        // present_artifacts 形态：顶层 anyOf 各分支只有 required 无 type，Vertex 会拒。
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "artifact_ids": { "type": "array", "items": { "type": "string" } },
                "paths": { "type": "array", "items": { "type": "string" } },
                "caption": { "type": "string" }
            },
            "anyOf": [
                { "required": ["artifact_ids"] },
                { "required": ["paths"] }
            ]
        });
        let out = normalize_gemini_schema(schema);
        assert!(out.get("anyOf").is_none());
        assert_eq!(out["type"], "object");
        assert_eq!(out["properties"]["caption"]["type"], "string");
    }

    #[test]
    fn normalize_strips_nested_constraint_only_combinators() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "inner": {
                    "type": "object",
                    "properties": { "a": { "type": "string" } },
                    "oneOf": [ { "required": ["a"] } ]
                }
            },
            "allOf": [ { "required": ["inner"] } ]
        });
        let out = normalize_gemini_schema(schema);
        assert!(out.get("allOf").is_none());
        assert!(out["properties"]["inner"].get("oneOf").is_none());
        assert_eq!(out["properties"]["inner"]["type"], "object");
    }

    #[test]
    fn normalize_still_collapses_nullable_anyof() {
        let schema = serde_json::json!({
            "description": "可空字符串",
            "anyOf": [ { "type": "string" }, { "type": "null" } ]
        });
        let out = normalize_gemini_schema(schema);
        assert_eq!(out["type"], "string");
        assert_eq!(out["description"], "可空字符串");
        assert!(out.get("anyOf").is_none());
    }

    #[test]
    fn normalize_keeps_typed_polymorphic_anyof() {
        // 分支带 type 的多态 anyOf：Vertex 支持，原样透传。
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "value": {
                    "anyOf": [ { "type": "string" }, { "type": "integer" } ]
                }
            }
        });
        let out = normalize_gemini_schema(schema);
        let branches = out["properties"]["value"]["anyOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0]["type"], "string");
        assert_eq!(branches[1]["type"], "integer");
    }

    #[test]
    fn normalize_strips_required_unless_type_object() {
        // Gemini: `required` is only legal on OBJECT. MCP schemas often park it
        // on string / untyped nodes (and inside typed anyOf branches).
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "required": ["query"]
                },
                "payload": {
                    "anyOf": [
                        {
                            "type": "object",
                            "properties": { "id": { "type": "string" } },
                            "required": ["id"]
                        },
                        { "type": "string", "required": ["id"] }
                    ]
                }
            },
            "required": ["query"]
        });
        let out = normalize_gemini_schema(schema);
        assert!(out["properties"]["query"].get("required").is_none());
        assert_eq!(out["required"], serde_json::json!(["query"]));
        let branches = out["properties"]["payload"]["anyOf"].as_array().unwrap();
        assert_eq!(branches[0]["required"], serde_json::json!(["id"]));
        assert!(branches[1].get("required").is_none());
    }

    #[test]
    fn normalize_infers_object_type_when_properties_present() {
        let schema = serde_json::json!({
            "properties": { "q": { "type": "string" } },
            "required": ["q"]
        });
        let out = normalize_gemini_schema(schema);
        assert_eq!(out["type"], "object");
        assert_eq!(out["required"], serde_json::json!(["q"]));
    }

    #[test]
    fn parses_gemini_response_text_and_inline_image() {
        for (mime_type, data) in [("image/png", "AAAA"), ("image/jpeg", "BBBB")] {
            // 非流式：text part + inlineData part → text 与 images 都解析出来。
            let response = serde_json::json!({
                "candidates": [{
                    "content": { "role": "model", "parts": [
                        { "text": "这是你的猫" },
                        { "inlineData": { "mimeType": mime_type, "data": data } }
                    ] },
                    "finishReason": "STOP"
                }]
            });
            let out = output_from_gemini_response(&response, "test").expect("output");
            assert_eq!(out.text, "这是你的猫");
            assert_eq!(out.images.len(), 1);
            assert_eq!(out.images[0].mime_type, mime_type);
            assert_eq!(out.images[0].data, data);
        }
    }
}
