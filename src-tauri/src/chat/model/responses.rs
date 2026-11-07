//! OpenAI **Responses API** adapter (`POST /v1/responses`).
//!
//! Peer to `openai.rs` (Chat Completions) and `anthropic.rs` (Messages). Codex /
//! Responses-native models (and proxies wrapping them) often emit tool-call arguments
//! ONLY over this protocol's streaming events (`response.function_call_arguments.*`) —
//! on Chat Completions they return empty `arguments`. This adapter speaks the Responses
//! wire format while presenting the same provider-agnostic `LanguageModelProvider`
//! surface, so the agent loop is unchanged.
//!
//! Conversation state is modelled as a flat `input` item list rather than chat messages:
//! tool calls are `function_call` items and tool results are `function_call_output`
//! items (see `responses_input_from_model_messages` in `types.rs`).

use super::{ModelTransport, ProviderConfig};
use reqwest::header::ACCEPT_ENCODING;
use serde_json::Value;

use crate::api::{send_model_with_failover, with_chat_request_timeout};
use crate::settings::{CacheRetention, ProviderApiFormat};
use crate::usage::{
    chat_usage_source_for_label, error_kind_from_message, operation_from_label, UsageRecordInput,
};

use super::{
    FirstTokenStreamSink, GenerateOutput, GenerateRequest, LanguageModelProvider, ModelError,
    ModelFuture, ModelUsage, StreamSink,
};
pub use kivio_agent_sdk::model::responses::output_from_responses;
use kivio_agent_sdk::model::responses::{OpenAiResponsesOptions, ResponsesFormat};

pub(super) struct OpenAiResponsesProvider<'a> {
    transport: ModelTransport<'a>,
    config: ProviderConfig<'a>,
}

impl<'a> OpenAiResponsesProvider<'a> {
    pub(super) fn new(transport: ModelTransport<'a>, config: ProviderConfig<'a>) -> Self {
        Self { transport, config }
    }
}

impl LanguageModelProvider for OpenAiResponsesProvider<'_> {
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

