mod support;

use std::sync::Arc;

use kivio_agent_sdk::{
    agent::{
        events::RetryStatus,
        history::request_from_history,
        provider_runtime::{ModelCall, ModelDelivery, ModelOutputSink, ModelRuntime},
        AgentSession, AgentSessionConfig,
    },
    model::{
        anthropic::{AnthropicMessagesConfig, AnthropicMessagesProvider},
        gemini::{GeminiConfig, GeminiProvider, GeminiThinkingMode},
        openai::{OpenAiChatConfig, OpenAiChatProvider},
        responses::{OpenAiResponsesConfig, OpenAiResponsesProvider},
        GenerateOutput, GenerateRequest, LanguageModelProvider, MessagePart, ModelError,
        ModelErrorKind, ModelMessage, ProviderSignature, StreamPart, StreamSink, ToolChoice,
    },
};
use kivio_agent_sdk_consumer::{echo_definition, initial_history, EchoTool};
use serde_json::{json, Value};
use support::{event, Response, Server, LIMIT};
use tokio::time::timeout;

trait ConformanceProvider: LanguageModelProvider + ModelRuntime {}
impl<T: LanguageModelProvider + ModelRuntime> ConformanceProvider for T {}

#[derive(Clone, Copy, Debug)]
enum ProtocolCase {
    Chat,
    Responses,
    Anthropic,
    Gemini,
}

const CASES: &[ProtocolCase] = &[
    ProtocolCase::Chat,
    ProtocolCase::Responses,
    ProtocolCase::Anthropic,
    ProtocolCase::Gemini,
];

impl ProtocolCase {
    async fn server(self, path: &str, responses: Vec<Response>) -> Server {
        match self {
            Self::Gemini => {
                Server::start_paths(
                    path,
                    vec![
                        format!("{path}/models/test-model:streamGenerateContent?alt=sse"),
                        format!("{path}/models/test-model:generateContent"),
                    ],
                    responses,
                )
                .await
            }
            _ => Server::start_at(path, responses).await,
        }
    }

