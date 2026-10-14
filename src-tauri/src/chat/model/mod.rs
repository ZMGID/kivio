//! Provider-agnostic Chat model contracts and provider adapters.
//!
//! Runtime code should exchange `GenerateRequest`, `GenerateOutput`, and `StreamPart`.
//! Provider-specific JSON belongs inside this module's adapters.

pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod responses;
mod sse;
pub mod types;

pub use anthropic::AnthropicMessagesProvider;
pub use gemini::GeminiProvider;
pub use openai::OpenAiChatProvider;
pub use responses::OpenAiResponsesProvider;
pub use types::*;

/// 官方 DeepSeek 开内置搜索时的协议跳转：
/// - Chat Completions → Responses 的服务端 `web_search`
/// - 地址已是 `/anthropic` 但协议还标成 Chat Completions → Anthropic Messages
/// 协议本身已是 Responses / Anthropic 时返回 None，走下面的正常分发。
fn official_deepseek_builtin_hop(
    provider: &crate::settings::ModelProvider,
    builtin_web_search: bool,
) -> Option<OfficialDeepseekBuiltinHop> {
    use crate::settings::ProviderApiFormat;
    if !builtin_web_search || !crate::utils::is_official_deepseek_api(&provider.base_url) {
        return None;
    }
    if crate::utils::is_official_deepseek_anthropic_api(&provider.base_url) {
        return match provider.api_format_kind() {
            ProviderApiFormat::AnthropicMessages => None,
            _ => Some(OfficialDeepseekBuiltinHop::Anthropic),
        };
    }
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => Some(OfficialDeepseekBuiltinHop::Responses),
        _ => None,
    }
}

enum OfficialDeepseekBuiltinHop {
    Responses,
    Anthropic,
}

/// 按供应商 `api_format` 分发到对应适配器的非流式调用。全 crate 统一入口：
/// 聊天 planning、以及翻译/截图/Lens 等旧调用路径都应经由这里，而不是各自 match 协议。
pub(crate) async fn generate_with_chat_provider(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    let resolved = crate::provider_oauth::resolve_provider(state, provider)
        .await
        .map_err(ModelError::new)?;
    let provider = &resolved;
    use crate::settings::ProviderApiFormat;
    match official_deepseek_builtin_hop(provider, request.options.builtin_web_search) {
        Some(OfficialDeepseekBuiltinHop::Responses) => {
            return OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await;
        }
        Some(OfficialDeepseekBuiltinHop::Anthropic) => {
            return AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await;
        }
        None => {}
    }
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        // xAI 与 OpenAI 的 Responses 是同一条线协议，共用适配器；差异只在请求体清洗，
        // 由 `responses.rs` 内部按 `api_format_kind()` 分叉。
        ProviderApiFormat::OpenAiResponses | ProviderApiFormat::XaiResponses => {
            OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        ProviderApiFormat::Gemini => {
            GeminiProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
    }
}

/// `generate_with_chat_provider` 的流式版本。同为全 crate 统一分发入口。
pub(crate) async fn stream_with_chat_provider(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
    sink: &mut (dyn StreamSink + Send),
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    let resolved = crate::provider_oauth::resolve_provider(state, provider)
        .await
        .map_err(ModelError::new)?;
    let provider = &resolved;
    use crate::settings::ProviderApiFormat;
    match official_deepseek_builtin_hop(provider, request.options.builtin_web_search) {
        Some(OfficialDeepseekBuiltinHop::Responses) => {
            return OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await;
        }
        Some(OfficialDeepseekBuiltinHop::Anthropic) => {
            return AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await;
        }
        None => {}
    }
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        // xAI 与 OpenAI 的 Responses 是同一条线协议，共用适配器；差异只在请求体清洗，
        // 由 `responses.rs` 内部按 `api_format_kind()` 分叉。
        ProviderApiFormat::OpenAiResponses | ProviderApiFormat::XaiResponses => {
            OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        ProviderApiFormat::Gemini => {
            GeminiProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
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
                            Some((reason.as_str(), full.as_str()))
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
            assert_eq!(output.finish_reason.as_deref(), Some("stop"));
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
        let stream =
            crate::chat::agent::stream::ChatStreamOutput::from_generate_output_with_snapshot(
                output,
                String::new(),
                String::new(),
            );
        let mut messages = model_messages_from_openai_messages(vec![
            stream.to_openai_compatible_message(),
            serde_json::json!({"role":"tool", "tool_call_id":"toolu_1", "content":"found"}),
        ]);
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
        assert!(error.is_stream_read_interrupted());
        let fresh = call(&state, &provider, true).await.unwrap();
        assert_eq!(fresh.text, "anonymous ok");
        assert!(fresh.reasoning.is_none());
        assert!(!fresh
            .to_openai_compatible_message()
            .to_string()
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
    async fn anonymous_model_protocols_propagate_server_auth_failure() {
        for format in FORMATS {
            for (status, code) in [("401 Unauthorized", 401), ("403 Forbidden", 403)] {
                for stream in [false, true] {
                    let (url, server) = endpoint(
                        vec![(
                            status,
                            r#"{"error":{"message":"server requires authentication"}}"#.into(),
                        )],
                        false,
                    )
                    .await;
                    let provider = provider(format, url, Vec::new());
                    let state = crate::state::test_app_state();
                    let error = call(&state, &provider, stream).await.unwrap_err();
                    assert_eq!(crate::api::extract_status_code(&error.message), Some(code));
                    assert!(error.message.contains("server requires authentication"));
                    let captured = server.await.unwrap();
                    assert_eq!(captured.len(), 1, "auth failure must not retry");
                    assert_anonymous(&captured[0]);
                }
            }
        }
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
