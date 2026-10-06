//! Provider-agnostic Chat model contracts and provider adapters.
//!
//! Runtime code should exchange `GenerateRequest`, `GenerateOutput`, and `StreamPart`.
//! Provider-specific JSON belongs inside this module's adapters.

pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod responses;
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
                captured.push(headers);
                let content_type = if response.starts_with("data:") {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let wire = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                socket.write_all(wire.as_bytes()).await.unwrap();
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
                stream_with_chat_provider(state, provider, 1, request, &mut DiscardSink).await
            } else {
                generate_with_chat_provider(state, provider, 1, request).await
            }
        })
        .await
        .expect("local model request timed out")
    }

    fn success(format: &str, stream: bool) -> String {
        let json = match format {
            "openai_chat" => r#"{"choices":[{"message":{"role":"assistant","content":"anonymous ok"},"finish_reason":"stop"}]}"#,
            "openai_responses" | "xai_responses" => r#"{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"anonymous ok"}]}]}"#,
            "anthropic_messages" => r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"anonymous ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#,
            "gemini" => r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"anonymous ok"}]},"finishReason":"STOP"}]}"#,
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
    async fn anonymous_model_protocols_reach_endpoint_and_return_response() {
        for format in FORMATS {
            for keys in [Vec::new(), vec!["".into(), " \t ".into()]] {
                for stream in [false, true] {
                    let (url, server) = endpoint(vec![("200 OK", success(format, stream))]).await;
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
                    let (url, server) = endpoint(vec![(
                        status,
                        r#"{"error":{"message":"server requires authentication"}}"#.into(),
                    )])
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
            let (url, server) = endpoint(vec![
                ("401 Unauthorized", r#"{"error":{"message":"invalid key"}}"#.into()),
                ("200 OK", success(format, false)),
            ])
            .await;
            let provider = provider(format, url, vec!["invalid-key".into(), "valid-key".into()]);
            let state = crate::state::test_app_state();
            assert_eq!(call(&state, &provider, false).await.unwrap().text, "anonymous ok");
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
