//! Provider-agnostic Chat model contracts and provider adapters.
//!
//! Runtime code should exchange `GenerateRequest`, `GenerateOutput`, and `StreamPart`.
//! Provider-specific JSON belongs inside this module's adapters.

pub(crate) mod context_usage;

pub(crate) mod agent_runtime;
pub mod anthropic;
mod error;
pub mod gemini;
mod legacy;
pub mod openai;
pub mod responses;
pub mod types;

use crate::chat::provider_runtime::ProviderRuntimeState;
use crate::chat::request_debug::RequestDebugState;
use crate::settings::{CacheRetention, ModelProvider, ProviderApiFormat};
use crate::state::AppState;
use crate::usage::UsageRecordInput;

use anthropic::AnthropicMessagesProvider;
pub use error::extract_status_code;
pub(crate) use error::is_permanent_provider_error;
pub use error::{is_missing_stream_terminal_error, stream_read_error, ModelError, ModelErrorKind};
use gemini::GeminiProvider;
pub use legacy::{
    generate_request_from_model_messages, generate_request_from_openai_messages,
    split_workbench_system_suffix, WORKBENCH_LOCATION_PROMPT_HEAD,
};
use openai::OpenAiChatProvider;
use responses::OpenAiResponsesProvider;
pub use types::*;

#[derive(Clone, Copy)]
struct ModelTransport<'a> {
    client: &'a reqwest::Client,
    runtime: &'a ProviderRuntimeState,
    retry_attempts: usize,
    request_debug: &'a (dyn Fn() -> Option<&'a RequestDebugState> + Sync),
    record_usage: &'a (dyn Fn(UsageRecordInput<'_>) + Sync),
}

#[cfg(test)]
impl<'a> ModelTransport<'a> {
    fn for_test(state: &'a AppState, provider: &ModelProvider) -> Self {
        Self {
            client: state.client_for(provider),
            runtime: state.provider_runtime(),
            retry_attempts: 1,
            request_debug: &|| None,
            record_usage: &|_| (),
        }
    }
}

#[derive(Clone, Copy)]
struct ProviderConfig<'a> {
    source: &'a ModelProvider,
    id: &'a str,
    base_url: &'a str,
    api_keys: &'a [String],
    preferred_api_key: Option<&'a str>,
    api_format: ProviderApiFormat,
    cache_retention: CacheRetention,
    compress_request_body: bool,
    claude_code: bool,
    codex_identity: bool,
    chatgpt_plan: bool,
    antigravity: bool,
    opencode_free: bool,
}

impl<'a> ProviderConfig<'a> {
    fn from_settings(provider: &'a ModelProvider) -> Self {
        Self {
            source: provider,
            id: &provider.id,
            base_url: &provider.base_url,
            api_keys: &provider.api_keys,
            preferred_api_key: provider.preferred_api_key(),
            api_format: provider.api_format_kind(),
            cache_retention: provider.cache_retention(),
            compress_request_body: provider.compress_request_body,
            claude_code: crate::provider_request::is_claude_code_identity(provider),
            codex_identity: crate::provider_request::is_codex_identity(provider),
            chatgpt_plan: crate::provider_oauth::is_chatgpt(provider),
            antigravity: crate::provider_oauth::antigravity::is_provider(provider),
            opencode_free: provider.is_opencode_free(),
        }
    }
}

fn adapt_request_protocol(
    provider: &mut ModelProvider,
    request: &mut GenerateRequest,
) -> Result<(), ModelError> {
    let deepseek_search = request.options.builtin_web_search
        && request.options.tool_choice != ToolChoice::None
        && crate::utils::is_official_deepseek_api(&provider.base_url);
    if deepseek_search && crate::utils::is_official_deepseek_anthropic_api(&provider.base_url) {
        provider.api_format = ProviderApiFormat::AnthropicMessages.as_str().into();
        return Ok(());
    }
    if provider.api_format_kind() != ProviderApiFormat::OpenAiChat {
        return Ok(());
    }
    let defaults = provider.model_extra_body(&request.model);
    let use_responses = deepseek_search
        || reqwest::Url::parse(&provider.base_url)
            .ok()
            .is_some_and(|endpoint| {
                kivio_agent_sdk::model::openai::chat_requires_responses(
                    request, &endpoint, defaults,
                )
            });
    if use_responses {
        let defaults =
            kivio_agent_sdk::model::responses::translate_chat_route_options(request, defaults)?;
        if let Some(info) = provider.model_overrides.get_mut(&request.model) {
            info.extra_body = Some(defaults);
        }
        provider.api_format = ProviderApiFormat::OpenAiResponses.as_str().into();
    }
    Ok(())
}

pub(crate) async fn generate_with_chat_provider(
    state: &AppState,
    provider: &ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    call_chat_provider(state, provider, retry_attempts, request, None).await
}

pub(crate) async fn stream_with_chat_provider(
    state: &AppState,
    provider: &ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
    sink: &mut (dyn StreamSink + Send),
) -> Result<GenerateOutput, ModelError> {
    call_chat_provider(state, provider, retry_attempts, request, Some(sink)).await
}

pub(crate) async fn generate_via_stream_collect(
    state: &AppState,
    provider: &ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    let mut sink = |_: StreamPart| Ok(());
    stream_with_chat_provider(state, provider, retry_attempts, request, &mut sink).await
}

async fn call_chat_provider(
    state: &AppState,
    provider: &ModelProvider,
    retry_attempts: usize,
    mut request: GenerateRequest,
    sink: Option<&mut (dyn StreamSink + Send)>,
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    let mut resolved = crate::provider_oauth::resolve_provider(state, provider)
        .await
        .map_err(ModelError::new)?;
    adapt_request_protocol(&mut resolved, &mut request)?;
    let config = ProviderConfig::from_settings(&resolved);
    let request_debug = || state.request_debug_enabled().then(|| state.request_debug());
    let record_usage = |input: UsageRecordInput<'_>| crate::usage::record_model_call(state, input);
    let transport = ModelTransport {
        client: state.client_for(&resolved),
        runtime: state.provider_runtime(),
        retry_attempts,
        request_debug: &request_debug,
        record_usage: &record_usage,
    };

    let openai;
    let anthropic;
    let responses;
    let gemini;
    let adapter: &(dyn LanguageModelProvider + Sync) = match config.api_format {
        ProviderApiFormat::OpenAiChat => {
            openai = OpenAiChatProvider::new(transport, config);
            &openai
        }
        ProviderApiFormat::AnthropicMessages => {
            anthropic = AnthropicMessagesProvider::new(transport, config);
            &anthropic
        }
        ProviderApiFormat::OpenAiResponses | ProviderApiFormat::XaiResponses => {
            responses = OpenAiResponsesProvider::new(transport, config);
            &responses
        }
        ProviderApiFormat::Gemini => {
            gemini = GeminiProvider::new(transport, config);
            &gemini
        }
    };
    match sink {
        Some(sink) => adapter.stream(request, sink).await,
        None => adapter.generate(request).await,
    }
}