    fn provider(self, server: &Server) -> Arc<dyn ConformanceProvider> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(LIMIT)
            .build()
            .unwrap();
        let extra = match self {
            Self::Chat => json!({"temperature":0.1,"max_tokens":1,
                "reasoning_effort":"low","vendor":"default"}),
            Self::Responses => json!({"temperature":0.1,"max_output_tokens":1,
                "reasoning":{"effort":"low"},"vendor":"default"}),
            Self::Anthropic => json!({"temperature":0.1,"max_tokens":1,"vendor":"default"}),
            Self::Gemini => json!({"generationConfig":{"temperature":0.1,"maxOutputTokens":1,
                "thinkingConfig":{"thinkingLevel":"low"}},"vendor":"default"}),
        };
        match self {
            Self::Chat => {
                let mut config = OpenAiChatConfig::new(server.endpoint.clone());
                config.api_key = Some("fixture-key".into());
                config
                    .headers
                    .insert("x-consumer", "standalone".parse().unwrap());
                config.options.temperature = Some(0.2);
                config.options.extra_body = extra;
                Arc::new(OpenAiChatProvider::new(client, config))
            }
            Self::Responses => {
                let mut config = OpenAiResponsesConfig::new(server.endpoint.clone());
                config.api_key = Some("fixture-key".into());
                config
                    .headers
                    .insert("x-consumer", "standalone".parse().unwrap());
                config.options.temperature = Some(0.2);
                config.options.extra_body = extra;
                Arc::new(OpenAiResponsesProvider::new(client, config))
            }
            Self::Anthropic => {
                let mut config = AnthropicMessagesConfig::new(server.endpoint.clone());
                config.api_key = Some("fixture-key".into());
                config
                    .headers
                    .insert("x-consumer", "standalone".parse().unwrap());
                config.options.temperature = Some(0.2);
                config.options.extra_body = extra;
                Arc::new(AnthropicMessagesProvider::new(client, config))
            }
            Self::Gemini => {
                let mut config = GeminiConfig::new(server.endpoint.clone());
                config.api_key = Some("fixture-key".into());
                config
                    .headers
                    .insert("x-consumer", "standalone".parse().unwrap());
                config.options.temperature = Some(0.2);
                config.options.thinking = GeminiThinkingMode::Level;
                config.options.include_tool_ids = true;
                config.options.provider_id = Some("fixture".into());
                config.options.extra_body = extra;
                Arc::new(GeminiProvider::new(client, config))
            }
        }
    }

    fn delta(self, text: &str) -> String {
        match self {
            Self::Chat => support::text_delta(text),
            Self::Responses => event(json!({"type":"response.output_text.delta","delta":text})),
            Self::Anthropic => event(
                json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":text}}),
            ),
            Self::Gemini => {
                event(json!({"candidates":[{"content":{"role":"model","parts":[{"text":text}]}}]}))
            }
        }
    }

    fn usage(self) -> Value {
        match self {
            Self::Chat => json!({"prompt_tokens":12,"completion_tokens":5,"total_tokens":17,
                "prompt_tokens_details":{"cached_tokens":4},"completion_tokens_details":{"reasoning_tokens":2}}),
            Self::Responses => json!({"input_tokens":12,"output_tokens":5,"total_tokens":17,
                "input_tokens_details":{"cached_tokens":4},"output_tokens_details":{"reasoning_tokens":2}}),
            Self::Anthropic => json!({"input_tokens":12,"output_tokens":5,
                "cache_read_input_tokens":4,"cache_creation_input_tokens":2}),
            Self::Gemini => json!({"promptTokenCount":12,"candidatesTokenCount":5,
                "totalTokenCount":19,"thoughtsTokenCount":2,"cachedContentTokenCount":4}),
        }
    }

    fn overrides(self) -> Value {
        match self {
            Self::Chat => json!({"temperature":0.9,"max_tokens":999,"reasoning_effort":"xhigh",
                "vendor":"call","tools":[{"type":"web_search"}],"tool_choice":"auto"}),
            Self::Responses => {
                json!({"temperature":0.9,"max_output_tokens":999,"reasoning":{"effort":"xhigh"},
                "vendor":"call","tools":[{"type":"web_search"}],"tool_choice":"auto"})
            }
            Self::Anthropic => json!({"temperature":0.9,"max_tokens":999,"vendor":"call",
                "tools":[{"type":"web_search_20250305","name":"web_search"}],"tool_choice":{"type":"auto"}}),
            Self::Gemini => json!({"generationConfig":{"temperature":0.9,"maxOutputTokens":999,
                "thinkingConfig":{"thinkingLevel":"xhigh"}},"vendor":"call",
                "tools":[{"googleSearch":{}}],"toolConfig":{"functionCallingConfig":{"mode":"AUTO"}}}),
        }
    }

    fn terminal(self, text: &str) -> String {
        match self {
            Self::Chat => format!(
                "{}data: [DONE]\n\n",
                event(
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":self.usage()})
                )
            ),
            Self::Responses => event(
                json!({"type":"response.completed","response":{"status":"completed",
                "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":self.usage()}}),
            ),
            Self::Anthropic => format!(
                "{}{}{}",
                event(json!({"type":"content_block_stop","index":1})),
                event(
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":self.usage()})
                ),
                event(json!({"type":"message_stop"}))
            ),
            Self::Gemini => {
                event(json!({"candidates":[{"finishReason":"STOP"}],"usageMetadata":self.usage()}))
            }
        }
    }

    fn answer(self, text: &str) -> Response {
        let reasoning = match self {
            Self::Chat => {
                event(json!({"choices":[{"index":0,"delta":{"reasoning_content":"considered"}}]}))
            }
            Self::Responses => {
                event(json!({"type":"response.reasoning_summary_text.delta","delta":"considered"}))
            }
            Self::Anthropic => [
                event(json!({"type":"message_start","message":{"id":"msg_42","role":"assistant","content":[],"usage":self.usage()}})),
                event(json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}})),
                event(json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"considered"}})),
                event(json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed-reasoning"}})),
                event(json!({"type":"content_block_stop","index":0})),
                event(json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}})),
            ].concat(),
            Self::Gemini => event(json!({"candidates":[{"content":{"role":"model","parts":[{"thought":true,"text":"considered"}]}}]})),
        };
        Response::Sse(format!(
            "{reasoning}{}{}",
            self.delta(text),
            self.terminal(text)
        ))
    }

    fn buffered(self, text: &str) -> Response {
        match self {
            Self::Chat => Response::Json(
                json!({"choices":[{"message":{"role":"assistant","content":text,"reasoning_content":"considered"},"finish_reason":"stop"}],"usage":self.usage()}),
            ),
            Self::Responses => Response::Json(json!({"status":"completed","output":[
                {"type":"reasoning","summary":[{"type":"summary_text","text":"considered"}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],
                "usage":self.usage()})),
            Self::Anthropic => {
                Response::Json(json!({"id":"msg_42","type":"message","role":"assistant",
                "content":[{"type":"thinking","thinking":"considered","signature":"signed-reasoning"},{"type":"text","text":text}],
                "stop_reason":"end_turn","usage":self.usage()}))
            }
            Self::Gemini => {
                Response::Json(json!({"candidates":[{"content":{"role":"model","parts":[
                {"thought":true,"text":"considered"},{"text":text}]},"finishReason":"STOP"}],"usageMetadata":self.usage()}))
            }
        }
    }

    fn tool(self, arguments: &str) -> Response {
        match self {
            Self::Chat => support::tool("echo", arguments),
            Self::Responses => {
                let item = json!({"id":"fc_42","type":"function_call","call_id":"call-42","name":"echo","arguments":arguments,"status":"completed"});
                Response::Sse(format!(
                    "{}{}{}",
                    event(
                        json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc_42","type":"function_call","call_id":"call-42","name":"echo","arguments":""}})
                    ),
                    event(json!({"type":"response.output_item.done","output_index":0,"item":item})),
                    event(
                        json!({"type":"response.completed","response":{"status":"completed","output":[item]}})
                    )
                ))
            }
            Self::Anthropic => Response::Sse([
                event(json!({"type":"message_start","message":{"id":"msg_42","role":"assistant","content":[],"usage":self.usage()}})),
                event(json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call-42","name":"echo","input":{}}})),
                event(json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":arguments}})),
                event(json!({"type":"content_block_stop","index":0})),
                event(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":self.usage()})),
                event(json!({"type":"message_stop"})),
            ].concat()),
            Self::Gemini => {
                let arguments = serde_json::from_str::<Value>(arguments).unwrap();
                Response::Sse(event(json!({"candidates":[{"content":{"role":"model","parts":[
                    {"functionCall":{"id":"call-42","name":"echo","args":arguments},"thoughtSignature":"signed-call"}]},
                    "finishReason":"STOP"}]})))
            }
        }
    }

    fn assert_tool_replay(self, body: &Value, expected: &str) {
        match self {
            Self::Chat => {
                let messages = body["messages"].as_array().unwrap();
                let call = messages
                    .iter()
                    .find(|m| m["tool_calls"].is_array())
                    .unwrap();
                assert_eq!(call["tool_calls"][0]["id"], "call-42");
                let result = messages.iter().find(|m| m["role"] == "tool").unwrap();
                assert_eq!(result["tool_call_id"], "call-42");
                assert!(result["content"].as_str().unwrap().contains(expected));
            }
            Self::Responses => {
                let input = body["input"].as_array().unwrap();
                let call = input.iter().find(|m| m["type"] == "function_call").unwrap();
                assert_eq!(call["call_id"], "call-42");
                let result = input
                    .iter()
                    .find(|m| m["type"] == "function_call_output")
                    .unwrap();
                assert_eq!(result["call_id"], "call-42");
                assert!(result["output"].as_str().unwrap().contains(expected));
            }
            Self::Anthropic => {
                let blocks = body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|message| message["content"].as_array().unwrap())
                    .collect::<Vec<_>>();
                let call = blocks
                    .iter()
                    .find(|block| block["type"] == "tool_use")
                    .unwrap();
                assert_eq!(call["id"], "call-42");
                let result = blocks
                    .iter()
                    .find(|block| block["type"] == "tool_result")
                    .unwrap();
                assert_eq!(result["tool_use_id"], "call-42");
                assert!(result["content"].as_str().unwrap().contains(expected));
            }
            Self::Gemini => {
                let parts = body["contents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|message| message["parts"].as_array().unwrap())
                    .collect::<Vec<_>>();
                let call = parts
                    .iter()
                    .find(|part| part["functionCall"].is_object())
                    .unwrap();
                assert_eq!(call["functionCall"]["id"], "call-42");
                assert_eq!(call["thoughtSignature"], "signed-call");
                let result = parts
                    .iter()
                    .find(|part| part["functionResponse"].is_object())
                    .unwrap();
                assert_eq!(result["functionResponse"]["id"], "call-42");
                assert_eq!(result["functionResponse"]["name"], "echo");
                assert!(
                    result["functionResponse"]["response"]
                        .to_string()
                        .contains(expected),
                    "expected {expected}, received {result}"
                );
            }
        }
    }
}

fn request() -> GenerateRequest {
    request_from_history(
        &initial_history(),
        "test-model",
        vec![echo_definition().model],
        Default::default(),
        Default::default(),
    )
}

fn normalized(output: &GenerateOutput) -> Value {
    json!({"text":output.text,"reasoning":output.reasoning,"tools":output.tool_calls,
        "usage":output.usage,"finish":output.finish_reason,"reasoning_items":output.reasoning_items})
}

fn anthropic_pause(streamed: bool) -> Response {
    let usage = json!({"input_tokens":8,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":0});
    let content = json!([
        {"type":"server_tool_use","id":"srv_42","name":"web_search","input":{"query":"fixture query"}},
        {"type":"web_search_tool_result","tool_use_id":"srv_42","content":[]}
    ]);
    if !streamed {
        return Response::Json(json!({"id":"msg_pause","type":"message","role":"assistant",
            "content":content,"stop_reason":"pause_turn","usage":usage}));
    }
    let mut events = event(
        json!({"type":"message_start","message":{"id":"msg_pause","role":"assistant","content":[],"usage":usage}}),
    );
    for (index, block) in content.as_array().unwrap().iter().enumerate() {
        events.push_str(&event(
            json!({"type":"content_block_start","index":index,"content_block":block}),
        ));
        events.push_str(&event(json!({"type":"content_block_stop","index":index})));
    }
    events.push_str(&event(
        json!({"type":"message_delta","delta":{"stop_reason":"pause_turn"},"usage":usage}),
    ));
    events.push_str(&event(json!({"type":"message_stop"})));
    Response::Sse(events)
}

#[derive(Default)]
struct Sink {
    started: usize,
    parts: Vec<StreamPart>,
}
impl StreamSink for Sink {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        self.parts.push(part);
        Ok(())
    }
}
impl ModelOutputSink for Sink {
    fn request_started(&mut self, _: &ModelCall) {
        self.started += 1;
    }
    fn retry(&mut self, _: RetryStatus) {
        panic!("provider must not retry");
    }
    fn reset_attempt(&mut self) {
        panic!("provider must not reset a request");
    }
    fn draft_snapshot(&self) -> Vec<(String, String)> {
        vec![]
    }
}

#[tokio::test]
async fn streamed_collected_and_buffered_results_share_the_public_contract() {
    for case in CASES {
        let server = case
            .server(
                "/custom/provider",
                vec![
                    case.answer("answer"),
                    case.answer("answer"),
                    case.buffered("answer"),
                ],
            )
            .await;
        let provider = case.provider(&server);
        let mut outputs = vec![];
        for delivery in [ModelDelivery::Streamed, ModelDelivery::Collected] {
            let mut sink = Sink::default();
            let call = ModelCall {
                provider_id: "fixture".into(),
                model: "test-model".into(),
                capabilities: None,
                history: initial_history(),
                tools: vec![],
                options: Default::default(),
                metadata: Default::default(),
                delivery,
            };
            let output = timeout(
                LIMIT,
                ModelRuntime::generate(provider.as_ref(), call, &mut sink),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(sink.started, 1, "{case:?}");
            assert_eq!(output.reasoning.as_deref(), Some("considered"), "{case:?}");
            if delivery == ModelDelivery::Collected {
                assert!(sink.parts.is_empty(), "{case:?}");
            } else {
                let text = sink
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        StreamPart::TextDelta { delta } => Some(delta.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                assert_eq!(text, "answer", "{case:?}");
            }
            let usage = output.usage.as_ref().unwrap();
            assert_eq!(
                (usage.input_tokens, usage.output_tokens, usage.total_tokens),
                match case {
                    ProtocolCase::Gemini => (Some(12), Some(5), Some(19)),
                    _ => (Some(12), Some(5), Some(17)),
                },
                "{case:?}"
            );
            assert_eq!(
                (usage.cached_input_tokens, usage.reasoning_tokens),
                (
                    Some(4),
                    match case {
                        ProtocolCase::Anthropic => None,
                        _ => Some(2),
                    }
                ),
                "{case:?}"
            );
            assert_eq!(
                usage.cache_creation_input_tokens,
                match case {
                    ProtocolCase::Anthropic => Some(2),
                    _ => None,
                },
                "{case:?}"
            );
            outputs.push(normalized(&output));
        }
        let buffered = LanguageModelProvider::generate(provider.as_ref(), request())
            .await
            .unwrap();
        assert_eq!(outputs[0], outputs[1], "{case:?}");
        assert_eq!(outputs[0], normalized(&buffered), "{case:?}");
        assert_eq!(server.count(), 3, "{case:?}");
        if matches!(case, ProtocolCase::Gemini) {
            let requests = server.requests.lock().unwrap();
            assert!(requests[..2].iter().all(|received| {
                received.headers.starts_with(
                "POST /custom/provider/models/test-model:streamGenerateContent?alt=sse HTTP/1.1"
            )
            }));
            assert!(requests[2]
                .headers
                .starts_with("POST /custom/provider/models/test-model:generateContent HTTP/1.1"));
        }
    }
}

#[tokio::test]
async fn responses_buffered_generation_accepts_sse_only_endpoints() {
    let case = ProtocolCase::Responses;
    let server = case
        .server("/responses-sse", vec![case.answer("answer")])
        .await;
    let provider = case.provider(&server);
    let output = LanguageModelProvider::generate(provider.as_ref(), request())
        .await
        .unwrap();
    assert_eq!(output.text, "answer");
    assert_eq!(output.reasoning.as_deref(), Some("considered"));
    assert_eq!(output.usage.unwrap().input_tokens, Some(12));
}

#[tokio::test]
async fn anthropic_native_continuations_keep_final_usage_separate_from_billing() {
    let case = ProtocolCase::Anthropic;
    for streamed in [false, true] {
        let final_answer = if streamed {
            case.answer("answer")
        } else {
            case.buffered("answer")
        };
        let server = case
            .server(
                "/native-messages",
                vec![anthropic_pause(streamed), final_answer],
            )
            .await;
        let provider = case.provider(&server);
        let mut sink = Sink::default();
        let output = if streamed {
            LanguageModelProvider::stream(provider.as_ref(), request(), &mut sink)
                .await
                .unwrap()
        } else {
            LanguageModelProvider::generate(provider.as_ref(), request())
                .await
                .unwrap()
        };
        assert_eq!(output.text, "answer");
        assert!(output.tool_calls.is_empty());
        let usage = output.usage.unwrap();
        assert_eq!(
            (usage.input_tokens, usage.output_tokens, usage.total_tokens),
            (Some(12), Some(5), Some(17))
        );
        let bill = output.billing_usage.unwrap();
        assert_eq!(
            (bill.input_tokens, bill.output_tokens, bill.total_tokens),
            (Some(20), Some(7), Some(27))
        );
        assert_eq!(
            (bill.cached_input_tokens, bill.cache_creation_input_tokens),
            (Some(7), Some(2))
        );
        if streamed {
            assert_eq!(
                sink.parts
                    .iter()
                    .filter(|part| matches!(part, StreamPart::Finish { .. }))
                    .count(),
                1
            );
        }
        assert_eq!(server.count(), 2);
        let requests = server.requests.lock().unwrap();
        let messages = requests[1].body["messages"].as_array().unwrap();
        let replay = messages.last().unwrap();
        assert_eq!(replay["role"], "assistant");
        assert_eq!(replay["content"][0]["type"], "server_tool_use");
        assert_eq!(replay["content"][1]["tool_use_id"], "srv_42");
    }
    let server = case
        .server(
            "/bounded-native-messages",
            (0..9).map(|_| anthropic_pause(false)).collect(),
        )
        .await;
    let provider = case.provider(&server);
    assert!(timeout(
        LIMIT,
        LanguageModelProvider::generate(provider.as_ref(), request())
    )
    .await
    .unwrap()
    .is_err());
    assert_eq!(server.count(), 9);
}

#[tokio::test]
async fn anthropic_thinking_omits_temperature_on_the_wire() {
    let case = ProtocolCase::Anthropic;
    let server = case
        .server("/thinking-messages", vec![case.answer("answer")])
        .await;
    let provider = case.provider(&server);
    let mut request = request();
    request.options.temperature = Some(0.7);
    request.options.thinking_level = Some("high".into());
    LanguageModelProvider::stream(provider.as_ref(), request, &mut |_| Ok(()))
        .await
        .unwrap();
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].body.get("temperature").is_none());
    assert_eq!(requests[0].body["thinking"]["type"], "adaptive");
    assert_eq!(requests[0].body["output_config"]["effort"], "high");
}

#[tokio::test]
async fn explicit_endpoint_headers_and_call_overrides_reach_the_wire() {
    for case in CASES {
        let server = case
            .server(
                "/gateway/custom-generation",
                vec![
                    case.answer("answer"),
                    case.answer("answer"),
                    case.answer("answer"),
                ],
            )
            .await;
        let provider = case.provider(&server);
        for (temperature, overrides) in [
            (None, json!({})),
            (Some(0.7), json!({})),
            (Some(0.7), case.overrides()),
        ] {
            let mut request = request();
            request.options.temperature = temperature;
            request.options.max_tokens = 321;
            request.options.thinking_level = Some("high".into());
            request.options.thinking_enabled = !matches!(case, ProtocolCase::Anthropic);
            request.options.tool_choice = ToolChoice::None;
            request.options.builtin_web_search = true;
            request.options.provider_options = overrides;
            LanguageModelProvider::stream(provider.as_ref(), request, &mut |_| Ok(()))
                .await
                .unwrap();
        }
        let requests = server.requests.lock().unwrap();
        for (received, temperature, vendor, max_tokens, effort) in [
            (&requests[0], 0.2, "default", 321, "high"),
            (&requests[1], 0.7, "default", 321, "high"),
            (&requests[2], 0.9, "call", 999, "xhigh"),
        ] {
            let headers = received.headers.to_ascii_lowercase();
            let path = match case {
                ProtocolCase::Gemini => {
                    "/gateway/custom-generation/models/test-model:streamgeneratecontent?alt=sse"
                }
                _ => "/gateway/custom-generation",
            };
            assert!(
                headers.starts_with(&format!("post {path} http/1.1")),
                "{case:?}"
            );
            let auth_header = match case {
                ProtocolCase::Anthropic => "x-api-key: fixture-key",
                ProtocolCase::Gemini => "x-goog-api-key: fixture-key",
                _ => "authorization: bearer fixture-key",
            };
            assert!(headers.contains(auth_header), "{case:?}");
            if matches!(case, ProtocolCase::Anthropic) {
                assert!(headers.contains("anthropic-version: 2023-06-01"));
            }
            assert!(headers.contains("x-consumer: standalone"), "{case:?}");
            assert_eq!(received.body["vendor"], vendor, "{case:?}");
            let generation_config = match case {
                ProtocolCase::Gemini => &received.body["generationConfig"],
                _ => &received.body,
            };
            assert_eq!(generation_config["temperature"], temperature, "{case:?}");
            let token_field = match case {
                ProtocolCase::Chat => "max_tokens",
                ProtocolCase::Responses => "max_output_tokens",
                ProtocolCase::Anthropic => "max_tokens",
                ProtocolCase::Gemini => "maxOutputTokens",
            };
            assert_eq!(generation_config[token_field], max_tokens, "{case:?}");
            match case {
                ProtocolCase::Chat => assert_eq!(received.body["reasoning_effort"], effort),
                ProtocolCase::Responses => assert_eq!(received.body["reasoning"]["effort"], effort),
                ProtocolCase::Anthropic => {}
                ProtocolCase::Gemini => {
                    assert_eq!(generation_config["thinkingConfig"]["thinkingLevel"], effort)
                }
            }
            assert!(received.body.get("tools").is_none(), "{case:?}");
            match case {
                ProtocolCase::Gemini => assert_eq!(
                    received.body["toolConfig"]["functionCallingConfig"]["mode"],
                    "NONE"
                ),
                ProtocolCase::Anthropic => {
                    assert_eq!(received.body["tool_choice"], json!({"type":"none"}))
                }
                _ => assert_eq!(received.body["tool_choice"], "none", "{case:?}"),
            }
        }
    }
}

#[tokio::test]
async fn session_tool_authorization_and_later_prompt_do_not_reexecute_history() {
    for case in CASES {
        for (arguments, allowed, executions, expected) in [
            (r#"{"text":"hello"}"#, true, 1, "hello"),
            (r#"{"text":"hello"}"#, false, 0, "Host denied echo"),
            (r#"{"text":"unfinished"#, true, 0, "invalid"),
        ] {
            let (arguments, expected) = match (case, expected) {
                (ProtocolCase::Gemini, "invalid") => (r#"{"text":{}}"#, "must be string"),
                _ => (arguments, expected),
            };
            let server = case
                .server(
                    "/conformance",
                    vec![
                        case.tool(arguments),
                        case.answer("done"),
                        case.answer("continued"),
                    ],
                )
                .await;
            let provider = case.provider(&server);
            let executor = Arc::new(EchoTool::new(allowed));
            let mut config = AgentSessionConfig::new(provider, "test-model");
            config
                .tools
                .register_dynamic(echo_definition(), executor.clone())
                .unwrap();
            let session = AgentSession::new(config);
            let first = timeout(LIMIT, session.prompt("echo hello"))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(first.content, "done", "{case:?}");
            assert_eq!(executor.call_count(), executions, "{case:?}");
            let next = timeout(LIMIT, session.prompt("continue"))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(next.content, "continued", "{case:?}");
            assert_eq!(executor.call_count(), executions, "{case:?}");
            assert_eq!(server.count(), 3, "{case:?}");
            assert_eq!(
                session.context().last().unwrap().text_content(),
                "continued"
            );
            let requests = server.requests.lock().unwrap();
            for received in &requests[1..] {
                case.assert_tool_replay(&received.body, expected);
            }
            let replay = requests[2].body.to_string();
            assert!(replay.contains("continue"), "{case:?}");
            assert!(replay.contains("done"), "{case:?}");
        }
    }
}

#[tokio::test]
async fn gemini_tool_signatures_survive_persistence_and_only_replay_to_their_source() {
    let case = ProtocolCase::Gemini;
    let server = case
        .server(
            "/signed",
            vec![case.tool(r#"{"text":"hello"}"#), case.answer("done")],
        )
        .await;
    let executor = Arc::new(EchoTool::new(true));
    let mut config = AgentSessionConfig::new(case.provider(&server), "test-model");
    config
        .tools
        .register_dynamic(echo_definition(), executor.clone())
        .unwrap();
    let session = AgentSession::new(config);
    assert_eq!(
        timeout(LIMIT, session.prompt("echo hello"))
            .await
            .unwrap()
            .unwrap()
            .content,
        "done"
    );
    assert_eq!(executor.call_count(), 1);
    let mut replay = request_from_history(
        &session.context(),
        "test-model",
        vec![echo_definition().model],
        Default::default(),
        Default::default(),
    );
    let persisted = serde_json::to_vec(&replay.messages).unwrap();
    replay.messages = serde_json::from_slice::<Vec<ModelMessage>>(&persisted).unwrap();
    let signature = replay
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| match part {
            MessagePart::ToolCall { signature, .. } => signature.as_ref(),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        signature,
        &ProviderSignature::Scoped {
            value: "signed-call".into(),
            model: "test-model".into(),
            provider_id: Some("fixture".into()),
        }
    );

    for (model, provider_id, expected_signature) in [
        ("test-model", "fixture", true),
        ("changed-model", "fixture", false),
        ("test-model", "changed-provider", false),
    ] {
        let server = Server::start_paths(
            "/replay",
            vec![format!("/replay/models/{model}:generateContent")],
            vec![case.buffered("replayed")],
        )
        .await;
        let mut config = GeminiConfig::new(server.endpoint.clone());
        config.options.provider_id = Some(provider_id.into());
        config.options.include_tool_ids = true;
        let provider = GeminiProvider::new(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(LIMIT)
                .build()
                .unwrap(),
            config,
        );
        replay.model = model.into();
        assert_eq!(
            LanguageModelProvider::generate(&provider, replay.clone())
                .await
                .unwrap()
                .text,
            "replayed"
        );
        let requests = server.requests.lock().unwrap();
        let call = requests[0].body["contents"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["parts"].as_array().unwrap())
            .find(|part| part["functionCall"].is_object())
            .unwrap();
        assert_eq!(call["functionCall"]["name"], "echo");
        assert_eq!(call["functionCall"]["args"]["text"], "hello");
        assert_eq!(
            call.get("thoughtSignature").is_some(),
            expected_signature,
            "{model}/{provider_id}"
        );
        if expected_signature {
            assert_eq!(call["thoughtSignature"], "signed-call");
        }
    }
}

#[tokio::test]
async fn http_errors_keep_status_code_and_retry_after_without_retrying() {
    for case in CASES {
        for (status, code, kind) in [
            (401, "invalid_api_key", ModelErrorKind::Auth),
            (429, "rate_limit_exceeded", ModelErrorKind::RateLimited),
        ] {
            let code = match (case, status) {
                (ProtocolCase::Anthropic, 401) => "authentication_error",
                (ProtocolCase::Anthropic, 429) => "rate_limit_error",
                (ProtocolCase::Gemini, 401) => "UNAUTHENTICATED",
                (ProtocolCase::Gemini, 429) => "RESOURCE_EXHAUSTED",
                _ => code,
            };
            let body = match case {
                ProtocolCase::Anthropic => {
                    json!({"type":"error","error":{"type":code,"message":"fixture failure"}})
                }
                ProtocolCase::Gemini => {
                    json!({"error":{"code":status,"status":code,"message":"fixture failure"}})
                }
                _ => json!({"error":{"code":code,"message":"fixture failure"}}),
            };
            let server = case
                .server(
                    "/conformance",
                    vec![Response::HttpHeaders(
                        status,
                        body.to_string(),
                        vec![("Retry-After", "7")],
                    )],
                )
                .await;
            let provider = case.provider(&server);
            let error =
                LanguageModelProvider::stream(provider.as_ref(), request(), &mut |_| Ok(()))
                    .await
                    .unwrap_err();
            assert_eq!(error.kind, kind, "{case:?}");
            assert_eq!(error.status, Some(status), "{case:?}");
            assert_eq!(error.provider_code.as_deref(), Some(code), "{case:?}");
            assert_eq!(
                error.retry_after,
                Some(std::time::Duration::from_secs(7)),
                "{case:?}"
            );
            assert_eq!(server.count(), 1, "{case:?}");
        }
    }
}

#[tokio::test]
async fn empty_or_unterminated_streams_cannot_report_success() {
    for case in CASES {
        for body in [String::new(), case.delta("unfinished")] {
            let server = case.server("/conformance", vec![Response::Sse(body)]).await;
            let provider = case.provider(&server);
            let error =
                LanguageModelProvider::stream(provider.as_ref(), request(), &mut |_| Ok(()))
                    .await
                    .unwrap_err();
            assert_eq!(error.kind, ModelErrorKind::Transport, "{case:?}");
            assert!(error.is_retryable_stream_error(), "{case:?}");
            assert_eq!(server.count(), 1, "{case:?}");
        }
    }
}

#[tokio::test]
async fn dropping_a_stream_or_rejecting_a_delta_closes_its_http_read() {
    for case in CASES {
        let server = case
            .server(
                "/conformance",
                vec![
                    Response::Hold(case.delta("partial")),
                    Response::Hold(case.delta("rejected")),
                ],
            )
            .await;
        let provider = case.provider(&server);
        let observed = tokio::sync::Notify::new();
        let mut sink = |part| {
            if matches!(part, StreamPart::TextDelta { .. }) {
                observed.notify_one();
            }
            Ok(())
        };
        let mut stream = LanguageModelProvider::stream(provider.as_ref(), request(), &mut sink);
        tokio::select! {
            result = &mut stream => panic!("{case:?} completed held stream: {result:?}"),
            result = timeout(LIMIT, observed.notified()) => result.unwrap(),
        }
        drop(stream);
        timeout(LIMIT, server.disconnected.notified())
            .await
            .unwrap();
        let rejected = ModelError::with_kind("consumer rejected output", ModelErrorKind::Other);
        let mut sink = |_| Err(rejected.clone());
        let error = LanguageModelProvider::stream(provider.as_ref(), request(), &mut sink)
            .await
            .unwrap_err();
        assert_eq!(error, rejected, "{case:?}");
        timeout(LIMIT, server.disconnected.notified())
            .await
            .unwrap();
        assert_eq!(server.count(), 2, "{case:?}");
    }
}
