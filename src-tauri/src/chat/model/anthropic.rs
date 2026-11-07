use super::{ModelTransport, ProviderConfig};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_ENCODING};
use serde_json::Value;

use crate::api::{send_model_with_failover, with_chat_request_timeout};
use crate::settings::CacheRetention;
use crate::usage::{
    chat_usage_source_for_label, error_kind_from_message, operation_from_label, UsageRecordInput,
};

use super::{
    FirstTokenStreamSink, GenerateOutput, GenerateRequest, LanguageModelProvider, ModelError,
    ModelFuture, ModelTool, ModelUsage, StreamSink,
};
#[cfg(test)]
use kivio_agent_sdk::model::anthropic::EXTENDED_CACHE_TTL_BETA;
pub use kivio_agent_sdk::model::anthropic::{
    anthropic_messages_from_generate_request, anthropic_tools_from_model_tools,
    output_from_anthropic_message,
};
use kivio_agent_sdk::model::anthropic::{
    AnthropicCacheTtl, AnthropicMessagesOptions, AnthropicThinkingMode, ANTHROPIC_VERSION,
    INTERLEAVED_THINKING_BETA,
};

/// Claude Code 客户端身份 beta，真实 Claude Code 每个请求都带。
const CLAUDE_CODE_BETA: &str = "claude-code-20250219";
/// Claude Code 2.x 内置工具名（规范大小写），对齐 pi 的 claudeCodeTools。
/// Claude Code 身份下，与之同名（大小写不敏感）的工具改用这个写法发出去，回包再映射回原名。
const CLAUDE_CODE_TOOL_NAMES: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

/// Kivio 工具名 → Claude Code 规范写法；不命中就原样返回。
fn to_claude_code_tool_name(name: &str) -> &str {
    CLAUDE_CODE_TOOL_NAMES
        .iter()
        .find(|cc| cc.eq_ignore_ascii_case(name))
        .copied()
        .unwrap_or(name)
}

/// Prefer exact declared names; case-insensitive recovery must be unambiguous.
fn from_claude_code_tool_name(name: &str, tools: &[ModelTool]) -> String {
    let mut candidate = None;
    let mut matches = 0;
    for tool in tools {
        let declared = &tool.name;
        if declared == name {
            return declared.clone();
        }
        if declared.eq_ignore_ascii_case(name) {
            matches += 1;
            candidate = Some(declared);
        }
    }
    if matches == 1 {
        candidate.expect("one matching declared tool").clone()
    } else {
        name.to_string()
    }
}

/// 改写最终请求体里的工具名：工具声明、历史 `tool_use`、`tool_choice`。
/// 只改 Kivio 自己的工具 —— `type` 字段非 custom 的服务端工具（web_search 等）名字是协议约定，不能动。
fn rename_tools_for_claude_code(body: &mut Value) {
    let rename = |slot: Option<&mut Value>| {
        if let Some(Value::String(name)) = slot {
            *name = to_claude_code_tool_name(name).to_string();
        }
    };
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools.iter_mut().filter(|tool| tool.get("type").is_none()) {
            rename(tool.get_mut("name"));
        }
    }
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
                continue;
            };
            for block in blocks
                .iter_mut()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            {
                rename(block.get_mut("name"));
            }
        }
    }
    if body["tool_choice"]["type"] == "tool" {
        rename(body.get_mut("tool_choice").and_then(|c| c.get_mut("name")));
    }
}

pub(super) struct AnthropicMessagesProvider<'a> {
    transport: ModelTransport<'a>,
    config: ProviderConfig<'a>,
}

impl<'a> AnthropicMessagesProvider<'a> {
    pub(super) fn new(transport: ModelTransport<'a>, config: ProviderConfig<'a>) -> Self {
        Self { transport, config }
    }
}

struct SingleRequest<'a, 'b>(&'a AnthropicMessagesProvider<'b>);
impl LanguageModelProvider for SingleRequest<'_, '_> {
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

impl LanguageModelProvider for AnthropicMessagesProvider<'_> {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            kivio_agent_sdk::model::anthropic::complete_anthropic_turn(
                &SingleRequest(self),
                request,
                None,
            )
            .await
        })
    }

    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            kivio_agent_sdk::model::anthropic::complete_anthropic_turn(
                &SingleRequest(self),
                request,
                Some(sink),
            )
            .await
        })
    }
}