#[cfg(test)]
mod anonymous_http_tests {
    use super::*;
    use crate::settings::{ModelProvider, ProviderCustomHeader};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const FORMATS: [&str; 5] = [
        "openai_chat",
        "openai_responses",
        "xai_responses",
        "anthropic_messages",
        "gemini",
    ];

    async fn endpoint(
        responses: Vec<(&'static str, String)>,
        hold_open: bool,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut captured = Vec::new();
            for (status, response) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                let header_end = loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "request ended before headers");
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while request.len() < header_end + length {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "request ended before body");
                    request.extend_from_slice(&buffer[..count]);
                }
                let body: serde_json::Value =
                    serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
                assert!(body.is_object());
                captured.push(String::from_utf8(request[..header_end + length].to_vec()).unwrap());
                let content_type = if response.starts_with("data:") {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let wire = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len() + usize::from(hold_open)
                );
                socket.write_all(wire.as_bytes()).await.unwrap();
                if hold_open {
                    // Advertise one more byte but never send it: completion must
                    // release the body without waiting for transport EOF.
                    assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
                }
            }
            captured
        });
        (url, server)
    }

    fn provider(format: &str, url: String, keys: Vec<String>) -> ModelProvider {
        let mut provider = ModelProvider {
            id: "anonymous-http-regression".into(),
            name: "Local model endpoint".into(),
            api_keys: keys,
            api_key_legacy: None,
            base_url: url,
            available_models: vec!["local-model".into()],
            enabled_models: vec!["local-model".into()],
            enabled: true,
            api_format: format.into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        provider.request.use_system_proxy = false;
        provider.request.custom_headers.push(ProviderCustomHeader {
            key: "X-Local-Routing".into(),
            value: "preserved".into(),
        });
        provider
    }

    #[tokio::test]
    async fn chatgpt_plan_replays_local_tool_results_and_requires_completed_inference() {
        use serde_json::{json, Value};

        let call = json!({"type":"function_call","namespace":"kivio","id":"fc_1",
            "name":"read","call_id":"call_1","arguments":"{\"path\":\"a\"}","status":"completed"});
        let event = |event: Value| format!("data: {event}\n\n");
        let done = |output: Value| {
            event(
                json!({"type":"response.completed","response":{"status":"completed","output":output}}),
            )
        };
        let delta = event(json!({"type":"response.output_text.delta","delta":"partial"}));
        let responses = vec![
            (
                "200 OK",
                event(json!({"type":"response.output_item.done","item":call})) + &done(json!([])),
            ),
            (
                "200 OK",
                done(
                    json!([{"type":"message","content":[{"type":"output_text","text":"answer"}]}]),
                ),
            ),
            (
                "200 OK",
                delta.clone()
                    + &event(json!({"type":"response.failed","response":{"error":{
                "code":"subscription_sharing_usage_limit_exceeded","message":"app limit","param":"model"}}})),
            ),
            (
                "200 OK",
                event(
                    json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
                ),
            ),
            ("200 OK", delta),
            (
                "403 Forbidden",
                json!({"detail":"region unavailable"}).to_string(),
            ),
        ];
        let (url, server) = endpoint(responses, false).await;
        let mut provider = provider("openai_responses", url, vec!["test-oauth".into()]);
        provider.request.oauth =
            Some(serde_json::from_value(json!({"provider":"chatgpt"})).unwrap());
        provider.request.cli_identity = "codex".into();
        let state = crate::state::AppState::new_headless(Default::default(), std::env::temp_dir());
        let adapter = OpenAiResponsesProvider::new(
            ModelTransport::for_test(&state, &provider),
            ProviderConfig::from_settings(&provider),
        );
        let mut request = GenerateRequest {
            model: "account-model".into(),
            system: "Read the requested file.".into(),
            messages: vec![ModelMessage::text(ModelRole::User, "read a")],
            tools: vec![ModelTool {
                name: "read".into(),
                description: "Read a file".into(),
                input_schema: json!({"type":"object"}),
            }],
            options: GenerateOptions {
                max_tokens: 64,
                builtin_web_search: true,
                provider_options: json!({"temperature":0.9,"previous_response_id":"stale","metadata":{},"prompt_cache_options":{"ttl":"30m"},"store":true}),
                ..Default::default()
            },
            metadata: RequestMetadata {
                conversation_id: Some("conv_plan".into()),
                ..Default::default()
            },
        };
        let out = adapter
            .stream(request.clone(), &mut |_: StreamPart| Ok(()))
            .await
            .unwrap();
        assert_eq!(out.tool_calls[0].function_name, "read");
        assert_eq!(out.tool_calls[0].arguments, json!({"path":"a"}));
        request.messages.push(out.replay_message());
        request.messages.push(ModelMessage {
            role: ModelRole::Tool,
            content: vec![MessagePart::ToolResult {
                tool_call_id: out.tool_calls[0].id.clone(),
                content: "file contents".into(),
                is_error: false,
                artifacts: vec![],
            }],
        });
        assert_eq!(
            adapter.generate(request.clone()).await.unwrap().text,
            "answer"
        );
        let limit = adapter.generate(request.clone()).await.unwrap_err();
        assert_eq!(
            limit.provider_code.as_deref(),
            Some("subscription_sharing_usage_limit_exceeded")
        );
        assert!(!limit.is_retryable_stream_error());
        for detail in ["app limit", "model", "https://chatgpt.com/settings/usage"] {
            assert!(limit.message.contains(detail));
        }
        assert!(adapter
            .generate(request.clone())
            .await
            .unwrap_err()
            .message
            .contains("max_output_tokens"));
        assert!(adapter
            .generate(request.clone())
            .await
            .unwrap_err()
            .is_retryable_stream_error());
        let admission = adapter.generate(request).await.unwrap_err();
        assert_eq!(admission.status, Some(403));
        assert!(admission.message.contains("region unavailable"));

        let requests = server.await.unwrap();
        for wire in &requests {
            let (headers, json) = wire.split_once("\r\n\r\n").unwrap();
            let headers = headers.to_ascii_lowercase();
            assert!(headers.starts_with("post /v1/responses "));
            assert!(headers.contains("authorization: bearer test-oauth"));
            assert!(headers.contains("accept: text/event-stream"));
            for obsolete in [
                "chatgpt-account-id:",
                "originator:",
                "openai-beta:",
                "x-local-routing:",
                "x-session-id:",
            ] {
                assert!(!headers.contains(obsolete), "{headers}");
            }
            let body: Value = serde_json::from_str(json).unwrap();
            assert_eq!(body["stream"], true);
            assert_eq!(body["store"], false);
            for field in [
                "max_output_tokens",
                "prompt_cache_options",
                "previous_response_id",
                "temperature",
                "metadata",
            ] {
                assert!(body.get(field).is_none());
            }
            assert_eq!(body["tools"][0]["type"], "namespace");
            assert_eq!(body["tools"][0]["tools"][0]["name"], "read");
            assert_eq!(body["tools"][1]["type"], "web_search");
        }
        let next: Value =
            serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(next["input"][0]["role"], "user");
        assert_eq!(next["input"][1]["namespace"], "kivio");
        assert_eq!(next["input"][1]["call_id"], "call_1");
        assert_eq!(
            next["input"][2],
            json!({"type":"function_call_output","call_id":"call_1","output":"file contents"})
        );
    }

    async fn call(
        state: &crate::state::AppState,
        provider: &ModelProvider,
        stream: bool,
    ) -> Result<GenerateOutput, ModelError> {
        call_with_retries(state, provider, stream, 1).await
    }

    async fn call_with_retries(
        state: &crate::state::AppState,
        provider: &ModelProvider,
        stream: bool,
        retry_attempts: usize,
    ) -> Result<GenerateOutput, ModelError> {
        let request = GenerateRequest {
            model: "local-model".into(),
            system: String::new(),
            messages: vec![ModelMessage::text(ModelRole::User, "hello")],
            tools: Vec::new(),
            options: Default::default(),
            metadata: Default::default(),
        };
        struct DiscardSink;
        impl StreamSink for DiscardSink {
            fn emit(&mut self, _: StreamPart) -> Result<(), ModelError> {
                Ok(())
            }
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            if stream {
                stream_with_chat_provider(
                    state,
                    provider,
                    retry_attempts,
                    request,
                    &mut DiscardSink,
                )
                .await
            } else {
                generate_with_chat_provider(state, provider, retry_attempts, request).await
            }
        })
        .await
        .expect("local model request timed out")
    }

    fn success(format: &str, stream: bool) -> String {
        let json = match format {
            "openai_chat" => {
                r#"{"choices":[{"message":{"role":"assistant","content":"anonymous ok"},"finish_reason":"stop"}]}"#
            }
            "openai_responses" | "xai_responses" => {
                r#"{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"anonymous ok"}]}]}"#
            }
            "anthropic_messages" => {
                r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"anonymous ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#
            }
            "gemini" => {
                r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"anonymous ok"}]},"finishReason":"STOP"}]}"#
            }
            _ => unreachable!(),
        };
        if !stream {
            return json.into();
        }
        match format {
            "openai_chat" => "data: {\"choices\":[{\"delta\":{\"content\":\"anonymous ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
            "openai_responses" | "xai_responses" => format!(
                "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"anonymous ok\"}}\n\ndata: {{\"type\":\"response.completed\",\"response\":{json}}}\n\n"
            ),
            "anthropic_messages" => concat!(
                "data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"anonymous ok\"}}\n\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            ).into(),
            "gemini" => format!("data: {json}\n\n"),
            _ => unreachable!(),
        }
    }

    fn finish_response(format: &str, finish: &str, stream: bool) -> String {
        use serde_json::json;
        let tool = finish == "tool_calls";
        let length = finish == "length";
        let body = match format {
            "openai_chat" => json!({"choices": [{
                "message": {"role":"assistant", "content":"answer"},
                "finish_reason": finish
            }]}),
            "openai_responses" | "xai_responses" => json!({
                "status": if length { "incomplete" } else { "completed" },
                "incomplete_details": if length { json!({"reason":"max_output_tokens"}) } else { serde_json::Value::Null },
                "output": if tool {
                    vec![json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"read","arguments":"{}"})]
                } else {
                    vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]})]
                }
            }),
            "anthropic_messages" => json!({
                "role":"assistant",
                "content":[{"type":"text","text":"answer"}],
                "stop_reason": if tool { "tool_use" } else if length { "max_tokens" } else { "end_turn" },
                "usage":{"input_tokens":1,"output_tokens":1}
            }),
            "gemini" => json!({
                "candidates":[{"content":{"role":"model","parts": if tool {
                    vec![json!({"functionCall":{"name":"read","args":{}}})]
                } else { vec![json!({"text":"answer"})] }},
                "finishReason": if length { "MAX_TOKENS" } else { "STOP" }}]
            }),
            _ => unreachable!(),
        };
        if !stream {
            return body.to_string();
        }
        let events = match format {
            "openai_chat" => {
                vec![json!({"choices":[{"delta":{"content":"answer"},"finish_reason":finish}]})]
            }
            "openai_responses" | "xai_responses" => {
                let mut events =
                    vec![json!({"type":"response.output_text.delta","delta":"answer"})];
                if tool {
                    events.push(json!({"type":"response.output_item.added","output_index":0,
                        "item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"read","arguments":"{}"}}));
                    events.push(json!({"type":"response.output_item.done","output_index":0,
                        "item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"read","arguments":"{}"}}));
                }
                events.push(json!({"type":if length { "response.incomplete" } else { "response.completed" },"response":body}));
                events
            }
            "anthropic_messages" => vec![
                json!({"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"answer"}}),
                json!({"type":"message_delta","delta":{"stop_reason":body["stop_reason"]},"usage":{"output_tokens":1}}),
                json!({"type":"message_stop"}),
            ],
            "gemini" => vec![body],
            _ => unreachable!(),
        };
        events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    #[tokio::test]
    async fn no_tools_and_finish_reason_survive_provider_wire_and_message_runtime() {
        use crate::chat::agent::history::RuntimeMessage;
        use crate::chat::agent::provider_runtime::{
            DiscardModelSink, ModelCall, ModelDelivery, ModelRuntime,
        };
        for format in FORMATS {
            for stream in [false, true] {
                for finish in ["stop", "length", "tool_calls"] {
                    let (url, server) = endpoint(
                        vec![("200 OK", finish_response(format, finish, stream))],
                        false,
                    )
                    .await;
                    let state = crate::state::AppState::new_headless(
                        crate::settings::Settings::default(),
                        std::env::temp_dir(),
                    );
                    let provider = provider(format, url, Vec::new());
                    let output = if stream {
                        agent_runtime::AppAgentModel::new(&state, &provider, 1)
                            .generate(
                                ModelCall {
                                    capabilities: None,
                                    provider_id: provider.id.clone(),
                                    model: "local-model".into(),
                                    history: vec![RuntimeMessage::text(ModelRole::User, "hello")],
                                    tools: Vec::new(),
                                    options: GenerateOptions {
                                        tool_choice: ToolChoice::None,
                                        thinking_enabled: false,
                                        builtin_web_search: true,
                                        max_tokens: 100,
                                        ..Default::default()
                                    },
                                    metadata: RequestMetadata {
                                        conversation_id: Some("conv".into()),
                                        message_id: Some("msg".into()),
                                        label: "finish regression".into(),
                                        ..Default::default()
                                    },
                                    delivery: ModelDelivery::Collected,
                                },
                                &mut DiscardModelSink,
                            )
                            .await
                            .unwrap()
                    } else {
                        let request = GenerateRequest {
                            model: "local-model".into(),
                            system: String::new(),
                            messages: vec![ModelMessage::text(ModelRole::User, "hello")],
                            tools: vec![ModelTool {
                                name: "read".into(),
                                description: String::new(),
                                input_schema: serde_json::json!({"type":"object"}),
                            }],
                            options: GenerateOptions {
                                tool_choice: ToolChoice::None,
                                builtin_web_search: true,
                                provider_options: serde_json::json!({
                                    "tools":[{"type":"web_search"}], "tool_choice":"auto",
                                    "toolConfig":{"functionCallingConfig":{"mode":"AUTO"}}
                                }),
                                ..Default::default()
                            },
                            metadata: Default::default(),
                        };
                        generate_with_chat_provider(&state, &provider, 1, request)
                            .await
                            .unwrap()
                    };
                    assert_eq!(
                        output.finish_reason.as_ref().unwrap().legacy_name(),
                        finish,
                        "{format} stream={stream}"
                    );
                    let captured = server.await.unwrap();
                    let (_, body) = captured[0].split_once("\r\n\r\n").unwrap();
                    let body: serde_json::Value = serde_json::from_str(body).unwrap();
                    assert!(
                        body.get("tools").is_none(),
                        "{format} stream={stream}: {body}"
                    );
                    match format {
                        "anthropic_messages" => assert_eq!(body["tool_choice"]["type"], "none"),
                        "gemini" => {
                            assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "NONE")
                        }
                        _ => assert_eq!(body["tool_choice"], "none"),
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn message_runtime_preserves_provider_errors() {
        use crate::chat::agent::history::RuntimeMessage;
        use crate::chat::agent::provider_runtime::{
            DiscardModelSink, ModelCall, ModelDelivery, ModelRuntime,
        };
        let (url, server) = endpoint(
            vec![(
                "401 Unauthorized",
                r#"{"error":{"message":"server requires authentication"}}"#.into(),
            )],
            false,
        )
        .await;
        let state = crate::state::test_app_state();
        let provider = provider("openai_chat", url, Vec::new());
        let result = agent_runtime::AppAgentModel::new(&state, &provider, 1)
            .generate(
                ModelCall {
                    capabilities: None,
                    provider_id: provider.id.clone(),
                    model: "local-model".into(),
                    history: vec![RuntimeMessage::text(ModelRole::User, "hello")],
                    tools: Vec::new(),
                    options: GenerateOptions {
                        tool_choice: ToolChoice::None,
                        thinking_enabled: false,
                        max_tokens: 100,
                        ..Default::default()
                    },
                    metadata: RequestMetadata {
                        conversation_id: Some("conv".into()),
                        message_id: Some("msg".into()),
                        label: "error regression".into(),
                        ..Default::default()
                    },
                    delivery: ModelDelivery::Collected,
                },
                &mut DiscardModelSink,
            )
            .await;
        let error = result.unwrap_err();
        assert_eq!(error.kind, ModelErrorKind::Auth);
        assert!(error.message.contains("server requires authentication"));
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn developer_delivery_survives_replay_and_provider_wire() {
        let delivery = "<async-result>worker output &lt;unsafe&gt;</async-result>";
        let history = vec![
            serde_json::json!({"role":"system", "content":"application policy"}),
            serde_json::json!({"role":"user", "content":"start"}),
            serde_json::json!({"role":"assistant", "content":"working"}),
            serde_json::json!({"role":"developer", "content":delivery}),
        ];
        let request = generate_request_from_openai_messages(
            "local-model",
            history,
            None,
            GenerateOptions::default(),
            "delivery",
            GenerateRequestContext::default(),
        );
        assert_eq!(request.messages[2].role, ModelRole::Developer);
        let stored = serde_json::to_vec(&request.messages).unwrap();
        let replayed: Vec<ModelMessage> = serde_json::from_slice(&stored).unwrap();
        assert_eq!(replayed[2].role, ModelRole::Developer);
        assert_eq!(
            openai_messages_from_model_messages(&replayed)[2]["role"],
            "developer"
        );

        for format in FORMATS {
            let (url, server) = endpoint(vec![("200 OK", success(format, false))], false).await;
            let state = crate::state::test_app_state();
            let provider = provider(format, url, Vec::new());
            generate_with_chat_provider(&state, &provider, 1, request.clone())
                .await
                .unwrap();
            let captured = server.await.unwrap();
            let (_, body) = captured[0].split_once("\r\n\r\n").unwrap();
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            match format {
                "openai_chat" => {
                    assert_eq!(body["messages"][3]["role"], "developer");
                    assert_eq!(body["messages"][3]["content"], delivery);
                }
                "openai_responses" | "xai_responses" => {
                    let item = body["input"].as_array().unwrap().last().unwrap();
                    assert_eq!(item["role"], "developer");
                    assert_eq!(item["content"][0]["text"], delivery);
                }
                "anthropic_messages" => {
                    assert_eq!(body["messages"][2]["role"], "user");
                    assert_eq!(body["messages"][2]["content"][0]["text"], delivery);
                }
                "gemini" => {
                    assert_eq!(body["contents"][2]["role"], "user");
                    assert_eq!(body["contents"][2]["parts"][0]["text"], delivery);
                }
                _ => unreachable!(),
            }
        }
    }

    fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
        headers.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    fn assert_anonymous(headers: &str) {
        for name in ["authorization", "x-api-key", "x-goog-api-key"] {
            assert_eq!(header(headers, name), None, "{headers}");
        }
        assert_eq!(header(headers, "x-local-routing"), Some("preserved"));
    }

    #[tokio::test]
    async fn anthropic_server_pause_continues_native_history_and_accumulates_usage() {
        let before = serde_json::json!([
            {"type":"thinking", "thinking":"plan", "signature":"opaque-signature"},
            {"type":"server_tool_use", "id":"srvtoolu_search", "name":"web_search", "input":{"query":"fixture"}},
            {"type":"web_search_tool_result", "tool_use_id":"srvtoolu_search", "content":[]},
            {"type":"text", "text":"Before "}
        ]);
        let after = serde_json::json!([{"type":"text", "text":"After"}]);
        for streaming in [false, true] {
            let response = |blocks: &serde_json::Value, reason: &str, input: u64, output: u64| {
                if !streaming {
                    return serde_json::json!({"content":blocks, "stop_reason":reason, "usage":{"input_tokens":input, "output_tokens":output}}).to_string();
                }
                let mut events = vec![
                    serde_json::json!({"type":"message_start", "message":{"usage":{"input_tokens":input, "output_tokens":0}}}),
                ];
                for (index, block) in blocks.as_array().unwrap().iter().enumerate() {
                    let mut start = block.clone();
                    if block["type"] == "text" {
                        start["text"] = serde_json::json!("");
                    }
                    events.push(serde_json::json!({"type":"content_block_start", "index":index, "content_block":start}));
                    if block["type"] == "text" {
                        events.push(serde_json::json!({"type":"content_block_delta", "index":index, "delta":{"type":"text_delta", "text":block["text"]}}));
                    }
                    events.push(serde_json::json!({"type":"content_block_stop", "index":index}));
                }
                events.push(serde_json::json!({"type":"message_delta", "delta":{"stop_reason":reason}, "usage":{"output_tokens":output}}));
                events
                    .iter()
                    .map(|event| format!("data: {event}\n\n"))
                    .collect::<String>()
            };
            let (url, server) = endpoint(
                vec![
                    ("200 OK", response(&before, "pause_turn", 3, 1)),
                    ("200 OK", response(&after, "end_turn", 5, 2)),
                ],
                false,
            )
            .await;
            let provider = provider("anthropic_messages", url, Vec::new());
            let state = crate::state::test_app_state();
            #[derive(Default)]
            struct RecordingSink(Vec<StreamPart>);
            impl StreamSink for RecordingSink {
                fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
                    self.0.push(part);
                    Ok(())
                }
            }
            let mut sink = RecordingSink::default();
            let output = if streaming {
                let request = GenerateRequest {
                    model: "local-model".into(),
                    system: String::new(),
                    messages: vec![ModelMessage::text(ModelRole::User, "hello")],
                    tools: Vec::new(),
                    options: Default::default(),
                    metadata: Default::default(),
                };
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    stream_with_chat_provider(&state, &provider, 1, request, &mut sink),
                )
                .await
                .expect("local continuation timed out")
                .unwrap()
            } else {
                call(&state, &provider, false).await.unwrap()
            };
            if streaming {
                let finishes: Vec<_> = sink
                    .0
                    .iter()
                    .filter_map(|part| match part {
                        StreamPart::Finish { reason, full } => {
                            Some((reason.legacy_name(), full.as_str()))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(finishes, vec![("stop", "Before After")]);
                let usage: Vec<_> = sink
                    .0
                    .iter()
                    .filter_map(|part| match part {
                        StreamPart::ContextUsage {
                            input_tokens,
                            output_tokens,
                        } => Some((*input_tokens, *output_tokens)),
                        _ => None,
                    })
                    .collect();
                assert!(usage.contains(&(3, 1)));
                assert_eq!(usage.last(), Some(&(5, 2)));
                assert!(
                    !usage.contains(&(8, 3)),
                    "billing must not become live context usage"
                );
            }
            assert_eq!(output.text, "Before After");
            assert_eq!(
                output.finish_reason.as_ref().map(FinishReason::legacy_name),
                Some("stop")
            );
            assert_eq!(output.usage.as_ref().unwrap().input_tokens, Some(5));
            assert_eq!(output.usage.as_ref().unwrap().output_tokens, Some(2));
            assert_eq!(output.billing_usage.as_ref().unwrap().input_tokens, Some(8));
            assert_eq!(
                output.billing_usage.as_ref().unwrap().output_tokens,
                Some(3)
            );
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2);
            let second: serde_json::Value =
                serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(second["messages"][1]["content"], before);
            assert_eq!(output.canonical_message.as_ref().unwrap().content.len(), 5);
        }
    }

    #[tokio::test]
    async fn anthropic_stream_signatures_reach_the_next_tool_round_after_reload() {
        let expected = serde_json::json!([
            {"type":"thinking", "thinking":" plan ", "signature":"sig-part-one"},
            {"type":"redacted_thinking", "data":"opaque-redacted"},
            {"type":"thinking", "thinking":"", "signature":"sig-empty"},
            {"type":"text", "text":"Checking", "citations":[{"type":"web_search_result_location", "url":"https://example.test/source", "title":"Source", "encrypted_index":"opaque-citation"}]},
            {"type":"tool_use", "id":"toolu_1", "name":"read", "input":{"path":"a"}}
        ]);
        let mut events = Vec::new();
        for (index, block) in expected.as_array().unwrap().iter().enumerate() {
            let mut start = block.clone();
            match block["type"].as_str().unwrap() {
                "thinking" => {
                    start["thinking"] = serde_json::json!("");
                    start["signature"] = serde_json::json!("");
                }
                "text" => {
                    start["text"] = serde_json::json!("");
                    start["citations"] = serde_json::json!([]);
                }
                "tool_use" => start["input"] = serde_json::json!({}),
                _ => {}
            }
            events.push(serde_json::json!({"type":"content_block_start", "index":index, "content_block":start}));
            let deltas = match block["type"].as_str().unwrap() {
                "thinking" => vec![
                    serde_json::json!({"type":"thinking_delta", "thinking":block["thinking"]}),
                    serde_json::json!({"type":"signature_delta", "signature":"sig-"}),
                    serde_json::json!({"type":"signature_delta", "signature":block["signature"].as_str().unwrap().strip_prefix("sig-").unwrap()}),
                ],
                "text" => vec![
                    serde_json::json!({"type":"text_delta", "text":block["text"]}),
                    serde_json::json!({"type":"citations_delta", "citation":block["citations"][0]}),
                ],
                "tool_use" => vec![
                    serde_json::json!({"type":"input_json_delta", "partial_json":block["input"].to_string()}),
                ],
                _ => Vec::new(),
            };
            for delta in deltas {
                events.push(
                    serde_json::json!({"type":"content_block_delta", "index":index, "delta":delta}),
                );
            }
            events.push(serde_json::json!({"type":"content_block_stop", "index":index}));
        }
        events
            .push(serde_json::json!({"type":"message_delta", "delta":{"stop_reason":"tool_use"}}));
        let body = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        let (url, server) = endpoint(
            vec![
                ("200 OK", body),
                ("200 OK", success("anthropic_messages", false)),
                ("200 OK", success("anthropic_messages", false)),
                ("200 OK", success("anthropic_messages", false)),
            ],
            false,
        )
        .await;
        let provider = provider("anthropic_messages", url, Vec::new());
        let state = crate::state::test_app_state();
        let output = call(&state, &provider, true).await.unwrap();
        let mut messages = vec![
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
        messages = serde_json::from_slice(&serde_json::to_vec(&messages).unwrap()).unwrap();
        generate_with_chat_provider(
            &state,
            &provider,
            1,
            GenerateRequest {
                model: "local-model".into(),
                system: String::new(),
                messages: messages.clone(),
                tools: Vec::new(),
                options: Default::default(),
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();
        for (model, provider_id) in [
            ("changed-model", provider.id.as_str()),
            ("local-model", "changed-provider"),
        ] {
            let mut changed_provider = provider.clone();
            changed_provider.id = provider_id.to_string();
            generate_with_chat_provider(
                &state,
                &changed_provider,
                1,
                GenerateRequest {
                    model: model.to_string(),
                    system: String::new(),
                    messages: messages.clone(),
                    tools: Vec::new(),
                    options: Default::default(),
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap();
        }
        let captured = server.await.unwrap();
        let (_, body) = captured[1].split_once("\r\n\r\n").unwrap();
        let request: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(request["messages"][0]["content"], expected);
        assert_eq!(
            request["messages"][1]["content"][0]["tool_use_id"],
            "toolu_1"
        );
        for captured in &captured[2..] {
            let (_, body) = captured.split_once("\r\n\r\n").unwrap();
            let request: serde_json::Value = serde_json::from_str(body).unwrap();
            assert!(request["messages"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|block| !matches!(
                    block["type"].as_str(),
                    Some("thinking" | "redacted_thinking")
                )));
        }
    }

    #[tokio::test]
    async fn interrupted_anthropic_thinking_is_not_a_replayable_completed_output() {
        let partial = concat!(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"unfinished\"}}\n\n"
        );
        let (url, server) = endpoint(
            vec![
                ("200 OK", partial.into()),
                ("200 OK", success("anthropic_messages", true)),
            ],
            false,
        )
        .await;
        let provider = provider("anthropic_messages", url, Vec::new());
        let state = crate::state::test_app_state();
        let error = call(&state, &provider, true).await.unwrap_err();
        assert!(error.is_retryable_stream_error());
        let fresh = call(&state, &provider, true).await.unwrap();
        assert_eq!(fresh.text, "anonymous ok");
        assert!(fresh.reasoning.is_none());
        assert!(!serde_json::to_string(&fresh.replay_message())
            .unwrap()
            .contains("unfinished"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn model_streams_complete_multiline_events_without_transport_eof() {
        for format in FORMATS {
            let wire = if format == "openai_chat" {
                success(format, true).replace("data: [DONE]", "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2,\"total_tokens\":9}}\n\ndata: [DONE]")
            } else {
                success(format, true)
            };
            let body = wire
                .lines()
                .map(|line| {
                    if let Some((prefix, suffix)) = line.split_once(',') {
                        format!("{prefix},\r\ndata: {suffix}\r\n")
                    } else {
                        format!("{line}\r\n")
                    }
                })
                .collect::<String>();
            let (url, server) = endpoint(vec![("200 OK", body)], true).await;
            let provider = provider(format, url, Vec::new());
            let state = crate::state::test_app_state();
            let output = call(&state, &provider, true).await.unwrap();
            assert_eq!(output.text, "anonymous ok", "{format}");
            if format == "openai_chat" {
                assert_eq!(
                    output.usage.as_ref().and_then(|usage| usage.input_tokens),
                    Some(7)
                );
            }
            tokio::time::timeout(std::time::Duration::from_secs(5), server)
                .await
                .expect("terminal model response must close the pending body")
                .unwrap();
        }
    }

    #[test]
    fn official_chat_routes_function_tools_and_their_options_to_responses() {
        use serde_json::json;
        for (base, model, search, routed) in [
            ("https://api.openai.com/v1", "gpt-6.2-sol", false, true),
            ("https://api.deepseek.com/v1", "deepseek-v4-pro", true, true),
            ("https://gateway.example/v1", "gpt-6.2-sol", false, false),
        ] {
            let mut provider = provider("openai_chat", base.into(), Vec::new());
            let mut request = GenerateRequest {
                model: model.into(),
                system: String::new(),
                messages: vec![ModelMessage::text(ModelRole::User, "Look this up.")],
                tools: vec![ModelTool {
                    name: "lookup".into(),
                    description: "Look up a value".into(),
                    input_schema: json!({"type":"object"}),
                }],
                options: GenerateOptions {
                    builtin_web_search: search,
                    provider_options: json!({"tool_choice":{"type":"function","function":{"name":"lookup"}}}),
                    ..Default::default()
                },
                metadata: Default::default(),
            };
            adapt_request_protocol(&mut provider, &mut request).unwrap();
            let config = ProviderConfig::from_settings(&provider);
            if routed {
                assert_eq!(config.api_format, ProviderApiFormat::OpenAiResponses);
                assert_eq!(
                    request.options.provider_options["tool_choice"],
                    json!({"type":"function","name":"lookup"})
                );
            } else {
                assert_eq!(config.api_format, ProviderApiFormat::OpenAiChat);
                assert_eq!(
                    request.options.provider_options["tool_choice"]["function"]["name"],
                    "lookup"
                );
            }
        }
    }

    #[tokio::test]
    async fn model_body_defaults_reach_each_endpoint_without_changing_delivery() {
        use serde_json::{json, Value};
        let state = crate::state::test_app_state();
        for format in FORMATS {
            for stream in [false, true] {
                let (url, server) =
                    endpoint(vec![("200 OK", success(format, stream))], false).await;
                let mut provider = provider(format, url, Vec::new());
                provider.model_overrides.insert("local-model".into(), crate::settings::ModelInfo {
                    extra_body: Some(json!({"vendor_option": {"mode":"custom"}, "model":"wrong-route", "stream": !stream, "stream_options": {"include_usage":true}})),
                    ..Default::default()
                });
                let output = call(&state, &provider, stream).await.unwrap();
                assert_eq!(output.text, "anonymous ok");
                let captured = server.await.unwrap();
                let (headers, payload) = captured[0].split_once("\r\n\r\n").unwrap();
                let body: Value = serde_json::from_str(payload).unwrap();
                assert_eq!(body["vendor_option"], json!({"mode":"custom"}), "{format}");
                if !stream || matches!(format, "gemini" | "anthropic_messages" | "xai_responses") {
                    assert!(body.get("stream_options").is_none(), "{format}");
                }
                if format == "gemini" {
                    assert!(body.get("model").is_none());
                    assert!(body.get("stream").is_none());
                    assert_eq!(
                        headers
                            .lines()
                            .next()
                            .unwrap()
                            .contains(":streamGenerateContent"),
                        stream
                    );
                } else {
                    assert_eq!(body["model"], "local-model");
                    assert_eq!(
                        body["stream"].as_bool().unwrap_or(false),
                        stream,
                        "{format}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn anonymous_model_protocols_reach_endpoint_and_return_response() {
        for format in FORMATS {
            for keys in [Vec::new(), vec!["".into(), " \t ".into()]] {
                for stream in [false, true] {
                    let (url, server) =
                        endpoint(vec![("200 OK", success(format, stream))], false).await;
                    let provider = provider(format, url, keys.clone());
                    let state = crate::state::test_app_state();
                    let output = call(&state, &provider, stream).await.unwrap();
                    assert_eq!(output.text, "anonymous ok", "{format}, stream={stream}");
                    let captured = server.await.unwrap();
                    assert_eq!(captured.len(), 1);
                    assert_anonymous(&captured[0]);
                }
            }
        }
    }

    #[tokio::test]
    async fn provider_terminal_outcomes_preserve_codes_without_masking_failures() {
        let cases = [
            (
                "openai_chat",
                r#"{"choices":[{"delta":{"content":"answer"},"finish_reason":"New_Status"}]}"#,
                FinishKind::Other,
                "New_Status",
                None,
                "New_Status",
            ),
            (
                "anthropic_messages",
                r#"{"type":"message_delta","delta":{"stop_reason":"New_Status"}}"#,
                FinishKind::Stop,
                "New_Status",
                None,
                "stop",
            ),
            (
                "gemini",
                r#"{"candidates":[{"finishReason":"MAX_TOKENS","content":{"parts":[{"functionCall":{"name":"read","args":{}}}]}}]}"#,
                FinishKind::Length,
                "MAX_TOKENS",
                None,
                "length",
            ),
        ];
        for (format, event, kind, raw, detail, legacy) in cases {
            let (url, server) =
                endpoint(vec![("200 OK", format!("data: {event}\n\n"))], false).await;
            let provider = provider(format, url, Vec::new());
            let state = crate::state::test_app_state();
            let output = call(&state, &provider, true).await.unwrap();
            let finish = output.finish_reason.as_ref().unwrap();
            assert_eq!(
                (finish.kind, finish.raw.as_str(), finish.detail.as_deref()),
                (kind, raw, detail),
                "{format}"
            );
            assert_eq!(finish.legacy_name(), legacy, "{format}");
            server.await.unwrap();
        }
        for (format, event) in [
            (
                "openai_responses",
                r#"{"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"New_Status"}}}"#,
            ),
            (
                "gemini",
                r#"{"candidates":[{"finishReason":"New_Status","content":{"parts":[{"text":"answer"}]}}]}"#,
            ),
        ] {
            let (url, server) =
                endpoint(vec![("200 OK", format!("data: {event}\n\n"))], false).await;
            let provider = provider(format, url, Vec::new());
            let state = crate::state::test_app_state();
            let error = call(&state, &provider, true).await.unwrap_err();
            assert_eq!(error.kind, ModelErrorKind::Other);
            assert_eq!(error.provider_code.as_deref(), Some("New_Status"));
            assert!(!error.is_retryable_stream_error());
            assert_eq!(
                server.await.unwrap().len(),
                1,
                "terminal error must not retry"
            );
        }
    }

    #[tokio::test]
    async fn anonymous_model_protocols_propagate_server_auth_failure() {
        for format in FORMATS {
            for (status, code) in [("401 Unauthorized", 401), ("403 Forbidden", 403)] {
                for stream in [false, true] {
                    let (url, server) = endpoint(
                        vec![(
                            status,
                            r#"{"error":{"message":"server requires authentication","code":"auth_required"}}"#.into(),
                        )],
                        false,
                    )
                    .await;
                    let provider = provider(format, url, Vec::new());
                    let state = crate::state::test_app_state();
                    let error = call(&state, &provider, stream).await.unwrap_err();
                    assert_eq!(error.status, Some(code));
                    assert_eq!(error.kind, ModelErrorKind::Auth);
                    assert_eq!(error.provider_code.as_deref(), Some("auth_required"));
                    assert!(!error.is_retryable_stream_error());
                    assert!(error.message.contains("server requires authentication"));
                    let captured = server.await.unwrap();
                    assert_eq!(captured.len(), 1, "auth failure must not retry");
                    assert_anonymous(&captured[0]);
                }
            }
        }
    }

    #[tokio::test]
    async fn responses_retry_records_the_body_that_succeeded_and_limits_learning_to_one_model() {
        use serde_json::{json, Value};
        let temp = tempfile::tempdir().unwrap();
        let mut settings = crate::settings::Settings::default();
        settings.chat_tools.request_debug_enabled = true;
        let state = AppState::new_headless(settings, temp.path().join("usage"));
        let (url, server) = endpoint(
            vec![
                (
                    "400 Bad Request",
                    json!({"error":{"message":"unsupported reasoning item"}}).to_string(),
                ),
                ("200 OK", success("openai_responses", false)),
            ],
            false,
        )
        .await;
        let provider = provider("openai_responses", url.clone(), Vec::new());
        let request = GenerateRequest {
            model: "local-model".into(),
            system: String::new(),
            messages: vec![
                ModelMessage {
                    role: ModelRole::Assistant,
                    content: vec![MessagePart::ReasoningItem {
                        model: "local-model".into(),
                        source: None,
                        item: json!({"type":"reasoning","id":"rs_old","encrypted_content":"old","summary":[]}),
                    }],
                },
                ModelMessage::text(ModelRole::User, "Continue"),
            ],
            tools: vec![],
            options: Default::default(),
            metadata: Default::default(),
        };
        let output = generate_with_chat_provider(&state, &provider, 1, request)
            .await
            .unwrap();
        assert_eq!(output.text, "anonymous ok");
        let bodies: Vec<Value> = server
            .await
            .unwrap()
            .iter()
            .map(|wire| serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap())
            .collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["input"][0]["type"], "reasoning");
        assert_eq!(bodies[1]["input"].as_array().unwrap().len(), 1);
        assert_eq!(
            crate::chat::request_debug::snapshot(&state)[0].request.body,
            bodies[1]
        );
        assert!(state
            .provider_runtime()
            .reasoning_replay_unsupported(&url, "local-model"));
        assert!(!state
            .provider_runtime()
            .reasoning_replay_unsupported(&url, "other-model"));
    }

    #[tokio::test]
    async fn model_protocols_use_selected_client_and_record_provider_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = crate::settings::Settings::default();
        settings.chat_tools.request_debug_enabled = true;
        let mut state = AppState::new_headless(settings, temp.path().join("usage"));
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-client-route",
            reqwest::header::HeaderValue::from_static("shared"),
        );
        state.http = reqwest::Client::builder()
            .no_proxy()
            .default_headers(headers)
            .build()
            .unwrap();

        for format in FORMATS {
            for use_system_proxy in [true, false] {
                let (url, server) = endpoint(vec![("200 OK", success(format, false))], false).await;
                let mut provider = provider(format, url, Vec::new());
                provider.request.use_system_proxy = use_system_proxy;
                let output = call(&state, &provider, false).await.unwrap();
                assert_eq!(output.text, "anonymous ok");
                let captured = server.await.unwrap();
                assert_eq!(
                    header(&captured[0], "x-client-route"),
                    use_system_proxy.then_some("shared")
                );
                assert_eq!(header(&captured[0], "x-local-routing"), Some("preserved"));

                let records = crate::chat::request_debug::snapshot(&state);
                let record = records.first().unwrap();
                assert_eq!(record.provider_id, provider.id);
                assert_eq!(record.provider_name, provider.name);
                assert_eq!(record.api_format, format);
            }
        }
        let usage = crate::usage::today_usage(&state.usage_dir).unwrap();
        assert_eq!(usage.summary.total_requests as usize, FORMATS.len() * 2);
    }

    #[tokio::test]
    async fn model_protocols_keep_nonempty_key_auth_and_failover() {
        for format in FORMATS {
            let (url, server) = endpoint(
                vec![
                    (
                        "401 Unauthorized",
                        r#"{"error":{"message":"invalid key"}}"#.into(),
                    ),
                    ("200 OK", success(format, false)),
                ],
                false,
            )
            .await;
            let provider = provider(format, url, vec!["invalid-key".into(), "valid-key".into()]);
            let state = crate::state::test_app_state();
            // Unified RetryBudget counts key switches as retries; attempts=1 cannot
            // leave room to try the backup key after a 401.
            assert_eq!(
                call_with_retries(&state, &provider, false, 2)
                    .await
                    .unwrap()
                    .text,
                "anonymous ok"
            );
            let captured = server.await.unwrap();
            assert_eq!(captured.len(), 2);
            for (headers, key) in captured.iter().zip(["invalid-key", "valid-key"]) {
                let (name, expected) = match format {
                    "anthropic_messages" => ("x-api-key", key.into()),
                    "gemini" => ("x-goog-api-key", key.into()),
                    _ => ("authorization", format!("Bearer {key}")),
                };
                assert_eq!(header(headers, name), Some(expected.as_str()));
                assert_eq!(header(headers, "x-local-routing"), Some("preserved"));
            }
        }
    }
}
