use reqwest::header::ACCEPT_ENCODING;
use serde_json::Value;

use super::{
    FirstTokenStreamSink, GenerateOutput, GenerateRequest, LanguageModelProvider, ModelError,
    ModelFuture, ModelUsage, StreamSink,
};
use super::{ModelTransport, ProviderConfig};
use crate::api::{send_model_with_failover, with_chat_request_timeout};
use crate::settings::CacheRetention;
use crate::usage::{
    chat_usage_source_for_label, error_kind_from_message, operation_from_label, UsageRecordInput,
};
use crate::utils;
pub use kivio_agent_sdk::model::openai::output_from_chat_completion;

pub(super) struct OpenAiChatProvider<'a> {
    transport: ModelTransport<'a>,
    config: ProviderConfig<'a>,
}

impl<'a> OpenAiChatProvider<'a> {
    pub(super) fn new(transport: ModelTransport<'a>, config: ProviderConfig<'a>) -> Self {
        Self { transport, config }
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
}

/// 判断错误是否是"端点拒绝 prompt_cache_key 字段"。仅在本次确实发了该字段时配合调用，
/// 错误体点名该字段即视为命中（NVIDIA NIM / 智谱 GLM 等严格端点的 400 会带上字段名）。
pub(super) fn error_rejects_prompt_cache_key(err: &str) -> bool {
    err.contains("prompt_cache_key")
}

/// 判断错误是否点名拒绝 `prompt_cache_retention`（long 的 24h 字段）。
pub(super) fn error_rejects_prompt_cache_retention(err: &str) -> bool {
    err.contains("prompt_cache_retention")
}

impl OpenAiChatProvider<'_> {
    /// 发送 chat 请求；若严格端点拒绝 `prompt_cache_key` / `prompt_cache_retention` 而 400，
    /// 记入 state 后去掉对应字段重试一次（本会话后续 request_body 就地跳过）。
    async fn send_chat_body(
        &self,
        request: &GenerateRequest,
        stream: bool,
        label: &str,
    ) -> Result<reqwest::Response, ModelError> {
        let body = self.request_body(request, stream);
        let result = self.post_chat(request, &body, stream, label).await;
        if let Err(ref err) = result {
            let mut learned = false;
            // key 被拒：key 与 24h 一并消失（24h 只跟 key 一起发）。
            if body.get("prompt_cache_key").is_some()
                && error_rejects_prompt_cache_key(&err.message)
            {
                self.transport
                    .runtime
                    .mark_prompt_cache_key_unsupported(self.config.base_url);
                learned = true;
            } else if body.get("prompt_cache_retention").is_some()
                && error_rejects_prompt_cache_retention(&err.message)
            {
                // 只拒 24h：保留 key，停发 retention。
                self.transport
                    .runtime
                    .mark_prompt_cache_retention_unsupported(self.config.base_url);
                learned = true;
            }
            if learned {
                let retry_body = self.request_body(request, stream);
                return self.post_chat(request, &retry_body, stream, label).await;
            }
        }
        result
    }

