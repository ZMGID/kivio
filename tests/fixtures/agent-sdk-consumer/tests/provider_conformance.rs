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
        openai::{OpenAiChatConfig, OpenAiChatProvider},
        responses::{OpenAiResponsesConfig, OpenAiResponsesProvider},
        GenerateOutput, GenerateRequest, LanguageModelProvider, ModelError, ModelErrorKind,
        StreamPart, StreamSink, ToolChoice,
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
}

const CASES: &[ProtocolCase] = &[ProtocolCase::Chat, ProtocolCase::Responses];

impl ProtocolCase {
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
        }
    }

    fn delta(self, text: &str) -> String {
        match self {
            Self::Chat => support::text_delta(text),
            Self::Responses => event(json!({"type":"response.output_text.delta","delta":text})),
        }
    }

    fn usage(self) -> Value {
        match self {
            Self::Chat => json!({"prompt_tokens":12,"completion_tokens":5,"total_tokens":17,
                "prompt_tokens_details":{"cached_tokens":4},"completion_tokens_details":{"reasoning_tokens":2}}),
            Self::Responses => json!({"input_tokens":12,"output_tokens":5,"total_tokens":17,
                "input_tokens_details":{"cached_tokens":4},"output_tokens_details":{"reasoning_tokens":2}}),
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
        let server = Server::start_at(
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
                (Some(12), Some(5), Some(17)),
                "{case:?}"
            );
            assert_eq!(
                (usage.cached_input_tokens, usage.reasoning_tokens),
                (Some(4), Some(2)),
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
    }
}

#[tokio::test]
async fn responses_buffered_generation_accepts_sse_only_endpoints() {
    let case = ProtocolCase::Responses;
    let server = Server::start_at("/responses-sse", vec![case.answer("answer")]).await;
    let provider = case.provider(&server);
    let output = LanguageModelProvider::generate(provider.as_ref(), request())
        .await
        .unwrap();
    assert_eq!(output.text, "answer");
    assert_eq!(output.reasoning.as_deref(), Some("considered"));
    assert_eq!(output.usage.unwrap().input_tokens, Some(12));
}

#[tokio::test]
async fn explicit_endpoint_headers_and_call_overrides_reach_the_wire() {
    for case in CASES {
        let server = Server::start_at(
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
            (
                Some(0.7),
                json!({"temperature":0.9,"max_tokens":999,"max_output_tokens":999,
                    "reasoning_effort":"xhigh","reasoning":{"effort":"xhigh"},
                    "vendor":"call","tools":[{"type":"web_search"}],"tool_choice":"auto"}),
            ),
        ] {
            let mut request = request();
            request.options.temperature = temperature;
            request.options.max_tokens = 321;
            request.options.thinking_level = Some("high".into());
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
            assert!(
                headers.starts_with("post /gateway/custom-generation http/1.1"),
                "{case:?}"
            );
            assert!(
                headers.contains("authorization: bearer fixture-key"),
                "{case:?}"
            );
            assert!(headers.contains("x-consumer: standalone"), "{case:?}");
            assert_eq!(received.body["temperature"], temperature, "{case:?}");
            assert_eq!(received.body["vendor"], vendor, "{case:?}");
            let token_field = match case {
                ProtocolCase::Chat => "max_tokens",
                ProtocolCase::Responses => "max_output_tokens",
            };
            assert_eq!(received.body[token_field], max_tokens, "{case:?}");
            let reasoning = match case {
                ProtocolCase::Chat => &received.body["reasoning_effort"],
                ProtocolCase::Responses => &received.body["reasoning"]["effort"],
            };
            assert_eq!(reasoning, effort, "{case:?}");
            assert!(received.body.get("tools").is_none(), "{case:?}");
            assert_eq!(received.body["tool_choice"], "none", "{case:?}");
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
            let server = Server::start_at(
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
                session.history().last().unwrap().text_content(),
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
async fn http_errors_keep_status_code_and_retry_after_without_retrying() {
    for case in CASES {
        for (status, code, kind) in [
            (401, "invalid_api_key", ModelErrorKind::Auth),
            (429, "rate_limit_exceeded", ModelErrorKind::RateLimited),
        ] {
            let server = Server::start_at(
                "/conformance",
                vec![Response::HttpHeaders(
                    status,
                    json!({"error":{"code":code,"message":"fixture failure"}}).to_string(),
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
            let server = Server::start_at("/conformance", vec![Response::Sse(body)]).await;
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
        let server = Server::start_at(
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
