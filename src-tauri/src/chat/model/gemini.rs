use super::{ModelTransport, ProviderConfig};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_ENCODING};
use serde_json::Value;

use crate::api::{send_model_with_failover, with_chat_request_timeout};
use crate::provider_oauth::antigravity;
use crate::usage::{
    chat_usage_source_for_label, error_kind_from_message, operation_from_label, UsageRecordInput,
};

use super::{
    FirstTokenStreamSink, GenerateOutput, GenerateRequest, LanguageModelProvider, ModelError,
    ModelFuture, ModelUsage, StreamPart, StreamSink,
};
#[cfg(test)]
use super::{MessagePart, ModelMessage, ModelRole, ModelTool};
pub use kivio_agent_sdk::model::gemini::{
    gemini_contents_from_generate_request, gemini_function_declarations,
};
use kivio_agent_sdk::model::gemini::{
    gemini_url, GeminiOptions, GeminiResponseFormat, GeminiThinkingMode,
};

/// Google Gemini **原生** `generateContent` adapter（peer of openai/anthropic）。
/// 用 Gemini 原生协议，天然不发 OpenAI 专有字段（`promptCacheKey`/`tool_choice`/…），
/// 绕开 Gemini OpenAI-compat 端点对未知 body 字段的 400 严格校验。
/// wire 形状据 opencode 真实流量确认（见任务 research/opencode-real-traffic.md）。
pub(super) struct GeminiProvider<'a> {
    transport: ModelTransport<'a>,
    config: ProviderConfig<'a>,
}

struct DiscardSink;
impl StreamSink for DiscardSink {
    fn emit(&mut self, _part: StreamPart) -> Result<(), ModelError> {
        Ok(())
    }
}

impl<'a> GeminiProvider<'a> {
    pub(super) fn new(transport: ModelTransport<'a>, config: ProviderConfig<'a>) -> Self {
        Self { transport, config }
    }
}

impl LanguageModelProvider for GeminiProvider<'_> {
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