impl OpenAiResponsesProvider<'_> {
    /// 发送 Responses 请求；若严格端点拒绝 `prompt_cache_key` / `prompt_cache_retention`
    /// 而 400，记入 state 后去掉对应字段重试（与 openai.rs::send_chat_body 同形状）。
    async fn send_responses_body(
        &self,
        request: &GenerateRequest,
        stream: bool,
        label: &str,
    ) -> Result<reqwest::Response, ModelError> {
        let body = self.request_body(request, stream);
        let result = self.post_responses(request, &body, stream, label).await;
        if let Err(ref err) = result {
            let mut learned = false;
            if body.get("prompt_cache_key").is_some()
                && super::openai::error_rejects_prompt_cache_key(&err.message)
            {
                self.transport
                    .runtime
                    .mark_prompt_cache_key_unsupported(self.config.base_url);
                learned = true;
            } else if body.get("prompt_cache_retention").is_some()
                && super::openai::error_rejects_prompt_cache_retention(&err.message)
            {
                self.transport
                    .runtime
                    .mark_prompt_cache_retention_unsupported(self.config.base_url);
                learned = true;
            } else if body_replays_reasoning_items(&body)
                && error_maybe_rejects_reasoning_replay(&err.message)
            {
                // 有些中转不实现 stateless reasoning 回放（对 input 里的 reasoning item
                // 直接 4xx）。学习后本会话不再对该 base_url 回放——去掉 item 的请求永远
                // 合法，所以这条重试是严格安全的降级（回到修复前的行为）。
                self.transport
                    .runtime
                    .mark_reasoning_replay_unsupported(self.config.base_url);
                learned = true;
            }
            if learned {
                let retry_body = self.request_body(request, stream);
                return self
                    .post_responses(request, &retry_body, stream, label)
                    .await;
            }
        }
        result
    }

    /// 单次发送（带多 key failover）。非流式套总超时；流式不套，避免活跃 SSE 被砍断。
    async fn post_responses(
        &self,
        request: &GenerateRequest,
        body: &Value,
        stream: bool,
        label: &str,
    ) -> Result<reqwest::Response, ModelError> {
        send_model_with_failover(
            self.transport.runtime,
            label,
            self.transport.retry_attempts,
            self.config.id,
            self.config.api_keys,
            |key| {
                let mut req = self
                    .transport
                    .client
                    .post(self.responses_url())
                    .header(ACCEPT_ENCODING, "identity");
                req = crate::provider_request::apply_api_key_auth(
                    req,
                    crate::settings::ProviderApiFormat::OpenAiResponses,
                    key,
                );
                for (name, value) in self.extra_header_pairs(&request.metadata, body) {
                    req = req.header(name, value);
                }
                let req =
                    crate::api::attach_json_body(req, body, self.config.compress_request_body);
                let req = if stream {
                    req
                } else {
                    with_chat_request_timeout(req)
                };
                req.send()
            },
        )
        .await
    }

    async fn generate_inner(&self, request: GenerateRequest) -> Result<GenerateOutput, ModelError> {
        crate::chat::video::reject_unimplemented_transport(&request, "Responses")?;
        let label = request_label(&request, "Responses API");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let response = self
            .send_responses_body(&request, false, &label)
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
        let output =
            kivio_agent_sdk::model::responses::decode_responses_body(&raw, &label, &request.model)
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
        let label = request_label(&request, "Responses stream");
        crate::chat::video::reject_unimplemented_transport(&request, "Responses")?;
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let mut measured_sink = FirstTokenStreamSink::new(sink, started);
        let sink = &mut measured_sink;
        let response = self
            .send_responses_body(&request, true, &label)
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

        let output = kivio_agent_sdk::model::responses::decode_responses_stream(
            response,
            sink,
            &label,
            &request.model,
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

    fn responses_url(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        // 官方 DeepSeek 文档是 `POST https://api.deepseek.com/responses`。
        // Chat Completions 预设常带 `/v1`，直接拼接会打到 `/v1/responses`；剥掉 `/v1`。
        // `/anthropic` 不能走这条（那边是 Messages，不是 Responses）。
        if crate::utils::is_official_deepseek_api(base)
            && !crate::utils::is_official_deepseek_anthropic_api(base)
        {
            let origin = if base.to_ascii_lowercase().ends_with("/v1") {
                base[..base.len() - 3].trim_end_matches('/')
            } else {
                base
            };
            if origin.ends_with("/responses") {
                return origin.to_string();
            }
            return format!("{origin}/responses");
        }
        format!("{base}/responses")
    }

    fn request_body(&self, request: &GenerateRequest, stream: bool) -> Value {
        let is_xai = self.config.api_format == ProviderApiFormat::XaiResponses;
        let cache_key_supported = !is_xai
            && self.config.cache_retention != CacheRetention::None
            && !self
                .transport
                .runtime
                .prompt_cache_key_unsupported(self.config.base_url);
        let prompt_cache_key = cache_key_supported
            .then(|| {
                request
                    .metadata
                    .conversation_id
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
            })
            .flatten();
        let prompt_cache_retention = (prompt_cache_key.is_some()
            && self.config.cache_retention == CacheRetention::Long
            && !self
                .transport
                .runtime
                .prompt_cache_retention_unsupported(self.config.base_url))
        .then(|| "24h".to_string());
        let mut options = OpenAiResponsesOptions {
            format: if is_xai {
                ResponsesFormat::Xai
            } else {
                ResponsesFormat::OpenAi
            },
            temperature: crate::chat::model_metadata::temperature_for_request(
                request.options.temperature,
                Some(self.config.source),
                &request.model,
            ),
            replay_reasoning: !self
                .transport
                .runtime
                .reasoning_replay_unsupported(self.config.base_url),
            prompt_cache_key,
            prompt_cache_retention,
            ..Default::default()
        };
        if self.config.codex_identity && !is_xai {
            options.extra_body = serde_json::json!({ "include": ["reasoning.encrypted_content"] });
            if !request.tools.is_empty() || request.options.builtin_web_search {
                options.extra_body["parallel_tool_calls"] = Value::Bool(true);
            }
            if options.prompt_cache_key.is_some() {
                options.prompt_cache_key = Some(crate::provider_request::session_uuid(
                    request.metadata.conversation_id.as_deref(),
                ));
            }
        }
        let mut body = kivio_agent_sdk::model::responses::request_body(request, stream, &options);
        if self.config.codex_oauth {
            crate::provider_oauth::codex_body(&mut body);
        }
        body
    }

    /// 重建本次请求实际会带的 headers（脱敏后）供请求调试面板展示。镜像发送路径：
    /// bearer_auth(key) + Accept-Encoding identity + JSON content-type + `extra_header_pairs`。
    /// Authorization 用首个 key（正常发送用的也是它）派生脱敏预览。
    fn debug_request_headers(
        &self,
        metadata: &crate::chat::model::RequestMetadata,
        body: &Value,
    ) -> std::collections::BTreeMap<String, String> {
        let mut headers = std::collections::BTreeMap::new();
        if let Some(key) = self.config.preferred_api_key {
            headers.insert("Authorization".to_string(), format!("Bearer {key}"));
        }
        headers.insert("Accept-Encoding".to_string(), "identity".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        for (name, value) in self.extra_header_pairs(metadata, body) {
            headers.insert(name, value);
        }
        crate::chat::request_debug::sanitize_headers(headers)
    }

    /// 发送路径与请求调试面板共用的附加头：会话亲和头（与 Chat Completions 一致）+
    /// CLI 身份 / 自定义 / OAuth 头 + 缺省 UA / Accept。
    /// Codex 身份不发亲和头：真实 Codex 不带 `x-session-id`，而且值是 `conv_*`，
    /// 和身份头里的 UUID `session_id` 摆在一起就露馅了。
    /// Accept 按**最终请求体**的 `stream` 取值：Codex OAuth 会把体强制改成流式。
    fn extra_header_pairs(
        &self,
        metadata: &crate::chat::model::RequestMetadata,
        body: &Value,
    ) -> Vec<(String, String)> {
        crate::provider_request::model_header_pairs(
            self.config.source,
            metadata.conversation_id.as_deref(),
            !self.config.codex_identity,
            body["stream"] == true,
        )
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
        let body = self.request_body(request, stream);
        let record = crate::chat::request_debug::build_debug_record(
            crate::chat::request_debug::DebugRecordArgs {
                provider: self.config.source,
                request,
                label,
                started_at,
                duration_ms: duration.as_millis() as u64,
                status: "success",
                url: self.responses_url(),
                headers: self.debug_request_headers(&request.metadata, &body),
                body,
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
        let body = self.request_body(request, stream);
        let record = crate::chat::request_debug::build_debug_record(
            crate::chat::request_debug::DebugRecordArgs {
                provider: self.config.source,
                request,
                label,
                started_at,
                duration_ms: duration.as_millis() as u64,
                status: "error",
                url: self.responses_url(),
                headers: self.debug_request_headers(&request.metadata, &body),
                body,
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

/// 请求体的 `input` 里是否带了回放的 reasoning item（学习兜底的触发前提）。
fn body_replays_reasoning_items(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("reasoning"))
        })
}

/// 该错误是否可能是端点拒绝 reasoning 回放。各家中转的 4xx 文案千奇百怪，无法可靠
/// 模式匹配，所以宽松地认所有请求级 4xx（400/404/422）：误判的代价只是该端点本会话
/// 退回「不回放」（= 修复前的行为），而漏判的代价是对话每轮 4xx 直接不可用。
/// 401/402/403/429 是 key/配额问题，归 failover 管，不在此列。
fn error_maybe_rejects_reasoning_replay(err: &str) -> bool {
    matches!(
        crate::api::extract_status_code(err),
        Some(400) | Some(404) | Some(422)
    )
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
    use crate::settings::{ModelInfo, ModelProvider};

    fn responses_temperature_body(
        provider_temperature: Option<f64>,
        request_temperature: Option<f64>,
    ) -> Value {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let model = "custom-temperature-model";
        let mut model_overrides = std::collections::HashMap::new();
        if let Some(temperature) = provider_temperature {
            model_overrides.insert(
                model.into(),
                ModelInfo {
                    temperature: Some(temperature),
                    ..ModelInfo::default()
                },
            );
        }
        let provider = ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://api.openai.com/v1".into(),
            available_models: vec![model.into()],
            enabled_models: vec![model.into()],
            enabled: true,
            api_format: "openai_responses".into(),
            model_overrides,
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let request = GenerateRequest {
            model: model.into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions {
                temperature: request_temperature,
                ..Default::default()
            },
            metadata: Default::default(),
        };
        OpenAiResponsesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        )
        .request_body(&request, false)
    }

    fn codex_identity_case(
        cli_identity: &str,
        thinking: bool,
        provider_options: Value,
    ) -> (Value, std::collections::BTreeMap<String, String>) {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut provider = ModelProvider {
            id: "test".into(),
            name: "Relay".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://relay.example/v1".into(),
            available_models: vec!["gpt-5.5".into()],
            enabled_models: vec!["gpt-5.5".into()],
            enabled: true,
            api_format: "openai_responses".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        provider.request.cli_identity = cli_identity.into();
        provider.request.prompt_caching = Some(true);
        let request = GenerateRequest {
            model: "gpt-5.5".into(),
            system: "sys".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: vec![crate::chat::model::ModelTool {
                name: "read".into(),
                description: "read".into(),
                input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            }],
            options: GenerateOptions {
                thinking_enabled: thinking,
                provider_options,
                ..Default::default()
            },
            metadata: crate::chat::model::RequestMetadata {
                conversation_id: Some("conv_abc".into()),
                ..Default::default()
            },
        };
        let adapter = OpenAiResponsesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        (
            adapter.request_body(&request, false),
            adapter
                .debug_request_headers(&request.metadata, &adapter.request_body(&request, false)),
        )
    }

    #[test]
    fn plain_responses_sends_store_false_and_session_headers() {
        // 关思考的普通调用也要无状态，不再只在思考分支里发。
        let (body, headers) = codex_identity_case("", false, Value::Null);
        assert_eq!(body["store"], false, "{body}");
        assert!(body.get("parallel_tool_calls").is_none());
        assert_eq!(body["prompt_cache_key"], "conv_abc");
        assert_eq!(headers["x-session-id"], "conv_abc");
        assert!(headers["User-Agent"].starts_with("Kivio/"));
        assert_eq!(headers["Accept"], "application/json");
    }

    #[test]
    fn accept_follows_final_body_stream_flag() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let provider = ModelProvider {
            id: "test".into(),
            name: "Relay".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://relay.example/v1".into(),
            available_models: vec!["gpt-5.5".into()],
            enabled_models: vec!["gpt-5.5".into()],
            enabled: true,
            api_format: "openai_responses".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = OpenAiResponsesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let metadata = Default::default();
        // SSE 请求不能报 Accept: application/json（Codex OAuth 会把体强制改成 stream:true）。
        let streaming = serde_json::json!({ "stream": true });
        assert_eq!(
            adapter.debug_request_headers(&metadata, &streaming)["Accept"],
            "text/event-stream"
        );
        assert_eq!(
            adapter.debug_request_headers(&metadata, &serde_json::json!({}))["Accept"],
            "application/json"
        );
    }

    #[test]
    fn codex_identity_aligns_responses_body_with_headers() {
        let (body, headers) = codex_identity_case("codex", false, Value::Null);
        let session = crate::provider_request::session_uuid(Some("conv_abc"));
        // prompt_cache_key 与头里的 session_id 同值，且是 UUID。
        assert_eq!(body["prompt_cache_key"], session.as_str());
        assert_eq!(headers["session_id"], session);
        assert_eq!(headers["originator"], "codex_cli_rs");
        assert!(!headers.contains_key("x-session-id"), "{headers:?}");
        assert!(!headers.contains_key("x-session-affinity"), "{headers:?}");
        assert_eq!(body["store"], false);
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(
            body["include"],
            serde_json::json!(["reasoning.encrypted_content"])
        );
        // 不擅自改用户选的思考档位。
        assert_eq!(body["reasoning"]["effort"], "none");
    }

    #[test]
    fn explicit_options_override_codex_identity_defaults() {
        let (body, _) = codex_identity_case(
            "codex",
            false,
            serde_json::json!({
                "include": ["custom"],
                "parallel_tool_calls": false,
                "prompt_cache_key": "custom-cache",
                "reasoning": { "effort": "high" },
            }),
        );
        assert_eq!(body["include"], serde_json::json!(["custom"]));
        assert_eq!(body["parallel_tool_calls"], false);
        assert_eq!(body["prompt_cache_key"], "custom-cache");
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn responses_prompt_cache_retention_none_long_and_learn() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let body_with = |retention: &str| {
            let provider = ModelProvider {
                id: "test".into(),
                name: "OpenAI".into(),
                api_keys: vec!["sk-test".into()],
                api_key_legacy: None,
                base_url: "https://api.openai.com/v1".into(),
                available_models: vec!["gpt-5.5".into()],
                enabled_models: vec!["gpt-5.5".into()],
                enabled: true,
                api_format: "openai_responses".into(),
                model_overrides: Default::default(),
                compress_request_body: false,
                request: crate::settings::ProviderRequestConfig {
                    prompt_cache_retention: retention.into(),
                    ..Default::default()
                },
                active_key_index: 0,
            };
            let request = GenerateRequest {
                model: "gpt-5.5".into(),
                system: "sys".into(),
                messages: vec![ModelMessage {
                    role: ModelRole::User,
                    content: vec![MessagePart::Text { text: "hi".into() }],
                }],
                tools: Vec::new(),
                options: GenerateOptions::default(),
                metadata: crate::chat::model::RequestMetadata {
                    conversation_id: Some("conv_abc".into()),
                    ..Default::default()
                },
            };
            OpenAiResponsesProvider::new(
                ModelTransport::for_test(&state, &provider),
                ProviderConfig::from_settings(&provider),
            )
            .request_body(&request, false)
        };
        assert!(body_with("none").get("prompt_cache_key").is_none());
        let short = body_with("short");
        assert_eq!(short["prompt_cache_key"], "conv_abc");
        assert!(short.get("prompt_cache_retention").is_none());
        let long = body_with("long");
        assert_eq!(long["prompt_cache_key"], "conv_abc");
        assert_eq!(long["prompt_cache_retention"], "24h");
        state
            .provider_runtime()
            .mark_prompt_cache_retention_unsupported("https://api.openai.com/v1");
        let after = body_with("long");
        assert_eq!(after["prompt_cache_key"], "conv_abc");
        assert!(after.get("prompt_cache_retention").is_none());
    }

    fn replay_request_body(
        state: &crate::state::AppState,
        api_format: &str,
        part_model: &str,
        request_model: &str,
    ) -> Value {
        let provider = ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://relay.example.com/v1".into(),
            available_models: vec![request_model.into()],
            enabled_models: vec![request_model.into()],
            enabled: true,
            api_format: api_format.into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let request = GenerateRequest {
            model: request_model.into(),
            system: String::new(),
            messages: vec![
                ModelMessage::text(ModelRole::User, "看下这张图"),
                ModelMessage {
                    role: ModelRole::Assistant,
                    content: vec![
                        MessagePart::ReasoningItem {
                            model: part_model.into(),
                            item: serde_json::json!({
                                "type": "reasoning", "id": "rs_1", "summary": [],
                                "encrypted_content": "gAAA"
                            }),
                        },
                        MessagePart::ToolCall {
                            id: "call_1".into(),
                            name: "read".into(),
                            arguments: serde_json::json!({ "path": "a.png" }),
                            arguments_raw: "{\"path\":\"a.png\"}".into(),
                            signature: None,
                        },
                    ],
                },
                ModelMessage {
                    role: ModelRole::Tool,
                    content: vec![MessagePart::ToolResult {
                        tool_call_id: "call_1".into(),
                        content: "图片内容描述".into(),
                        is_error: false,
                        artifacts: Vec::new(),
                    }],
                },
            ],
            tools: Vec::new(),
            options: Default::default(),
            metadata: Default::default(),
        };
        OpenAiResponsesProvider::new(
            ModelTransport::for_test(state, &provider),
            ProviderConfig::from_settings(&provider),
        )
        .request_body(&request, false)
    }

    fn fresh_state() -> crate::state::AppState {
        crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        )
    }

    #[test]
    fn request_body_replays_matching_reasoning_items_in_native_order() {
        let state = fresh_state();
        let body = replay_request_body(&state, "openai_responses", "gpt-5.6", "gpt-5.6");
        let input = body["input"].as_array().unwrap();
        // user → reasoning → function_call → function_call_output
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["id"], "rs_1");
        assert_eq!(input[1]["encrypted_content"], "gAAA");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[3]["type"], "function_call_output");
    }

    #[test]
    fn request_body_skips_reasoning_items_on_model_switch_and_xai() {
        // 会话中途换模型：旧密文别的模型解不开，回放必 400 → 跳过。
        let state = fresh_state();
        let body = replay_request_body(&state, "openai_responses", "gpt-5.6", "gpt-6");
        let input = body["input"].as_array().unwrap();
        assert!(input.iter().all(|item| item["type"] != "reasoning"));

        // xAI：types 的回放一直丢弃推理项，且我们不向它要密文。
        let body = replay_request_body(&state, "xai_responses", "grok-4.3", "grok-4.3");
        let input = body["input"].as_array().unwrap();
        assert!(input.iter().all(|item| item["type"] != "reasoning"));
    }

    #[test]
    fn learned_unsupported_endpoint_stops_replaying_reasoning_items() {
        let state = fresh_state();
        state
            .provider_runtime()
            .mark_reasoning_replay_unsupported("https://relay.example.com/v1");
        let body = replay_request_body(&state, "openai_responses", "gpt-5.6", "gpt-5.6");
        let input = body["input"].as_array().unwrap();
        assert!(
            input.iter().all(|item| item["type"] != "reasoning"),
            "已学习为不支持的端点不再回放: {body}"
        );
    }

    #[test]
    fn reasoning_replay_learning_triggers_only_on_request_level_4xx() {
        let body_with_replay = serde_json::json!({
            "input": [{ "type": "reasoning", "id": "rs_1", "encrypted_content": "gAAA" }]
        });
        assert!(body_replays_reasoning_items(&body_with_replay));
        assert!(!body_replays_reasoning_items(
            &serde_json::json!({ "input": [{ "role": "user", "content": [] }] })
        ));

        // 错误串遵循 api.rs 的约定格式 `"{label} Error: {status} - {body}"`。
        assert!(error_maybe_rejects_reasoning_replay(
            "Responses stream Error: 400 Bad Request - invalid item rs_1"
        ));
        // 401/429 是 key/配额问题，归 failover；5xx 是服务端问题——都不该学习。
        assert!(!error_maybe_rejects_reasoning_replay(
            "Responses stream Error: 401 Unauthorized - bad key"
        ));
        assert!(!error_maybe_rejects_reasoning_replay(
            "Responses stream Error: 429 Too Many Requests - slow down"
        ));
        assert!(!error_maybe_rejects_reasoning_replay(
            "Responses stream Error: 502 Bad Gateway - upstream"
        ));
        // 网络错误无状态码 → 不学习。
        assert!(!error_maybe_rejects_reasoning_replay(
            "connection reset by peer"
        ));
    }

    #[test]
    fn request_body_temperature_is_optional_and_model_scoped() {
        let default_body = responses_temperature_body(None, None);
        assert!(default_body.get("temperature").is_none());

        let configured_body = responses_temperature_body(Some(0.4), None);
        assert_eq!(configured_body["temperature"], serde_json::json!(0.4));

        let explicit_body = responses_temperature_body(Some(0.4), Some(1.2));
        assert_eq!(explicit_body["temperature"], serde_json::json!(1.2));
    }
}