    /// 单次发送（带多 key failover）。非流式套 `with_chat_request_timeout` 总超时；
    /// 流式不套，避免活跃 SSE 被总超时砍断（与改动前两条路径各自的行为一致）。
    async fn post_chat(
        &self,
        request: &GenerateRequest,
        body: &Value,
        stream: bool,
        label: &str,
    ) -> Result<reqwest::Response, ModelError> {
        if crate::chat::video::has_video(request) {
            crate::chat::video::validate_request(self.config.source, request)?;
            crate::chat::video::validate_body(body)?;
        }
        if self.config.opencode_free && !crate::opencode_free::is_free_model(&request.model) {
            return Err("OpenCode Free only supports free models; refresh the model list".into());
        }
        send_model_with_failover(
            self.transport.runtime,
            label,
            self.transport.retry_attempts,
            self.config.id,
            self.config.api_keys,
            |key| {
                let req = self.transport.client.post(self.chat_completions_url());
                let req = crate::provider_request::apply_api_key_auth(
                    req,
                    crate::settings::ProviderApiFormat::OpenAiChat,
                    key,
                );
                let req = crate::api::attach_json_body(
                    self.with_session_headers(
                        req.header(ACCEPT_ENCODING, "identity"),
                        &request.metadata,
                        stream,
                    ),
                    body,
                    self.config.compress_request_body,
                );
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
        let label = request_label(&request, "Chat API");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let response = self
            .send_chat_body(&request, false, &label)
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
        let output = output_from_chat_completion(&value, &raw, &label)?;
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
        let label = request_label(&request, "Chat stream");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let mut measured_sink = FirstTokenStreamSink::new(sink, started);
        let sink = &mut measured_sink;
        let response = self
            .send_chat_body(&request, true, &label)
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

        let output = kivio_agent_sdk::model::openai::decode_chat_stream(response, sink, &label)
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

    fn chat_completions_url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    /// 会话亲和头 + 供应商「请求配置」的 CLI 身份头 / 自定义头 + 缺省 UA / Accept，
    /// 由 `provider_request::model_header_pairs` 统一装配（同名只留一条，自定义优先）。
    fn with_session_headers(
        &self,
        request: reqwest::RequestBuilder,
        metadata: &crate::chat::model::RequestMetadata,
        stream: bool,
    ) -> reqwest::RequestBuilder {
        let mut request = request;
        for (name, value) in crate::provider_request::model_header_pairs(
            self.config.source,
            metadata.conversation_id.as_deref(),
            true,
            stream,
        ) {
            request = request.header(name, value);
        }
        request
    }

    /// 重建本次请求实际会带的 headers（脱敏后）供请求调试面板展示。静态头（Authorization/
    /// Accept-Encoding/Content-Type）与发送路径一一对应；其余头与发送路径共用
    /// `model_header_pairs`，故与真实发送零漂移。Authorization 用首个 key 派生脱敏预览。
    fn debug_request_headers(
        &self,
        metadata: &crate::chat::model::RequestMetadata,
        stream: bool,
    ) -> std::collections::BTreeMap<String, String> {
        let mut headers = std::collections::BTreeMap::new();
        if let Some(key) = self.config.preferred_api_key {
            headers.insert("Authorization".to_string(), format!("Bearer {key}"));
        }
        headers.insert("Accept-Encoding".to_string(), "identity".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        for (name, value) in crate::provider_request::model_header_pairs(
            self.config.source,
            metadata.conversation_id.as_deref(),
            true,
            stream,
        ) {
            headers.insert(name, value);
        }
        crate::chat::request_debug::sanitize_headers(headers)
    }

    /// 记录一次成功调用到请求调试缓冲。开关关时首行短路 → 不构造 body/headers（零开销）。
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
                url: self.chat_completions_url(),
                headers: self.debug_request_headers(&request.metadata, stream),
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
                url: self.chat_completions_url(),
                headers: self.debug_request_headers(&request.metadata, stream),
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

    fn request_body(&self, request: &GenerateRequest, stream: bool) -> Value {
        let model_id = request.model.to_ascii_lowercase();
        let model_id = model_id.rsplit('/').next().unwrap_or(&model_id);
        let kimi_always_on = model_id.starts_with("kimi-k3") || model_id.starts_with("kimi-k2.7");
        let native_thinking_switch =
            (utils::provider_supports_thinking_field(self.config.base_url) && !kimi_always_on)
                || (model_id.starts_with("deepseek-")
                    && reqwest::Url::parse(self.config.base_url)
                        .ok()
                        .is_some_and(|url| url.host_str() == Some("hyper.charm.land")));
        let prompt_cache_key = (self.config.cache_retention != CacheRetention::None
            && !self
                .transport
                .runtime
                .prompt_cache_key_unsupported(self.config.base_url))
        .then(|| request.metadata.conversation_id.clone())
        .flatten();
        let prompt_cache_retention = (matches!(self.config.cache_retention, CacheRetention::Long)
            && !self
                .transport
                .runtime
                .prompt_cache_retention_unsupported(self.config.base_url))
        .then(|| "24h".to_string());
        let options = kivio_agent_sdk::model::openai::OpenAiChatOptions {
            temperature: crate::chat::model_metadata::temperature_for_request(
                request.options.temperature,
                Some(self.config.source),
                &request.model,
            ),
            extra_body: self
                .config
                .source
                .model_overrides
                .get(&request.model)
                .and_then(|info| info.extra_body.clone())
                .unwrap_or_default(),
            prompt_cache_key,
            prompt_cache_retention,
            native_thinking_switch,
        };
        kivio_agent_sdk::model::openai::request_body(request, stream, &options)
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
    use crate::state::AppState;

    /// Build a real OpenAI-compatible provider request body via the production
    /// `request_body` path and assert how thinking maps to the wire.
    fn build_openai_body(thinking_level: Option<&str>, base_url: &str) -> Value {
        build_openai_body_with(thinking_level, true, base_url)
    }

    fn build_openai_body_with(
        thinking_level: Option<&str>,
        thinking_enabled: bool,
        base_url: &str,
    ) -> Value {
        build_openai_model_body(thinking_level, thinking_enabled, base_url, "gpt-5")
    }

    fn build_openai_model_body(
        thinking_level: Option<&str>,
        thinking_enabled: bool,
        base_url: &str,
        model: &str,
    ) -> Value {
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        let provider = ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: base_url.into(),
            available_models: vec!["gpt-5".into()],
            enabled_models: vec!["gpt-5".into()],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = OpenAiChatProvider::new(
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
                thinking_enabled,
                thinking_level: thinking_level.map(|s| s.to_string()),
                ..Default::default()
            },
            metadata: Default::default(),
        };
        adapter.request_body(&request, false)
    }

    fn build_openai_temperature_body(
        provider_temperature: Option<f64>,
        request_temperature: Option<f64>,
    ) -> Value {
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        let mut model_overrides = std::collections::HashMap::new();
        if let Some(temperature) = provider_temperature {
            model_overrides.insert(
                "custom-temperature-model".into(),
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
            base_url: "https://api.example.com/v1".into(),
            available_models: vec!["custom-temperature-model".into()],
            enabled_models: vec!["custom-temperature-model".into()],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides,
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = OpenAiChatProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let request = GenerateRequest {
            model: "custom-temperature-model".into(),
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
        adapter.request_body(&request, false)
    }

    #[test]
    fn thinking_level_maps_to_reasoning_effort() {
        // 开思考但未设等级 → 不发 reasoning_effort（与改动前一致）。
        let none = build_openai_body(None, "https://api.openai.com/v1");
        assert!(none.get("reasoning_effort").is_none(), "body: {none}");

        // 选了等级 → 发标准 reasoning_effort。
        let mid = build_openai_body(Some("medium"), "https://api.openai.com/v1");
        eprintln!("[openai reasoning_effort=medium] {mid}");
        assert_eq!(mid["reasoning_effort"], "medium");

        let high = build_openai_body(Some("high"), "https://api.openai.com/v1");
        assert_eq!(high["reasoning_effort"], "high");
    }

    #[test]
    fn thinking_off_sends_explicit_none_on_openai_compat() {
        // UI Off → 显式 reasoning_effort:"none"，不能省略（多家省略默认 high）。
        let off = build_openai_body_with(None, false, "https://api.openai.com/v1");
        assert_eq!(off["reasoning_effort"], "none", "body: {off}");
        assert!(off.get("thinking").is_none(), "body: {off}");

        // 代理 / 中转同样走 none（base_url 不含 deepseek.com / moonshot.cn）。
        let relay = build_openai_body_with(None, false, "https://relay.example.com/v1");
        assert_eq!(relay["reasoning_effort"], "none", "body: {relay}");
    }

    #[test]
    fn hyper_deepseek_uses_native_thinking_switch() {
        let base = "https://hyper.charm.land/v1";
        let off = build_openai_model_body(None, false, base, "deepseek-v4-flash");
        assert_eq!(off["thinking"]["type"], "disabled", "body: {off}");
        assert!(off.get("reasoning_effort").is_none(), "body: {off}");
        let on = build_openai_model_body(Some("high"), true, base, "deepseek-v4-flash");
        assert_eq!(on["thinking"]["type"], "enabled", "body: {on}");
        assert_eq!(on["reasoning_effort"], "high", "body: {on}");
        let other = build_openai_model_body(None, false, base, "gpt-5");
        assert_eq!(other["reasoning_effort"], "none", "body: {other}");
        assert!(other.get("thinking").is_none(), "body: {other}");
    }

    #[test]
    fn thinking_off_uses_native_switch_on_official_endpoints() {
        // DeepSeek / Kimi 官方 Chat Completions：开关是 thinking.type，不是
        // reasoning_effort:"none"（官方 schema 的 effort 只有 low/high/max）。
        let ds = build_openai_body_with(None, false, "https://api.deepseek.com/v1");
        assert_eq!(ds["thinking"]["type"], "disabled", "body: {ds}");
        assert!(
            ds.get("reasoning_effort").is_none(),
            "DeepSeek Chat 不该发 none: {ds}"
        );

        let kimi = build_openai_body_with(None, false, "https://api.moonshot.cn/v1");
        assert_eq!(kimi["thinking"]["type"], "disabled", "body: {kimi}");
        assert!(kimi.get("reasoning_effort").is_none(), "body: {kimi}");

        for base in [
            "https://api.z.ai/api/paas/v4",
            "https://open.bigmodel.cn/api/paas/v4",
        ] {
            let glm = build_openai_model_body(None, false, base, "glm-5.1");
            assert_eq!(glm["thinking"]["type"], "disabled", "body: {glm}");
            assert!(glm.get("reasoning_effort").is_none(), "body: {glm}");
            let on = build_openai_model_body(Some("low"), true, base, "glm-5.3");
            assert_eq!(on["thinking"]["type"], "enabled", "body: {on}");
            assert_eq!(on["reasoning_effort"], "low");
        }
    }

    #[test]
    fn kimi_international_endpoint_respects_model_thinking_contract() {
        let base = "https://api.moonshot.ai/v1";
        let off = build_openai_model_body(None, false, base, "kimi-k2.6");
        assert_eq!(off["thinking"]["type"], "disabled", "body: {off}");
        assert!(off.get("reasoning_effort").is_none());
        let k3 = build_openai_model_body(Some("low"), true, base, "kimi-k3");
        assert!(k3.get("thinking").is_none(), "body: {k3}");
        assert_eq!(k3["reasoning_effort"], "low");
        let code = build_openai_model_body(None, true, base, "kimi-k2.7-code");
        assert!(code.get("thinking").is_none(), "body: {code}");
    }

    #[test]
    fn thinking_on_sends_thinking_enabled_on_deepseek_and_kimi() {
        let ds = build_openai_body(Some("high"), "https://api.deepseek.com/v1");
        assert_eq!(ds["thinking"]["type"], "enabled", "body: {ds}");
        assert_eq!(ds["reasoning_effort"], "high", "body: {ds}");

        let kimi = build_openai_body(Some("high"), "https://api.moonshot.cn/v1");
        assert_eq!(kimi["thinking"]["type"], "enabled", "body: {kimi}");
        assert_eq!(kimi["reasoning_effort"], "high", "body: {kimi}");

        let openai = build_openai_body(Some("high"), "https://api.openai.com/v1");
        assert!(
            openai.get("thinking").is_none(),
            "OpenAI Chat 不该发 thinking 对象: {openai}"
        );
    }

    #[test]
    fn request_body_omits_temperature_by_default() {
        let body = build_openai_temperature_body(None, None);
        assert!(body.get("temperature").is_none(), "body: {body}");
    }

    #[test]
    fn request_body_omits_max_tokens_when_zero() {
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        let provider = ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".into(),
            available_models: vec!["glm-5.3".into()],
            enabled_models: vec!["glm-5.3".into()],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = OpenAiChatProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let request = GenerateRequest {
            model: "glm-5.3".into(),
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
        let body = adapter.request_body(&request, true);
        assert!(
            body.get("max_tokens").is_none(),
            "max_tokens=0 must omit the field: {body}"
        );
        let capped = GenerateRequest {
            options: GenerateOptions {
                max_tokens: 16_384,
                ..Default::default()
            },
            ..request
        };
        let capped_body = adapter.request_body(&capped, true);
        assert_eq!(capped_body["max_tokens"], 16_384);
    }

    #[test]
    fn request_body_uses_provider_temperature_override() {
        let body = build_openai_temperature_body(Some(0.4), None);
        assert_eq!(body["temperature"], serde_json::json!(0.4), "body: {body}");
    }

    #[test]
    fn explicit_request_temperature_wins_over_provider_override() {
        let body = build_openai_temperature_body(Some(0.4), Some(1.2));
        assert_eq!(body["temperature"], serde_json::json!(1.2), "body: {body}");
    }

    #[test]
    fn model_extra_body_merges_into_request_body_root() {
        // model_overrides[model].extra_body 原样进 body 根部（NVIDIA NIM chat_template_kwargs 用例）。
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        let mut model_overrides = std::collections::HashMap::new();
        model_overrides.insert(
            "deepseek-ai/deepseek-v4-pro".to_string(),
            ModelInfo {
                extra_body: Some(serde_json::json!({
                    "chat_template_kwargs": { "thinking": true }
                })),
                ..ModelInfo::default()
            },
        );
        let provider = ModelProvider {
            id: "nv".into(),
            name: "NV".into(),
            api_keys: vec!["nvapi-test".into()],
            api_key_legacy: None,
            base_url: "https://integrate.api.nvidia.com/v1".into(),
            available_models: vec!["deepseek-ai/deepseek-v4-pro".into()],
            enabled_models: vec!["deepseek-ai/deepseek-v4-pro".into()],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides,
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = OpenAiChatProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let request = GenerateRequest {
            model: "deepseek-ai/deepseek-v4-pro".into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        };
        let body = adapter.request_body(&request, false);
        assert_eq!(
            body["chat_template_kwargs"],
            serde_json::json!({ "thinking": true }),
            "body: {body}"
        );

        // 无 override 的模型不受影响。
        let plain = GenerateRequest {
            model: "other-model".into(),
            ..request
        };
        let plain_body = adapter.request_body(&plain, false);
        assert!(
            plain_body.get("chat_template_kwargs").is_none(),
            "body: {plain_body}"
        );
    }

    #[test]
    fn request_body_carries_session_cache_key_and_stream_usage() {
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        let provider = ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-test".into()],
            api_key_legacy: None,
            base_url: "https://api.openai.com/v1".into(),
            available_models: vec!["gpt-5".into()],
            enabled_models: vec!["gpt-5".into()],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let adapter = OpenAiChatProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let tool = crate::chat::model::ModelTool {
            name: "web_fetch".into(),
            description: "Fetch".into(),
            input_schema: serde_json::json!({ "type": "object" }),
        };
        let mut request = GenerateRequest {
            model: "gpt-5".into(),
            system: "sys".into(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text { text: "hi".into() }],
            }],
            tools: vec![tool],
            options: GenerateOptions::default(),
            metadata: crate::chat::model::RequestMetadata {
                conversation_id: Some("conv_abc".into()),
                ..Default::default()
            },
        };

        // 流式 + 有会话 id + 有工具：只发标准 snake_case 缓存键（不发驼峰 promptCacheKey，
        // 避免严格端点对未知字段 400）、stream_options、tool_choice 齐全。
        let body = adapter.request_body(&request, true);
        assert_eq!(body["prompt_cache_key"], "conv_abc");
        assert!(body.get("promptCacheKey").is_none());
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["tool_choice"], "auto");

        // 非流式：不带 stream_options；无会话 id：不带缓存键；无工具：不带 tool_choice。
        request.metadata.conversation_id = None;
        request.tools = Vec::new();
        let body = adapter.request_body(&request, false);
        assert!(body.get("stream_options").is_none());
        assert!(body.get("prompt_cache_key").is_none());
        assert!(body.get("promptCacheKey").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn learned_unsupported_endpoint_skips_prompt_cache_key() {
        // 端点被记入 prompt_cache_key_unsupported 后，即便有会话 id 也不再发该字段；
        // 未记入的端点照常硬发。这是「默认开 + 400 自愈」学习后的就地跳过。
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        let make = |base_url: &str| {
            let provider = ModelProvider {
                id: "test".into(),
                name: "Test".into(),
                api_keys: vec!["sk-test".into()],
                api_key_legacy: None,
                base_url: base_url.into(),
                available_models: vec!["m".into()],
                enabled_models: vec!["m".into()],
                enabled: true,
                api_format: "openai_chat".into(),
                model_overrides: Default::default(),
                compress_request_body: false,
                request: Default::default(),
                active_key_index: 0,
            };
            let adapter = OpenAiChatProvider::new(
                ModelTransport::for_test(&state, &provider),
                ProviderConfig::from_settings(&provider),
            );
            let request = GenerateRequest {
                model: "m".into(),
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
            adapter.request_body(&request, false)
        };

        // 未学习：中转站也硬发。
        assert_eq!(
            make("https://integrate.api.nvidia.com/v1")["prompt_cache_key"],
            "conv_abc"
        );
        // 学习该端点拒绝后：就地跳过。
        state
            .provider_runtime()
            .mark_prompt_cache_key_unsupported("https://integrate.api.nvidia.com/v1");
        assert!(make("https://integrate.api.nvidia.com/v1")
            .get("prompt_cache_key")
            .is_none());
        // 其它端点不受影响，仍发送。
        assert_eq!(
            make("https://api.openai.com/v1")["prompt_cache_key"],
            "conv_abc"
        );
    }

    #[test]
    fn prompt_caching_toggle_gates_the_cache_key() {
        // retention=none 不发；short 发 key；long 发 key+24h（对齐 pi，不按主机降级）。
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        let body_with = |base_url: &str, retention: &str| {
            let provider = ModelProvider {
                id: "test".into(),
                name: "Test".into(),
                api_keys: vec!["sk-test".into()],
                api_key_legacy: None,
                base_url: base_url.into(),
                available_models: vec!["m".into()],
                enabled_models: vec!["m".into()],
                enabled: true,
                api_format: "openai_chat".into(),
                model_overrides: Default::default(),
                compress_request_body: false,
                request: crate::settings::ProviderRequestConfig {
                    prompt_cache_retention: retention.into(),
                    ..Default::default()
                },
                active_key_index: 0,
            };
            let adapter = OpenAiChatProvider::new(
                ModelTransport::for_test(&state, &provider),
                ProviderConfig::from_settings(&provider),
            );
            adapter.request_body(
                &GenerateRequest {
                    model: "m".into(),
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
                },
                false,
            )
        };
        let short = body_with("https://api.openai.com/v1", "short");
        assert_eq!(short["prompt_cache_key"], "conv_abc");
        assert!(short.get("prompt_cache_retention").is_none());
        assert!(body_with("https://api.openai.com/v1", "none")
            .get("prompt_cache_key")
            .is_none());
        let long = body_with("https://api.openai.com/v1", "long");
        assert_eq!(long["prompt_cache_key"], "conv_abc");
        assert_eq!(long["prompt_cache_retention"], "24h");
        let relay = body_with("https://api.deepseek.com/v1", "long");
        assert_eq!(relay["prompt_cache_key"], "conv_abc");
        assert_eq!(relay["prompt_cache_retention"], "24h");
        // 学习停发 24h 后仍保留 key。
        state
            .provider_runtime()
            .mark_prompt_cache_retention_unsupported("https://api.deepseek.com/v1");
        let after = body_with("https://api.deepseek.com/v1", "long");
        assert_eq!(after["prompt_cache_key"], "conv_abc");
        assert!(after.get("prompt_cache_retention").is_none());
        // 默认 short：OpenAI 与 Anthropic 均启用客户端缓存字段。
        let mut p = ModelProvider {
            id: "x".into(),
            name: "x".into(),
            api_keys: vec!["k".into()],
            api_key_legacy: None,
            base_url: "https://api.openai.com/v1".into(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        assert_eq!(p.request.prompt_cache_retention, "short");
        assert!(p.prompt_caching_enabled());
        p.api_format = "anthropic_messages".into();
        assert!(p.prompt_caching_enabled(), "Anthropic 默认 short 也应启用");
    }

    #[test]
    fn error_rejects_prompt_cache_key_matches_field_name() {
        assert!(super::error_rejects_prompt_cache_key(
            "400 Bad Request - {\"message\":\"Unsupported parameter(s): `prompt_cache_key`\"}"
        ));
        assert!(!super::error_rejects_prompt_cache_key(
            "400 Bad Request - {\"message\":\"some other validation error\"}"
        ));
    }
}