impl GeminiProvider<'_> {
    async fn generate_inner(&self, request: GenerateRequest) -> Result<GenerateOutput, ModelError> {
        if self.config.antigravity {
            return self.stream_inner(request, &mut DiscardSink).await;
        }
        let label = request_label(&request, "Gemini generateContent");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let body = self.request_body(&request, false);
        if crate::chat::video::has_video(&request) {
            crate::chat::video::validate_request(self.config.source, &request)?;
            crate::chat::video::validate_body(&body)?;
        }
        let url = self.endpoint_url(&request.model, false);
        let response = send_model_with_failover(
            self.transport.runtime,
            &label,
            self.transport.retry_attempts,
            self.config.id,
            self.config.api_keys,
            |key| {
                with_chat_request_timeout(crate::api::attach_json_body(
                    crate::provider_request::apply(
                        self.transport
                            .client
                            .post(&url)
                            .headers(self.auth_headers(key))
                            .header(ACCEPT_ENCODING, "identity"),
                        self.config.source,
                        request.metadata.conversation_id.as_deref(),
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
        let output = kivio_agent_sdk::model::gemini::decode_gemini_body(
            &raw,
            &label,
            &request.model,
            Some(self.config.id),
            GeminiResponseFormat::Native,
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
        if self.config.antigravity && request.options.builtin_web_search {
            return Err(ModelError::new("Antigravity does not support built-in Google Search here. Turn off native search or use agent web-search tools."));
        }
        let label = request_label(&request, "Gemini stream");
        let started_at = chrono::Local::now().timestamp();
        let started = std::time::Instant::now();
        let mut measured_sink = FirstTokenStreamSink::new(sink, started);
        let sink = &mut measured_sink;
        let body = self.request_body(&request, true);
        if crate::chat::video::has_video(&request) {
            crate::chat::video::validate_request(self.config.source, &request)?;
            crate::chat::video::validate_body(&body)?;
        }
        let url = self.endpoint_url(&request.model, true);
        let response = send_model_with_failover(
            self.transport.runtime,
            &label,
            self.transport.retry_attempts,
            self.config.id,
            self.config.api_keys,
            |key| {
                crate::api::attach_json_body(
                    crate::provider_request::apply(
                        self.transport
                            .client
                            .post(&url)
                            .headers(self.auth_headers(key))
                            .header(ACCEPT_ENCODING, "identity"),
                        self.config.source,
                        request.metadata.conversation_id.as_deref(),
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

        let output = kivio_agent_sdk::model::gemini::decode_gemini_stream(
            response,
            sink,
            &label,
            &request.model,
            Some(self.config.id),
            if self.config.antigravity {
                GeminiResponseFormat::ResponseEnvelope
            } else {
                GeminiResponseFormat::Native
            },
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

    fn endpoint_url(&self, model: &str, stream: bool) -> String {
        if self.config.antigravity {
            return format!(
                "{}/v1internal:streamGenerateContent?alt=sse",
                antigravity::BASE
            );
        }
        gemini_url(self.config.base_url, model, stream)
    }

    fn auth_headers(&self, key: &str) -> HeaderMap {
        if self.config.antigravity {
            HeaderMap::new()
        } else {
            gemini_headers(key).unwrap_or_default()
        }
    }

    fn request_body(&self, request: &GenerateRequest, _stream: bool) -> Value {
        let thinking = if gemini_supports_thinking_level(&request.model) {
            GeminiThinkingMode::Level
        } else if let Some(budget) = request.options.thinking_level.as_deref().and_then(|level| {
            crate::chat::model_metadata::gemini_thinking_budget(&request.model, level)
        }) {
            GeminiThinkingMode::Budget(budget)
        } else {
            GeminiThinkingMode::IncludeThoughts
        };
        let body = kivio_agent_sdk::model::gemini::request_body(
            request,
            &GeminiOptions {
                temperature: crate::chat::model_metadata::temperature_for_request(
                    request.options.temperature,
                    Some(self.config.source),
                    &request.model,
                ),
                thinking,
                zero_budget_when_disabled: crate::chat::model_metadata::gemini_thinking_profile(
                    &request.model,
                )
                .is_some_and(|profile| profile.supports_off),
                image_output: crate::chat::model_metadata::is_image_output_model(&request.model),
                include_tool_ids: self.config.antigravity
                    && (request.model.starts_with("claude-")
                        || request.model.starts_with("gpt-oss-")),
                provider_id: Some(self.config.id.to_string()),
                ..Default::default()
            },
        );
        if self.config.antigravity {
            antigravity::wrap_request(self.config.source, &request.model, body)
        } else {
            body
        }
    }

    /// 重建本次请求实际会带的 headers（脱敏后）供请求调试面板展示。
    fn debug_request_headers(
        &self,
        metadata: &crate::chat::model::RequestMetadata,
    ) -> std::collections::BTreeMap<String, String> {
        let mut headers = std::collections::BTreeMap::new();
        if let Some(key) = self.config.preferred_api_key {
            if !self.config.antigravity {
                headers.insert("x-goog-api-key".to_string(), key.to_string());
            }
        }
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("Accept-Encoding".to_string(), "identity".to_string());
        for (name, value) in crate::provider_request::header_pairs(
            self.config.source,
            metadata.conversation_id.as_deref(),
        ) {
            headers.insert(name, value);
        }
        crate::chat::request_debug::sanitize_headers(headers)
    }

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
                url: self.endpoint_url(&request.model, stream),
                headers: self.debug_request_headers(&request.metadata),
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
                url: self.endpoint_url(&request.model, stream),
                headers: self.debug_request_headers(&request.metadata),
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

fn gemini_headers(api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    if !api_key.trim().is_empty() {
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_str(api_key).map_err(|err| format!("Invalid API key: {err}"))?,
        );
    }
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    Ok(headers)
}

/// Gemini 3.x+ 才支持 `thinkingConfig.thinkingLevel`（字符串档位）；2.5 及更早只认
/// `thinkingBudget`（数值），传错会 400。保守解析模型名里 `gemini-<major>` 的主版本号，
/// 取不到（未知/非 gemini 命名）则视为不支持，回退仅 `includeThoughts`。
fn gemini_supports_thinking_level(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    let Some(rest) = lower.split("gemini-").nth(1) else {
        return false;
    };
    let major: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    major.parse::<u32>().map(|v| v >= 3).unwrap_or(false)
}

fn request_label(request: &GenerateRequest, fallback: &str) -> String {
    if request.metadata.label.trim().is_empty() {
        fallback.to_string()
    } else {
        request.metadata.label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::model::GenerateOptions;
    use crate::settings::ModelInfo;

    fn provider() -> crate::settings::ModelProvider {
        crate::settings::ModelProvider {
            id: "test".into(),
            name: "Gemini".into(),
            api_keys: vec!["AIza-test".into()],
            api_key_legacy: None,
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            available_models: vec!["gemini-3.1-flash-lite".into()],
            enabled_models: vec!["gemini-3.1-flash-lite".into()],
            enabled: true,
            api_format: "gemini".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        }
    }

    fn body_for(request: &GenerateRequest, stream: bool) -> Value {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let p = provider();
        GeminiProvider::new(
            ModelTransport::for_test(&state, &p),
            ProviderConfig::from_settings(&p),
        )
        .request_body(request, stream)
    }

    #[test]
    fn antigravity_adapter_envelope_headers_and_response() {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut p = provider();
        p.base_url = antigravity::BASE.into();
        p.request.oauth = Some(crate::provider_oauth::OAuthConfig {
            provider: "antigravity".into(),
            credential_id: Some("ref".into()),
            project_id: Some("account-project".into()),
        });
        crate::provider_oauth::validate_provider(&p).unwrap();
        let adapter = GeminiProvider::new(
            ModelTransport::for_test(&state, &p),
            ProviderConfig::from_settings(&p),
        );
        let request = GenerateRequest {
            model: "claude-test-thinking".into(),
            system: "System prompt".into(),
            messages: vec![ModelMessage::text(ModelRole::User, "Hello")],
            tools: vec![],
            options: Default::default(),
            metadata: Default::default(),
        };
        let body = adapter.request_body(&request, true);
        assert_eq!(body["project"], "account-project");
        assert_eq!(body["model"], "claude-test-thinking");
        assert_eq!(
            body["request"]["systemInstruction"]["parts"][0]["text"],
            "System prompt"
        );
        assert!(body["request"]["generationConfig"]
            .get("thinkingConfig")
            .is_none());
        assert!(adapter
            .endpoint_url(&request.model, false)
            .ends_with("/v1internal:streamGenerateContent?alt=sse"));
        assert!(!adapter.auth_headers("token").contains_key("x-goog-api-key"));
        assert!(crate::provider_request::header_pairs(&p, None)
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer AIza-test"));
        let value = serde_json::json!({"response":{
            "candidates":[{"content":{"parts":[{"text":"Hello"},{"functionCall":{"id":"server-call","name":"glob","args":{}},"thoughtSignature":"signature"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":3}
        }});
        let output = kivio_agent_sdk::model::gemini::decode_gemini_body(
            &value.to_string(),
            "Antigravity fixture",
            &request.model,
            Some("test"),
            GeminiResponseFormat::ResponseEnvelope,
        )
        .unwrap();
        assert_eq!(output.tool_calls[0].id, "server-call");
        assert_eq!(
            output.tool_calls[0]
                .signature
                .as_ref()
                .map(|signature| signature.value()),
            Some("signature")
        );
        let saved = serde_json::to_vec(&output.replay_message()).unwrap();
        let mut replay_request = request;
        replay_request
            .messages
            .push(serde_json::from_slice(&saved).unwrap());
        let replay_body = adapter.request_body(&replay_request, true);
        let replay = replay_body["request"]["contents"].as_array().unwrap();
        let call = &replay[1]["parts"][1];
        assert_eq!(call["functionCall"]["id"], "server-call");
        assert_eq!(call["thoughtSignature"], "signature");
        p.base_url = "https://example.com".into();
        assert!(crate::provider_oauth::validate_provider(&p).is_err());
    }

    #[test]
    fn thinking_level_maps_to_thinking_level_field() {
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
        // 无档位但仍开思考 ⇒ 只发 includeThoughts，不编造 thinkingLevel。
        let none = body_for(&base, false);
        assert_eq!(
            none["generationConfig"]["thinkingConfig"]["includeThoughts"], true,
            "body: {none}"
        );
        assert!(
            none["generationConfig"]["thinkingConfig"]
                .get("thinkingLevel")
                .is_none(),
            "body: {none}"
        );
        // 设 medium ⇒ thinkingConfig.thinkingLevel=medium。
        let mut req = base.clone();
        req.options.thinking_level = Some("medium".into());
        let med = body_for(&req, false);
        assert_eq!(
            med["generationConfig"]["thinkingConfig"]["thinkingLevel"], "medium",
            "body: {med}"
        );
        assert_eq!(
            med["generationConfig"]["thinkingConfig"]["includeThoughts"], true,
            "3.x 也必须 opt-in 思考摘要: {med}"
        );
        // 档位原样下发，适配器不再做协议级收敛：Gemini 的 thinkingLevel 只有 low/medium/high，
        // 传 max 就吃 400 —— 该不该出现 max 由模型库 `reasoningEfforts` 门控（Gemini 条目均无
        // 此字段，走 low/medium/high 兜底，故实际取不到 max）。
        let mut req_max = base.clone();
        req_max.options.thinking_level = Some("max".into());
        let mx = body_for(&req_max, false);
        assert_eq!(
            mx["generationConfig"]["thinkingConfig"]["thinkingLevel"], "max",
            "body: {mx}"
        );
        // Gemini 2.5：thinkingLevel 会 400，回退到仅 includeThoughts（非回归）。
        let mut req_25 = base.clone();
        req_25.model = "gemini-2.5-flash".into();
        req_25.options.thinking_level = Some("high".into());
        let g25 = body_for(&req_25, false);
        assert!(
            g25["generationConfig"]["thinkingConfig"]
                .get("thinkingLevel")
                .is_none(),
            "2.5 不应带 thinkingLevel: {g25}"
        );
        assert_eq!(
            g25["generationConfig"]["thinkingConfig"]["includeThoughts"], true,
            "body: {g25}"
        );
    }

    #[test]
    fn gemini_25_off_disables_thinking_and_levels_set_a_budget() {
        let mut req = GenerateRequest {
            model: "gemini-2.5-flash".into(),
            system: String::new(),
            messages: vec![ModelMessage::text(ModelRole::User, "hi")],
            tools: vec![],
            options: GenerateOptions {
                thinking_enabled: false,
                ..Default::default()
            },
            metadata: Default::default(),
        };
        let off = body_for(&req, false);
        assert_eq!(
            off["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            0
        );
        req.options.thinking_enabled = true;
        req.options.thinking_level = Some("high".into());
        let on = body_for(&req, false);
        assert_eq!(
            on["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            24576
        );
        req.model = "gemini-2.5-pro".into();
        let pro = body_for(&req, false);
        assert_eq!(
            pro["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            32768
        );
    }

    fn temperature_body(
        provider_temperature: Option<f64>,
        request_temperature: Option<f64>,
    ) -> Value {
        let state = crate::state::AppState::new_headless(
            crate::settings::Settings::default(),
            std::env::temp_dir(),
        );
        let mut p = provider();
        let model = "custom-temperature-model";
        if let Some(temperature) = provider_temperature {
            p.model_overrides.insert(
                model.into(),
                ModelInfo {
                    temperature: Some(temperature),
                    ..ModelInfo::default()
                },
            );
        }
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
        GeminiProvider::new(
            ModelTransport::for_test(&state, &p),
            ProviderConfig::from_settings(&p),
        )
        .request_body(&request, false)
    }

    #[test]
    fn request_body_omits_temperature_by_default() {
        let body = temperature_body(None, None);
        assert!(
            body["generationConfig"].get("temperature").is_none(),
            "body: {body}"
        );
    }

    #[test]
    fn request_body_uses_provider_temperature_override() {
        let body = temperature_body(Some(0.4), None);
        assert_eq!(
            body["generationConfig"]["temperature"],
            serde_json::json!(0.4),
            "body: {body}"
        );
    }

    #[test]
    fn explicit_request_temperature_wins_over_provider_override() {
        let body = temperature_body(Some(0.4), Some(1.2));
        assert_eq!(
            body["generationConfig"]["temperature"],
            serde_json::json!(1.2),
            "body: {body}"
        );
    }

    #[test]
    fn function_declarations_for_real_present_artifacts_pass_vertex_validation() {
        // 验收标准：真实 present_artifacts 工具定义经 gemini_function_declarations 后，
        // parameters 无 anyOf 且顶层带 type:object（否则 Vertex 拒整个请求）。
        let def = crate::mcp::types::native_present_artifacts_tool();
        let decls = gemini_function_declarations(&[ModelTool::from(&def)]);
        assert_eq!(decls.len(), 1);
        let params = &decls[0]["parameters"];
        assert!(params.get("anyOf").is_none());
        assert_eq!(params["type"], "object");
        assert!(params.get("additionalProperties").is_none());
    }

    fn image_request(model: &str) -> GenerateRequest {
        GenerateRequest {
            model: model.into(),
            system: String::new(),
            messages: vec![ModelMessage {
                role: ModelRole::User,
                content: vec![MessagePart::Text {
                    text: "画一只猫".into(),
                }],
            }],
            tools: Vec::new(),
            options: GenerateOptions::default(),
            metadata: Default::default(),
        }
    }

    #[test]
    fn response_modalities_only_for_image_model() {
        // 文本模型：请求体不含 responseModalities（红线——多余字段会撞 Gemini 400）。
        let text = body_for(&image_request("gemini-3.1-flash-lite"), false);
        assert!(
            text["generationConfig"].get("responseModalities").is_none(),
            "text model must NOT carry responseModalities: {text}"
        );
        // 出图模型：generationConfig.responseModalities = ["TEXT","IMAGE"]。
        let image = body_for(&image_request("gemini-3.1-flash-image"), false);
        assert_eq!(
            image["generationConfig"]["responseModalities"],
            serde_json::json!(["TEXT", "IMAGE"]),
            "image model must carry responseModalities: {image}"
        );
    }
}

#[cfg(test)]
mod interrupted_history_tests {
    use crate::chat::model::generate_request_from_openai_messages;
    #[test]
    fn interrupted_tool_history_keeps_function_response_name() {
        let messages = vec![
            serde_json::json!({"role":"assistant", "tool_calls":[
                {"id":"missing", "type":"function", "function":{"name":"read", "arguments":"{}"}}
            ]}),
            serde_json::json!({"role":"user", "content":"new instruction"}),
            serde_json::json!({"role":"tool", "tool_call_id":"unrelated", "content":"keep"}),
            serde_json::json!({"role":"assistant", "content":"next turn"}),
            serde_json::json!({"role":"tool", "tool_call_id":"missing", "content":"late"}),
        ];
        let request = generate_request_from_openai_messages(
            "m",
            messages,
            None,
            Default::default(),
            "t",
            Default::default(),
        );
        let gemini = crate::chat::model::gemini::gemini_contents_from_generate_request(&request);
        assert_eq!(gemini[1]["parts"][0]["functionResponse"]["name"], "read");
    }
}
