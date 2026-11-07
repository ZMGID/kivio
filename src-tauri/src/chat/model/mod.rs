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
mod sse;
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
    codex_oauth: bool,
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
            codex_oauth: crate::provider_oauth::is_codex(provider),
            antigravity: crate::provider_oauth::antigravity::is_provider(provider),
            opencode_free: provider.is_opencode_free(),
        }
    }

    // DeepSeek's built-in search uses Responses, or Messages on its Anthropic endpoint.
    fn protocol_for_request(&self, request: &GenerateRequest) -> ProviderApiFormat {
        if request.options.builtin_web_search
            && request.options.tool_choice != ToolChoice::None
            && crate::utils::is_official_deepseek_api(self.base_url)
        {
            if crate::utils::is_official_deepseek_anthropic_api(self.base_url) {
                return ProviderApiFormat::AnthropicMessages;
            }
            if self.api_format == ProviderApiFormat::OpenAiChat {
                return ProviderApiFormat::OpenAiResponses;
            }
        }
        self.api_format
    }
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
    request: GenerateRequest,
    sink: Option<&mut (dyn StreamSink + Send)>,
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    let resolved = crate::provider_oauth::resolve_provider(state, provider)
        .await
        .map_err(ModelError::new)?;
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
    let adapter: &(dyn LanguageModelProvider + Sync) = match config.protocol_for_request(&request) {
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
    async fn unknown_provider_finish_codes_keep_raw_and_legacy_behavior() {
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
                "openai_responses",
                r#"{"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"New_Status"}}}"#,
                FinishKind::Other,
                "incomplete",
                Some("New_Status"),
                "incomplete",
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
                r#"{"candidates":[{"finishReason":"New_Status","content":{"parts":[{"text":"answer"}]}}]}"#,
                FinishKind::Other,
                "New_Status",
                None,
                "new_status",
            ),
            (
                "gemini",
                r#"{"candidates":[{"finishReason":"MAX_TOKENS","content":{"parts":[{"functionCall":{"name":"read","args":{}}}]}}]}"#,
                FinishKind::ToolCalls,
                "MAX_TOKENS",
                None,
                "tool_calls",
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