impl AnthropicMessagesProvider<'_> {
    async fn generate_inner(&self, request: GenerateRequest) -> Result<GenerateOutput, ModelError> {
        crate::chat::video::reject_unimplemented_transport(&request, "Anthropic Messages")?;
        let label = request_label(&request, "Anthropic Messages API");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let body = self.request_body(&request, false);
        let response = send_model_with_failover(
            self.transport.runtime,
            &label,
            self.transport.retry_attempts,
            self.config.id,
            self.config.api_keys,
            |key| {
                with_chat_request_timeout(crate::api::attach_json_body(
                    self.with_extra_headers(
                        self.transport
                            .client
                            .post(self.messages_url())
                            .headers(anthropic_headers(key).unwrap_or_default())
                            .header(ACCEPT_ENCODING, "identity"),
                        &request,
                        false,
                    ),
                    &body,
                    self.config.compress_request_body,
                ))
                .send()
            },
        )
        .await
        .map_err(|err| {
            self.record_usage_failure(
                &request,
                &label,
                started_at,
                started.elapsed(),
                &err.message,
            );
            self.record_debug_failure(
                &request,
                &label,
                false,
                &err.message,
                started_at,
                started.elapsed(),
            );
            err
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
        let map_tool_name = |name: &str| self.declared_tool_name(name, &request);
        let output = kivio_agent_sdk::model::anthropic::decode_anthropic_body(
            &raw,
            &label,
            &request.model,
            Some(self.config.id),
            Some(&map_tool_name),
        )
        .map_err(|error| {
            self.record_usage_failure(
                &request,
                &label,
                started_at,
                started.elapsed(),
                &error.message,
            );
            self.record_debug_failure(
                &request,
                &label,
                false,
                &error.message,
                started_at,
                started.elapsed(),
            );
            error
        })?;
        self.record_usage_success(
            &request,
            &label,
            started_at,
            started.elapsed(),
            output.usage.clone(),
            None,
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
        crate::chat::video::reject_unimplemented_transport(&request, "Anthropic Messages")?;
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let mut measured_sink = FirstTokenStreamSink::new(sink, started);
        let sink = &mut measured_sink;
        let body = self.request_body(&request, true);
        let response = send_model_with_failover(
            self.transport.runtime,
            &label,
            self.transport.retry_attempts,
            self.config.id,
            self.config.api_keys,
            |key| {
                crate::api::attach_json_body(
                    self.with_extra_headers(
                        self.transport
                            .client
                            .post(self.messages_url())
                            .headers(anthropic_headers(key).unwrap_or_default())
                            .header(ACCEPT_ENCODING, "identity"),
                        &request,
                        true,
                    ),
                    &body,
                    self.config.compress_request_body,
                )
                .send()
            },
        )
        .await
        .map_err(|err| {
            self.record_usage_failure(
                &request,
                &label,
                started_at,
                started.elapsed(),
                &err.message,
            );
            self.record_debug_failure(
                &request,
                &label,
                true,
                &err.message,
                started_at,
                started.elapsed(),
            );
            err
        })?;

        let map_tool_name = |name: &str| self.declared_tool_name(name, &request);
        let output = kivio_agent_sdk::model::anthropic::decode_anthropic_stream(
            response,
            sink,
            &label,
            &request.model,
            Some(self.config.id),
            Some(&map_tool_name),
        )
        .await
        .map_err(|error| {
            self.record_usage_failure(
                &request,
                &label,
                started_at,
                started.elapsed(),
                &error.message,
            );
            self.record_debug_failure(
                &request,
                &label,
                true,
                &error.message,
                started_at,
                started.elapsed(),
            );
            error
        })?;
        self.record_usage_success(
            &request,
            &label,
            started_at,
            started.elapsed(),
            output.usage.clone(),
            sink.first_token_ms(),
        );
        self.record_debug_success(
            &request,
            &label,
            true,
            &output,
            started_at,
            started.elapsed(),
        );
        Ok(output)
    }

    fn messages_url(&self) -> String {
        anthropic_messages_url(self.config.base_url)
    }

    fn wire_request<'a>(
        &self,
        request: &'a GenerateRequest,
    ) -> (
        std::borrow::Cow<'a, GenerateRequest>,
        AnthropicMessagesOptions,
    ) {
        use crate::chat::model_metadata::ClaudeThinkingKind;
        let profile = crate::chat::model_metadata::claude_thinking_profile(&request.model);
        let mut resolved = std::borrow::Cow::Borrowed(request);
        if let (Some(profile), Some(level)) = (profile, request.options.thinking_level.as_deref()) {
            let clamped = profile.clamp_effort(level);
            if clamped != level {
                resolved.to_mut().options.thinking_level = Some(clamped.to_string());
            }
        }
        let temperature = if profile.is_some_and(|profile| profile.forbid_temperature) {
            None
        } else {
            crate::chat::model_metadata::temperature_for_request(
                request.options.temperature,
                Some(self.config.source),
                &request.model,
            )
        };
        let thinking_mode = match profile.map(|profile| profile.kind) {
            Some(ClaudeThinkingKind::Extended) => AnthropicThinkingMode::Extended {
                output_effort: profile.is_some_and(|profile| profile.supports_output_effort),
            },
            Some(ClaudeThinkingKind::Unsupported) => AnthropicThinkingMode::Unsupported,
            _ => AnthropicThinkingMode::Adaptive,
        };
        let options = AnthropicMessagesOptions {
            temperature,
            default_max_tokens: crate::settings::default_chat_max_output_tokens(),
            thinking_mode,
            off_thinking_type: profile
                .and_then(|profile| profile.off_thinking_type)
                .map(str::to_string),
            allow_unsigned_thinking: crate::utils::is_official_deepseek_anthropic_api(
                self.config.base_url,
            ),
            provider_id: Some(self.config.id.to_string()),
            system_prefix: self
                .config
                .claude_code
                .then(|| crate::provider_request::CLAUDE_CODE_SYSTEM_PREFIX.to_string()),
            cache: self.prompt_cache_control(&request.metadata),
            extra_body: serde_json::json!({ "metadata": { "user_id": self.user_id(&request.metadata) } }),
        };
        (resolved, options)
    }

    fn request_body(&self, request: &GenerateRequest, stream: bool) -> Value {
        let (request, options) = self.wire_request(request);
        let mut body = kivio_agent_sdk::model::anthropic::request_body(&request, stream, &options);
        if self.config.claude_code {
            rename_tools_for_claude_code(&mut body);
        }
        body
    }

    /// 该供应商本次是否要打 prompt 缓存断点。
    ///
    /// 除了开关，还要求本次请求属于某个**会话**（有 `conversation_id`）。翻译器 / 截图翻译 /
    /// Lens / 上下文压缩 / 标题总结走的是同一个生成入口但都是一次性调用，给它们打断点等于
    /// 按 1.25× 写一份下次内容全变、永远读不到的缓存 —— 纯加价。OpenAI 侧靠「没有会话 id 就
    /// 不发 prompt_cache_key」天然排除了这些路径，这里对齐同一条门。
    fn prompt_cache_control(
        &self,
        metadata: &crate::chat::model::RequestMetadata,
    ) -> Option<AnthropicCacheTtl> {
        if self.config.cache_retention == CacheRetention::None {
            return None;
        }
        metadata
            .conversation_id
            .as_deref()
            .filter(|id| !id.is_empty())?;
        // 对齐 pi：long → ttl:1h（默认 supportsLongCacheRetention=true，不做主机名启发式）。
        let long_ttl = matches!(self.config.cache_retention, CacheRetention::Long);
        Some(if long_ttl {
            AnthropicCacheTtl::OneHour
        } else {
            AnthropicCacheTtl::Default
        })
    }

    /// `metadata.user_id`：Claude Code 身份下是它自己的 JSON 形状（设备 + 会话），
    /// 否则是本机稳定的匿名设备 id —— 官方建议的「不透明的终端用户标识」。
    fn user_id(&self, metadata: &crate::chat::model::RequestMetadata) -> String {
        let device_id = crate::provider_request::device_id();
        if !self.config.claude_code {
            return device_id.to_string();
        }
        serde_json::json!({
            "device_id": device_id,
            "session_id": crate::provider_request::session_uuid(
                metadata.conversation_id.as_deref()
            ),
        })
        .to_string()
    }

    /// 回包里的工具名映射回本次请求声明的原名（只有 Claude Code 身份会改写工具名）。
    fn declared_tool_name(&self, name: &str, request: &GenerateRequest) -> String {
        if self.config.claude_code {
            from_claude_code_tool_name(name, &request.tools)
        } else {
            name.to_string()
        }
    }

    /// 本次请求按功能需要声明的 beta。
    fn feature_betas(&self, request: &GenerateRequest) -> Vec<&'static str> {
        let (request, options) = self.wire_request(request);
        let mut betas = Vec::new();
        if self.config.claude_code {
            betas.extend([CLAUDE_CODE_BETA, INTERLEAVED_THINKING_BETA]);
        }
        for beta in kivio_agent_sdk::model::anthropic::feature_betas(&request, &options) {
            if !betas.contains(&beta) {
                betas.push(beta);
            }
        }
        betas
    }

    /// 供应商「请求配置」带来的附加头（CLI 身份 / 自定义头）+ 按功能声明的 beta 头。
    /// 发送路径与请求调试面板共用，杜绝「面板显示的和实际发的不一致」。
    fn extra_header_pairs(&self, request: &GenerateRequest, stream: bool) -> Vec<(String, String)> {
        // Anthropic 不发会话亲和头：官方与主流中转都不认，身份模式下有 X-Claude-Code-Session-Id。
        let mut pairs = crate::provider_request::model_header_pairs(
            self.config.source,
            request.metadata.conversation_id.as_deref(),
            false,
            stream,
        );
        // anthropic-beta 不是保留头（用户可能要开别的 beta），所以这里得跟用户填的那条合并成
        // 一行、去重 —— 发两行的话调试面板（BTreeMap）只显示一条，就和实际发出去的对不上了。
        let betas = self.feature_betas(request);
        if !betas.is_empty() {
            let mut merged: Vec<String> = pairs
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("anthropic-beta"))
                .map(|(_, value)| {
                    value
                        .split(',')
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            for beta in betas {
                if !merged.iter().any(|v| v == beta) {
                    merged.push(beta.to_string());
                }
            }
            crate::provider_request::upsert_pair(
                &mut pairs,
                "anthropic-beta".to_string(),
                merged.join(","),
            );
        }
        pairs
    }

    /// 把 `extra_header_pairs` 贴到请求上。
    fn with_extra_headers(
        &self,
        builder: reqwest::RequestBuilder,
        request: &GenerateRequest,
        stream: bool,
    ) -> reqwest::RequestBuilder {
        let mut builder = builder;
        for (name, value) in self.extra_header_pairs(request, stream) {
            builder = builder.header(name, value);
        }
        builder
    }

    /// 重建本次请求实际会带的 headers（脱敏后）供请求调试面板展示。镜像 `anthropic_headers`
    /// （x-api-key / anthropic-version / content-type）+ 发送路径另加的 Accept-Encoding
    /// 与 `extra_header_pairs`。x-api-key 用首个 key（正常发送用的也是它）派生脱敏预览。
    fn debug_request_headers(
        &self,
        request: &GenerateRequest,
        stream: bool,
    ) -> std::collections::BTreeMap<String, String> {
        let mut headers = std::collections::BTreeMap::new();
        if let Some(key) = self.config.preferred_api_key {
            headers.insert("x-api-key".to_string(), key.to_string());
        }
        headers.insert(
            "anthropic-version".to_string(),
            ANTHROPIC_VERSION.to_string(),
        );
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("Accept-Encoding".to_string(), "identity".to_string());
        for (name, value) in self.extra_header_pairs(request, stream) {
            headers.insert(name, value);
        }
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
        let Some(debug) = (self.transport.request_debug)() else {
            return;
        };
        let record = crate::chat::request_debug::build_debug_record(
            crate::chat::request_debug::DebugRecordArgs {
                provider: self.config.source,
                request,
                label,
                started_at,
                duration_ms: duration.as_millis() as u64,
                status: "success",
                url: self.messages_url(),
                headers: self.debug_request_headers(request, stream),
                body: self.request_body(request, stream),
                stream,
                response: crate::chat::request_debug::RequestDebugResponse::from_output(
                    output,
                    Some(200),
                ),
            },
        );
        debug.record(record);
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
        let Some(debug) = (self.transport.request_debug)() else {
            return;
        };
        let record = crate::chat::request_debug::build_debug_record(
            crate::chat::request_debug::DebugRecordArgs {
                provider: self.config.source,
                request,
                label,
                started_at,
                duration_ms: duration.as_millis() as u64,
                status: "error",
                url: self.messages_url(),
                headers: self.debug_request_headers(request, stream),
                body: self.request_body(request, stream),
                stream,
                response: crate::chat::request_debug::RequestDebugResponse::from_error(
                    error,
                    crate::api::extract_status_code(error),
                ),
            },
        );
        debug.record(record);
    }

    fn record_usage_success(
        &self,
        request: &GenerateRequest,
        label: &str,
        started_at: i64,
        duration: std::time::Duration,
        usage: Option<ModelUsage>,
        first_token_ms: Option<u64>,
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
        (self.transport.record_usage)(UsageRecordInput {
            provider: self.config.source,
            model: &request.model,
            source: &source,
            operation: &operation,
            status: "success",
            status_code: Some(200),
            usage,
            usage_source: "provider_reported",
            started_at,
            duration_ms: duration.as_millis() as u64,
            first_token_ms,
            reasoning_effort: crate::usage::reasoning_effort_for_request(request),
            conversation_id: request.metadata.conversation_id.clone(),
            message_id: request.metadata.message_id.clone(),
            error_kind: None,
        });
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
        (self.transport.record_usage)(UsageRecordInput {
            provider: self.config.source,
            model: &request.model,
            source: &source,
            operation: &operation,
            status: crate::usage::failure_status_from_message(error),
            status_code: crate::api::extract_status_code(error),
            usage: None,
            usage_source: "missing",
            started_at,
            duration_ms: duration.as_millis() as u64,
            first_token_ms: None,
            reasoning_effort: crate::usage::reasoning_effort_for_request(request),
            conversation_id: request.metadata.conversation_id.clone(),
            message_id: request.metadata.message_id.clone(),
            error_kind: Some(error_kind_from_message(error)),
        });
    }
}

fn anthropic_headers(api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    if !api_key.trim().is_empty() {
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(api_key).map_err(|err| format!("Invalid API key: {err}"))?,
        );
    }
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
    use crate::chat::model::{GenerateOptions, MessagePart, ModelMessage, ModelRole};
    use crate::settings::ModelInfo;

    #[test]
    fn end_to_end_openai_dangling_history_becomes_paired_wire() {
        // Real entry point: an OpenAI-format history where an assistant tool_call
        // was never answered (interrupted turn), followed by the next user prompt.
        use crate::chat::model::generate_request_from_openai_messages;
        let openai = vec![
            serde_json::json!({ "role": "user", "content": "hi" }),
            serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "functions.x:0",
                    "type": "function",
                    "function": { "name": "read", "arguments": "{}" }
                }, {"id":"toolu_kivio_66756e6374696f6e732e783a30", "type":"function", "function":{"name":"read", "arguments":"{}"}}]
            }),
            serde_json::json!({ "role": "user", "content": "never mind, do this instead" }),
        ];
        let request = generate_request_from_openai_messages(
            "claude-opus-4-8",
            openai,
            None,
            Default::default(),
            "test",
            Default::default(),
        );
        let wire = anthropic_messages_from_generate_request(&request);
        let mapped = wire[1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["type"] == "tool_use")
            .unwrap()["id"]
            .as_str()
            .unwrap();
        assert!(mapped
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'));
        assert_ne!(
            mapped, "toolu_kivio_66756e6374696f6e732e783a30",
            "encoded foreign id must not collide with an existing valid id"
        );
        assert!(wire
            .iter()
            .flat_map(|m| m["content"].as_array().unwrap())
            .all(|b| b["type"] != "text" || !b["text"].as_str().unwrap().trim().is_empty()));

        // Every tool_use id must have a tool_result in the immediately following turn.
        for (i, msg) in wire.iter().enumerate() {
            let Some(blocks) = msg["content"].as_array() else {
                continue;
            };
            for block in blocks {
                if block["type"] == "tool_use" {
                    let id = block["id"].as_str().unwrap();
                    let next = &wire[i + 1]["content"];
                    let paired = next
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b["type"] == "tool_result" && b["tool_use_id"] == id);
                    assert!(paired, "tool_use {id} must be paired in the next turn");
                }
            }
        }
    }

    /// Build a real Anthropic request body via the production `request_body`
    /// path and assert how `thinking_level` maps to the wire.
    fn build_anthropic_body(
        thinking_level: Option<&str>,
        provider_temperature: Option<f64>,
        request_temperature: Option<f64>,
    ) -> Value {
        build_anthropic_body_for(
            "claude-opus-4-8",
            thinking_level,
            provider_temperature,
            request_temperature,
        )
    }

    fn build_anthropic_body_for(
        model: &str,
        thinking_level: Option<&str>,
        provider_temperature: Option<f64>,
        request_temperature: Option<f64>,
    ) -> Value {
        build_anthropic_body_with(
            model,
            thinking_level,
            thinking_level.is_some(),
            provider_temperature,
            request_temperature,
        )
    }

    fn build_anthropic_body_with(
        model: &str,
        thinking_level: Option<&str>,
        thinking_enabled: bool,
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
                model.to_string(),
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
            available_models: vec![model.to_string()],
            enabled_models: vec![model.to_string()],
            enabled: true,
            api_format: "anthropic_messages".into(),
            model_overrides,
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let request = GenerateRequest {
            model: model.into(),
            system: "sys".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions {
                temperature: request_temperature,
                thinking_level: thinking_level.map(|s| s.to_string()),
                thinking_enabled,
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

        // 4.6+：adaptive + output_config.effort + display summarized（4.7+ 默认 omitted）。
        let high = build_anthropic_body(Some("high"), None, None);
        eprintln!("[anthropic effort=high] {high}");
        assert_eq!(high["thinking"]["type"], "adaptive");
        assert_eq!(high["thinking"]["display"], "summarized");
        assert_eq!(high["output_config"]["effort"], "high");
        assert!(
            high["thinking"].get("budget_tokens").is_none(),
            "must not send budget_tokens"
        );
    }

    #[test]
    fn opus_4_5_uses_extended_thinking_with_effort() {
        // https://platform.claude.com/docs/en/build-with-claude/extended-thinking
        // Opus 4.5：enabled + budget_tokens；官方还要求同时设 effort。
        let body = build_anthropic_body_for("claude-opus-4-5-20251101", Some("high"), None, None);
        assert_eq!(body["thinking"]["type"], "enabled", "body: {body}");
        assert_eq!(body["thinking"]["display"], "summarized");
        assert_eq!(body["thinking"]["budget_tokens"], 7168); // 16000 clamped under max_tokens=8192
        assert_eq!(body["output_config"]["effort"], "high");

        let alias = build_anthropic_body_for("opus-4.5", Some("low"), None, None);
        assert_eq!(alias["thinking"]["type"], "enabled");
        assert_eq!(alias["thinking"]["budget_tokens"], 1024);
        assert_eq!(alias["output_config"]["effort"], "low");
    }

    #[test]
    fn sonnet_4_5_uses_extended_thinking_without_effort() {
        // Sonnet/Haiku 4.5 不在官方 effort 支持名单，只能 budget_tokens。
        let body = build_anthropic_body_for("claude-sonnet-4.5", Some("medium"), None, None);
        assert_eq!(body["thinking"]["type"], "enabled", "body: {body}");
        assert_eq!(body["thinking"]["display"], "summarized");
        assert_eq!(body["thinking"]["budget_tokens"], 7168); // 10000 clamped under 8192
        assert!(
            body.get("output_config").is_none(),
            "sonnet 4.5 must not send output_config.effort: {body}"
        );
    }

    #[test]
    fn thinking_enabled_strips_temperature() {
        let body = build_anthropic_body_for("claude-opus-4-5", Some("high"), Some(0.4), None);
        assert!(body.get("temperature").is_none(), "body: {body}");
        assert_eq!(body["thinking"]["type"], "enabled");
    }

    #[test]
    fn request_body_temperature_is_optional_and_model_scoped() {
        // 4.6 关思考时仍允许 temperature；4.7+ 一律不能带。
        let default_body = build_anthropic_body_for("claude-opus-4-6", None, None, None);
        assert!(default_body.get("temperature").is_none());

        let configured_body = build_anthropic_body_for("claude-opus-4-6", None, Some(0.4), None);
        assert_eq!(configured_body["temperature"], serde_json::json!(0.4));

        let explicit_body = build_anthropic_body_for("claude-opus-4-6", None, Some(0.4), Some(1.2));
        assert_eq!(explicit_body["temperature"], serde_json::json!(1.2));
    }

    #[test]
    fn newer_models_omit_temperature_even_without_thinking() {
        let body = build_anthropic_body_for("claude-opus-4-8", None, Some(0.4), None);
        assert!(body.get("temperature").is_none(), "body: {body}");
    }

    #[test]
    fn sonnet_5_off_sends_disabled() {
        let body = build_anthropic_body_with("claude-sonnet-5", None, false, None, None);
        assert_eq!(body["thinking"]["type"], "disabled", "body: {body}");
    }

    #[test]
    fn haiku_55_uses_adaptive_thinking_and_can_disable_it() {
        let on = build_anthropic_body_for("claude-haiku-5.5", Some("high"), None, None);
        assert_eq!(on["thinking"]["type"], "adaptive", "body: {on}");
        assert_eq!(on["output_config"]["effort"], "high");
        let off = build_anthropic_body_with("claude-haiku-5.5", None, false, None, None);
        assert_eq!(off["thinking"]["type"], "disabled", "body: {off}");
        assert!(off.get("output_config").is_none());
    }

    #[test]
    fn always_on_models_omit_thinking_when_off() {
        let body = build_anthropic_body_with("claude-fable-5", None, false, None, None);
        assert!(body.get("thinking").is_none(), "body: {body}");
    }

    #[test]
    fn opus_55_keeps_always_on_thinking_and_supported_efforts() {
        for model in ["claude-opus-5-5", "anthropic/claude-opus-5.5"] {
            let off = build_anthropic_body_with(model, None, false, Some(0.4), None);
            assert!(off.get("thinking").is_none(), "body: {off}");
            assert!(off.get("temperature").is_none(), "body: {off}");
            for level in ["low", "medium", "high", "xhigh", "max"] {
                let body = build_anthropic_body_for(model, Some(level), None, None);
                assert_eq!(body["thinking"]["type"], "adaptive");
                assert_eq!(body["output_config"]["effort"], level);
            }
        }
        let opus5 = build_anthropic_body_with("claude-opus-5", None, false, None, None);
        assert_eq!(opus5["thinking"]["type"], "disabled");
    }

    #[test]
    fn sonnet_55_off_uses_between_tools_and_on_uses_adaptive() {
        for model in ["claude-sonnet-5-5", "anthropic/claude-sonnet-5.5"] {
            let off = build_anthropic_body_with(model, Some("max"), false, Some(0.4), None);
            assert_eq!(
                off["thinking"],
                serde_json::json!({ "type": "between_tools" })
            );
            assert!(off.get("output_config").is_none(), "body: {off}");
            assert!(off.get("temperature").is_none(), "body: {off}");
            for level in ["low", "medium", "high", "xhigh", "max"] {
                let body = build_anthropic_body_for(model, Some(level), None, None);
                assert_eq!(body["thinking"]["type"], "adaptive");
                assert_eq!(body["output_config"]["effort"], level);
            }
        }
        let sonnet5 = build_anthropic_body_with("claude-sonnet-5", None, false, None, None);
        assert_eq!(sonnet5["thinking"]["type"], "disabled");
    }

    #[test]
    fn claude_3_does_not_send_thinking() {
        for model in [
            "claude-3-5-sonnet-20241022",
            "claude-3-haiku-20240307",
            "claude-3.5-haiku",
        ] {
            let body = build_anthropic_body_for(model, Some("high"), None, None);
            assert!(
                body.get("thinking").is_none(),
                "{model} must not send thinking: {body}"
            );
        }
    }

    #[test]
    fn stale_xhigh_clamps_to_supported_effort() {
        let body = build_anthropic_body_for("claude-opus-4-6", Some("xhigh"), None, None);
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "max");
    }

    #[test]
    fn opus_4_uses_extended_thinking_without_effort() {
        let body = build_anthropic_body_for("claude-opus-4", Some("high"), None, None);
        assert_eq!(body["thinking"]["type"], "enabled", "body: {body}");
        assert!(
            body.get("output_config").is_none(),
            "opus 4 must not send output_config.effort: {body}"
        );
    }

    #[test]
    fn builtin_web_search_appends_server_tool() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
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
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let base = GenerateRequest {
            model: "claude-opus-4-8".into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        };
        // 默认（off）：无客户端工具且未开内置 ⇒ 不发 tools。
        let off = adapter.request_body(&base, false);
        assert!(off.get("tools").is_none(), "body: {off}");
        // 开内置 ⇒ tools 含 web_search_20250305 服务端工具。
        let mut req = base.clone();
        req.options.builtin_web_search = true;
        let on = adapter.request_body(&req, false);
        let tools = on["tools"].as_array().expect("tools array present");
        assert!(
            tools
                .iter()
                .any(|t| t["type"] == "web_search_20250305" && t["name"] == "web_search"),
            "body: {on}"
        );
    }

    #[test]
    fn prompt_caching_off_leaves_body_untouched() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let provider = cache_test_provider(false, "short");
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let body = adapter.request_body(&cache_test_request(), false);
        // 关缓存时 system 仍是块数组，但没有任何 cache_control。
        assert_eq!(
            body["system"],
            serde_json::json!([{ "type": "text", "text": "you are kivio" }])
        );
        assert!(!body.to_string().contains("cache_control"), "body: {body}");
    }

    #[test]
    fn prompt_caching_marks_tools_system_and_last_message() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let provider = cache_test_provider(true, "short");
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let body = adapter.request_body(&cache_test_request(), false);

        let ephemeral = serde_json::json!({ "type": "ephemeral" });
        // system 从字符串变成块数组，最后一块带断点。
        assert_eq!(
            body["system"][0]["text"],
            serde_json::json!("you are kivio")
        );
        assert_eq!(body["system"][0]["cache_control"], ephemeral);
        // 只有最后一个工具带断点（前缀越长命中越多，中间打断点是浪费）。
        let tools = body["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 2);
        assert!(tools[0].get("cache_control").is_none());
        assert_eq!(tools[1]["cache_control"], ephemeral);
        // 最后一条消息的最后一个内容块带断点。
        let messages = body["messages"].as_array().expect("messages");
        let last = messages.last().expect("last message");
        let content = last["content"].as_array().expect("content blocks");
        assert_eq!(content.last().unwrap()["cache_control"], ephemeral);
        // 断点总数 ≤ Anthropic 的 4 个上限。
        assert_eq!(body.to_string().matches("cache_control").count(), 3);
    }

    #[test]
    fn long_retention_adds_ttl_and_beta_header() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let provider = cache_test_provider(true, "long");
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let body = adapter.request_body(&cache_test_request(), false);
        assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
        // 1h 不带 beta 头会被拒，所以头和 ttl 必须同进同出。
        let in_session = cache_test_request();
        let headers = adapter.debug_request_headers(&in_session, false);
        assert_eq!(
            headers.get("anthropic-beta").map(String::as_str),
            Some(EXTENDED_CACHE_TTL_BETA)
        );

        let short_provider = cache_test_provider(true, "short");
        let short_adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &short_provider),
            ProviderConfig::from_settings(&short_provider),
        );
        assert!(short_adapter
            .debug_request_headers(&in_session, false)
            .get("anthropic-beta")
            .is_none());
        // 一次性调用（无会话 id）连断点都不打，自然也不该带 beta 头。
        assert!(adapter
            .debug_request_headers(
                &GenerateRequest {
                    metadata: Default::default(),
                    ..cache_test_request()
                },
                false,
            )
            .get("anthropic-beta")
            .is_none());
    }

    #[test]
    fn long_retention_on_any_host_sends_ttl() {
        // 对齐 pi：long 默认 supportsLongCacheRetention=true，不做主机名启发式。
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut provider = cache_test_provider(true, "long");
        provider.base_url = "https://api.deepseek.com/anthropic".into();
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let body = adapter.request_body(&cache_test_request(), false);
        assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
        let in_session = cache_test_request();
        assert_eq!(
            adapter
                .debug_request_headers(&in_session, false)
                .get("anthropic-beta")
                .map(String::as_str),
            Some(EXTENDED_CACHE_TTL_BETA)
        );
    }

    #[test]
    fn one_shot_calls_get_no_breakpoints_even_with_caching_on() {
        // 翻译器 / 截图翻译 / Lens / 上下文压缩 / 标题总结走同一个生成入口，但都没有
        // conversation_id。给它们打断点 = 按 1.25× 写一份下次内容全变、永远读不到的缓存。
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let provider = cache_test_provider(true, "short");
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let mut request = cache_test_request();
        request.metadata.conversation_id = None;
        let body = adapter.request_body(&request, false);
        assert_eq!(
            body["system"],
            serde_json::json!([{ "type": "text", "text": "you are kivio" }])
        );
        assert!(!body.to_string().contains("cache_control"), "body: {body}");
        // 空串也算没有会话。
        request.metadata.conversation_id = Some(String::new());
        assert!(!adapter
            .request_body(&request, false)
            .to_string()
            .contains("cache_control"));
    }

    #[test]
    fn anthropic_and_openai_default_to_short() {
        // 统一 short 默认：OpenAI / Anthropic 均启用客户端缓存字段。
        let mut p = cache_test_provider(true, "short");
        p.request.prompt_caching = None;
        p.request.prompt_cache_retention = "short".into();
        assert!(p.prompt_caching_enabled());
        p.api_format = "openai_chat".into();
        assert!(p.prompt_caching_enabled());
        p.request.prompt_cache_retention = "none".into();
        assert!(!p.prompt_caching_enabled());
    }

    fn cache_test_provider(caching: bool, retention: &str) -> crate::settings::ModelProvider {
        crate::settings::ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://api.anthropic.com".into(),
            available_models: vec!["claude-opus-4-8".into()],
            enabled_models: vec!["claude-opus-4-8".into()],
            enabled: true,
            api_format: "anthropic_messages".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: crate::settings::ProviderRequestConfig {
                prompt_cache_retention: if caching {
                    retention.into()
                } else {
                    "none".into()
                },
                ..Default::default()
            },
            active_key_index: 0,
        }
    }

    fn cache_test_tool(name: &str) -> ModelTool {
        ModelTool {
            name: name.into(),
            description: format!("{name} a file"),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        }
    }

    fn cache_test_request() -> GenerateRequest {
        GenerateRequest {
            model: "claude-opus-4-8".into(),
            system: "you are kivio".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: vec![cache_test_tool("read"), cache_test_tool("write")],
            options: GenerateOptions::default(),
            // 有会话 id 才会打断点（一次性调用不该付这份钱），见 prompt_cache_control。
            metadata: crate::chat::model::RequestMetadata {
                conversation_id: Some("conv_abc".into()),
                ..Default::default()
            },
        }
    }

    fn identity_adapter_request(
        cli_identity: &str,
    ) -> (Value, std::collections::BTreeMap<String, String>) {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut provider = cache_test_provider(false, "short");
        provider.request.cli_identity = cli_identity.to_string();
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let request = cache_test_request();
        (
            adapter.request_body(&request, false),
            adapter.debug_request_headers(&request, false),
        )
    }

    #[test]
    fn plain_request_is_honest_and_stable() {
        let (body, headers) = identity_adapter_request("");
        // 不伪装：不带 Claude Code 开场、不带 claude-code beta，UA 如实报 Kivio。
        assert_eq!(body["system"].as_array().map(Vec::len), Some(1));
        assert!(!body.to_string().contains("Claude Code"), "body: {body}");
        assert!(headers["User-Agent"].starts_with("Kivio/"));
        assert!(headers.get("anthropic-beta").is_none());
        // user_id 是稳定的匿名设备 id，两次请求一致。
        let user_id = body["metadata"]["user_id"].as_str().expect("user_id");
        assert_eq!(user_id, crate::provider_request::device_id());
        assert_eq!(identity_adapter_request("").0["metadata"], body["metadata"]);
    }

    #[test]
    fn claude_code_identity_aligns_body_with_headers() {
        let (body, headers) = identity_adapter_request("claude_code");
        let system = body["system"].as_array().expect("system blocks");
        assert_eq!(
            system[0]["text"],
            crate::provider_request::CLAUDE_CODE_SYSTEM_PREFIX
        );
        assert_eq!(system[1]["text"], "you are kivio");
        let user_id: Value =
            serde_json::from_str(body["metadata"]["user_id"].as_str().expect("user_id"))
                .expect("json user_id");
        assert_eq!(user_id["device_id"], crate::provider_request::device_id());
        // 会话 id 与头里的 X-Claude-Code-Session-Id 是同一个。
        assert_eq!(user_id["session_id"], headers["X-Claude-Code-Session-Id"]);
        let betas = &headers["anthropic-beta"];
        assert!(betas.contains(CLAUDE_CODE_BETA), "{betas}");
        assert!(betas.contains(INTERLEAVED_THINKING_BETA), "{betas}");
        assert!(headers["User-Agent"].starts_with("claude-cli/"));
    }

    #[test]
    fn claude_code_identity_renames_tools_and_maps_them_back() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut provider = cache_test_provider(false, "short");
        provider.request.cli_identity = "claude_code".into();
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let mut request = cache_test_request();
        request.tools.push(cache_test_tool("todo_write"));
        request.options.builtin_web_search = true;
        request.messages.push(ModelMessage {
            role: ModelRole::Assistant,
            content: vec![MessagePart::ToolCall {
                id: "toolu_1".into(),
                name: "read".into(),
                arguments: serde_json::json!({}),
                arguments_raw: "{}".into(),
                signature: None,
            }],
        });
        let body = adapter.request_body(&request, false);
        let names: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // read/write 命中 Claude Code 内置名；todo_write 对不上（下划线）原样；服务端 web_search 不动。
        assert_eq!(names, ["Read", "Write", "todo_write", "web_search"]);
        assert!(body.to_string().contains(r#""name":"Read""#));
        assert!(!body.to_string().contains(r#""name":"read""#), "{body}");
        // 回包映射回本次声明的原名；未声明的名字原样。
        assert_eq!(adapter.declared_tool_name("Read", &request), "read");
        assert_eq!(adapter.declared_tool_name("Bash", &request), "Bash");
        // 非流式：存进历史的 provider message 也必须是原名，不只是 tool_calls。
        let response = serde_json::json!({
            "content": [{ "type": "tool_use", "id": "toolu_2", "name": "Read", "input": {} }],
            "stop_reason": "tool_use",
        });
        let map_tool_name = |name: &str| adapter.declared_tool_name(name, &request);
        let output = kivio_agent_sdk::model::anthropic::decode_anthropic_body(
            &response.to_string(),
            "test",
            &request.model,
            Some(adapter.config.id),
            Some(&map_tool_name),
        )
        .expect("output");
        assert_eq!(output.tool_calls[0].function_name, "read");
        assert_eq!(
            crate::chat::model::openai_messages_from_model_messages(&[output.replay_message()])[0]
                ["tool_calls"][0]["function"]["name"],
            "read"
        );

        // 不选身份时完全不改写。
        let plain_provider = cache_test_provider(false, "short");
        let plain = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &plain_provider),
            ProviderConfig::from_settings(&plain_provider),
        );
        assert_eq!(
            plain.request_body(&request, false)["tools"][0]["name"],
            "read"
        );
        assert_eq!(plain.declared_tool_name("Read", &request), "Read");
    }

    #[test]
    fn claude_code_tool_mapping_preserves_case_distinct_mcp_calls() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut provider = cache_test_provider(false, "short");
        provider.request.cli_identity = "claude_code".into();
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let mut request = cache_test_request();
        request.tools = ["read", "Read"]
            .into_iter()
            .map(|name| {
                let mut tool = cache_test_tool(name);
                tool.name = format!("mcp__server__{name}");
                tool
            })
            .collect();
        let response = serde_json::json!({
            "content": [
                { "type": "tool_use", "id": "lower", "name": "mcp__server__read", "input": {} },
                { "type": "tool_use", "id": "upper", "name": "mcp__server__Read", "input": {} },
                { "type": "tool_use", "id": "ambiguous", "name": "mcp__server__READ", "input": {} },
            ],
            "stop_reason": "tool_use",
        });
        let map_tool_name = |name: &str| adapter.declared_tool_name(name, &request);
        let output = kivio_agent_sdk::model::anthropic::decode_anthropic_body(
            &response.clone().to_string(),
            "test",
            &request.model,
            Some(adapter.config.id),
            Some(&map_tool_name),
        )
        .expect("tool calls");
        let names: Vec<&str> = output
            .tool_calls
            .iter()
            .map(|call| call.function_name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "mcp__server__read",
                "mcp__server__Read",
                "mcp__server__READ"
            ]
        );
        assert_eq!(
            crate::chat::model::openai_messages_from_model_messages(&[output.replay_message()])[0]
                ["tool_calls"][1]["function"]["name"],
            "mcp__server__Read"
        );

        // Removing the case-distinct peer makes recovery unambiguous.
        request.tools.pop();
        let map_tool_name = |name: &str| adapter.declared_tool_name(name, &request);
        let output = kivio_agent_sdk::model::anthropic::decode_anthropic_body(
            &response.to_string(),
            "test",
            &request.model,
            Some(adapter.config.id),
            Some(&map_tool_name),
        )
        .expect("unique tool calls");
        assert_eq!(output.tool_calls[2].function_name, "mcp__server__read");
    }

    #[test]
    fn accept_header_defaults_but_user_value_wins() {
        let (_, headers) = identity_adapter_request("");
        assert_eq!(headers["Accept"], "application/json");
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut provider = cache_test_provider(false, "short");
        provider.request.custom_headers = vec![crate::settings::ProviderCustomHeader {
            key: "accept".into(),
            value: "text/event-stream".into(),
        }];
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let pairs = adapter.extra_header_pairs(&cache_test_request(), false);
        let accepts: Vec<_> = pairs
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("accept"))
            .collect();
        assert_eq!(accepts.len(), 1);
        assert_eq!(accepts[0].1, "text/event-stream");
    }

    #[test]
    fn manual_thinking_declares_interleaved_beta_and_merges_user_betas() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut provider = cache_test_provider(true, "long");
        provider.request.custom_headers = vec![crate::settings::ProviderCustomHeader {
            key: "anthropic-beta".into(),
            value: "foo-1, extended-cache-ttl-2025-04-11".into(),
        }];
        let adapter = AnthropicMessagesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let mut request = cache_test_request();
        request.model = "claude-sonnet-4-5".into();
        request.options.thinking_enabled = true;
        request.options.thinking_level = Some("high".into());
        assert_eq!(
            adapter.request_body(&request, false)["thinking"]["type"],
            "enabled"
        );
        // 用户的 beta 在前、去重后合并成一行。
        assert_eq!(
            adapter.debug_request_headers(&request, false)["anthropic-beta"],
            format!("foo-1,{EXTENDED_CACHE_TTL_BETA},{INTERLEAVED_THINKING_BETA}")
        );
        // adaptive 思考自带交错，不加这个 beta。
        request.model = "claude-opus-4-8".into();
        assert!(
            !adapter.debug_request_headers(&request, false)["anthropic-beta"]
                .contains(INTERLEAVED_THINKING_BETA)
        );
    }
}
