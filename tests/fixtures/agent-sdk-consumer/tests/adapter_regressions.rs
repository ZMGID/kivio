mod support;

use kivio_agent_sdk::model::{
    gemini::{GeminiConfig, GeminiProvider},
    responses::{OpenAiResponsesConfig, OpenAiResponsesProvider},
    GenerateRequest, LanguageModelProvider, ModelErrorKind, ModelMessage, ModelRole, StreamPart,
};
use serde_json::json;
use support::{event, Response, Server};

fn request() -> GenerateRequest {
    GenerateRequest {
        model: "test-model".into(),
        system: String::new(),
        tools: vec![],
        messages: vec![ModelMessage::text(ModelRole::User, "hello")],
        options: Default::default(),
        metadata: Default::default(),
    }
}

#[tokio::test]
async fn gemini_blocked_responses_are_errors_in_both_delivery_modes() {
    for value in [
        json!({"promptFeedback":{"blockReason":"SAFETY"}}),
        json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"echo","args":{}}}]},"finishReason":"SAFETY"}]}),
        json!({"candidates":[{"finishReason":"MALFORMED_FUNCTION_CALL"}]}),
    ] {
        let server = Server::start_paths(
            "/gemini",
            vec![
                "/gemini/models/test-model:generateContent".into(),
                "/gemini/models/test-model:streamGenerateContent?alt=sse".into(),
            ],
            vec![
                Response::Json(value.clone()),
                Response::Sse(event(value.clone())),
            ],
        )
        .await;
        let provider = GeminiProvider::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            GeminiConfig::new(server.endpoint.clone()),
        );
        let buffered = provider.generate(request()).await;
        let mut events = vec![];
        let streamed = provider
            .stream(request(), &mut |part| {
                events.push(part);
                Ok(())
            })
            .await;
        for result in [buffered, streamed] {
            let error = result.expect_err("blocked output must not succeed");
            assert_eq!(
                error.kind,
                if value.to_string().contains("SAFETY") {
                    ModelErrorKind::ContentModeration
                } else {
                    ModelErrorKind::Other
                }
            );
        }
        assert!(!events.iter().any(|part| matches!(
            part,
            StreamPart::Finish { .. } | StreamPart::ToolCallStart { .. }
        )));
    }
}

#[tokio::test]
async fn gemini_generated_usage_includes_thoughts() {
    let value = json!({"candidates":[{"content":{"parts":[{"text":"answer"}]},"finishReason":"STOP"}],
        "usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":5,"thoughtsTokenCount":70,"totalTokenCount":175}});
    let server = Server::start_paths(
        "/gemini",
        vec![
            "/gemini/models/test-model:generateContent".into(),
            "/gemini/models/test-model:streamGenerateContent?alt=sse".into(),
        ],
        vec![Response::Json(value.clone()), Response::Sse(event(value))],
    )
    .await;
    let provider = GeminiProvider::new(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        GeminiConfig::new(server.endpoint.clone()),
    );
    for output in [
        provider.generate(request()).await.unwrap(),
        provider.stream(request(), &mut |_| Ok(())).await.unwrap(),
    ] {
        let usage = output.usage.unwrap();
        assert_eq!(usage.output_tokens, Some(75));
        assert_eq!(usage.reasoning_tokens, Some(70));
    }
}

#[tokio::test]
async fn responses_refusal_is_visible_once_in_both_delivery_modes() {
    for deltas in [false, true] {
        let message = json!({"type":"message","id":"msg_a","role":"assistant","content":[{"type":"refusal","refusal":"Cannot help."}]});
        let response = json!({"status":"completed","output":[message]});
        let mut wire = String::new();
        if deltas {
            wire.push_str(&event(json!({"type":"response.refusal.delta","item_id":"msg_a","content_index":0,"delta":"Cannot "})));
            wire.push_str(&event(json!({"type":"response.refusal.delta","item_id":"msg_a","content_index":0,"delta":"help."})));
        }
        wire.push_str(&event(json!({"type":"response.refusal.done","item_id":"msg_a","content_index":0,"refusal":"Cannot help."})));
        wire.push_str(&event(
            json!({"type":"response.output_item.done","item":message}),
        ));
        wire.push_str(&event(
            json!({"type":"response.completed","response":response}),
        ));
        let server = Server::start_at(
            "/responses",
            vec![Response::Json(response), Response::Sse(wire)],
        )
        .await;
        let provider = OpenAiResponsesProvider::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            OpenAiResponsesConfig::new(server.endpoint.clone()),
        );
        assert_eq!(
            provider.generate(request()).await.unwrap().text,
            "Cannot help."
        );
        let mut visible = String::new();
        let output = provider
            .stream(request(), &mut |part| {
                if let StreamPart::TextDelta { delta } = part {
                    visible.push_str(&delta);
                }
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(output.text, "Cannot help.");
        assert_eq!(visible, "Cannot help.");
    }
}

#[tokio::test]
async fn responses_incomplete_distinguishes_limits_from_errors() {
    for reason in ["max_output_tokens", "content_filter", "unknown_reason"] {
        let value =
            json!({"status":"incomplete","incomplete_details":{"reason":reason},"output":[]});
        let server = Server::start_at(
            "/responses",
            vec![
                Response::Json(value.clone()),
                Response::Sse(event(
                    json!({"type":"response.incomplete","response":value}),
                )),
            ],
        )
        .await;
        let provider = OpenAiResponsesProvider::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            OpenAiResponsesConfig::new(server.endpoint.clone()),
        );
        for result in [
            provider.generate(request()).await,
            provider.stream(request(), &mut |_| Ok(())).await,
        ] {
            if reason == "max_output_tokens" {
                assert_eq!(
                    result.unwrap().finish_reason.unwrap().kind,
                    kivio_agent_sdk::model::FinishKind::Length
                );
            } else {
                let error = result.expect_err("non-length incomplete response must fail");
                assert_eq!(
                    error.kind,
                    if reason == "content_filter" {
                        ModelErrorKind::ContentModeration
                    } else {
                        ModelErrorKind::Other
                    }
                );
                assert!(error.message.contains(reason));
            }
        }
    }
}

#[test]
fn known_routes_select_limit_and_cache_fields_without_changing_unknown_gateways() {
    use kivio_agent_sdk::model::{
        openai::{self, CacheRetention, OpenAiChatOptions},
        responses::{self, OpenAiResponsesOptions},
    };
    for (model, supported) in [
        ("gpt-5-2025-08-07", false),
        ("gpt-5.5-2026-04-23", false),
        ("gpt-5.6", true),
        ("gpt-6.1-sol", true),
        ("claude-sonnet-4.6", false),
    ] {
        assert_eq!(
            openai::supports_explicit_prompt_cache(model),
            supported,
            "{model}"
        );
    }
    let mut request = request();
    request.model = "o3".into();
    request.options.max_tokens = 123;
    for (url, modern) in [
        ("https://api.openai.com/v1", true),
        ("https://gateway.example/v1", false),
    ] {
        let options = OpenAiChatOptions {
            endpoint: Some(url.parse().unwrap()),
            ..Default::default()
        };
        let body = openai::request_body(&request, false, &options);
        assert_eq!(
            body[if modern {
                "max_completion_tokens"
            } else {
                "max_tokens"
            }],
            123
        );
        assert!(body
            .get(if modern {
                "max_tokens"
            } else {
                "max_completion_tokens"
            })
            .is_none());
    }
    request.model = "anthropic/claude-sonnet-4.6".into();
    let options = OpenAiChatOptions {
        endpoint: Some("https://openrouter.ai/api/v1".parse().unwrap()),
        cache_retention: Some(CacheRetention::Long),
        prompt_cache_key: Some("conversation".into()),
        prompt_cache_retention: Some("24h".into()),
        ..Default::default()
    };
    let body = openai::request_body(&request, true, &options);
    assert_eq!(
        body["cache_control"],
        json!({"type":"ephemeral","ttl":"1h"})
    );
    assert!(body.get("prompt_cache_retention").is_none());
    let unknown = OpenAiChatOptions {
        endpoint: Some("https://gateway.example/v1".parse().unwrap()),
        ..options.clone()
    };
    let body = openai::request_body(&request, true, &unknown);
    assert!(body.get("cache_control").is_none());
    assert_eq!(body["prompt_cache_retention"], "24h");
    request.model = "gpt-5.6".into();
    for (retention, expected) in [
        (CacheRetention::None, json!({"mode":"explicit"})),
        (CacheRetention::Short, json!({"ttl":"30m"})),
        (CacheRetention::Long, json!({"ttl":"12h"})),
    ] {
        let options = OpenAiResponsesOptions {
            endpoint: Some("https://api.openai.com/v1".parse().unwrap()),
            cache_retention: Some(retention),
            prompt_cache_key: Some("conversation".into()),
            prompt_cache_retention: Some("24h".into()),
            ..Default::default()
        };
        let body = responses::request_body(&request, true, &options);
        assert_eq!(body["prompt_cache_options"], expected);
        let chat = openai::request_body(
            &request,
            true,
            &OpenAiChatOptions {
                endpoint: options.endpoint.clone(),
                cache_retention: Some(retention),
                ..Default::default()
            },
        );
        assert_eq!(
            chat["prompt_cache_options"],
            if retention == CacheRetention::None {
                json!({"mode":"explicit"})
            } else {
                json!({"ttl":"30m"})
            }
        );
        assert!(body.get("prompt_cache_retention").is_none());
    }
    request.options.provider_options = json!({"max_completion_tokens":456});
    request.model = "o3".into();
    let body = openai::request_body(
        &request,
        false,
        &OpenAiChatOptions {
            endpoint: Some("https://api.openai.com/v1".parse().unwrap()),
            ..Default::default()
        },
    );
    assert_eq!(body["max_completion_tokens"], 456);
}

#[tokio::test]
async fn reasoning_details_survive_stream_history_and_scoped_replay() {
    use kivio_agent_sdk::model::{
        model_messages_from_openai_messages,
        openai::{OpenAiChatConfig, OpenAiChatProvider},
        openai_messages_from_model_messages,
    };
    let details = json!([
        {"type":"reasoning.text","index":0,"id":"r0","text":"Think twice","signature":"signed","format":"anthropic-claude-v1"},
        {"type":"reasoning.encrypted","index":1,"id":"r1","data":"opaque"}
    ]);
    let tool = json!({"id":"call_1","type":"function","function":{"name":"echo","arguments":"{}"}});
    let response = json!({"choices":[{"message":{"role":"assistant","content":null,"reasoning_details":details,"tool_calls":[tool]},"finish_reason":"tool_calls"}]});
    let wire = [
        json!({"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","index":0,"id":"r0","text":"Think "}]}}]}),
        json!({"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","index":0,"text":"twice","signature":"signed","format":"anthropic-claude-v1"},details[1]]}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"echo","arguments":"{}"}}]},"finish_reason":"tool_calls"}]})
    ].into_iter().map(event).collect::<String>();
    for stream in [false, true] {
        let server = Server::start(vec![if stream {
            Response::Sse(wire.clone())
        } else {
            Response::Json(response.clone())
        }])
        .await;
        let mut config = OpenAiChatConfig::new(server.endpoint.clone());
        config.options.provider_id = Some("original".into());
        let provider = OpenAiChatProvider::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            config,
        );
        let output = if stream {
            provider.stream(request(), &mut |_| Ok(())).await
        } else {
            provider.generate(request()).await
        }
        .unwrap();
        assert_eq!(output.reasoning.as_deref(), Some("Think twice"));
        let history = openai_messages_from_model_messages(&[output.replay_message()]);
        let restored = model_messages_from_openai_messages(history);
        for (provider_id, model, expected) in [
            ("original", "test-model", true),
            ("other", "test-model", false),
            ("original", "other-model", false),
        ] {
            let server = Server::start(vec![Response::Json(json!({"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}]}))]).await;
            let mut config = OpenAiChatConfig::new(server.endpoint.clone());
            config.options.provider_id = Some(provider_id.into());
            let provider = OpenAiChatProvider::new(
                reqwest::Client::builder().no_proxy().build().unwrap(),
                config,
            );
            let mut next = request();
            next.model = model.into();
            next.messages.extend(restored.clone());
            next.messages.push(ModelMessage {
                role: ModelRole::Tool,
                content: vec![kivio_agent_sdk::model::MessagePart::ToolResult {
                    tool_call_id: "call_1".into(),
                    content: "ok".into(),
                    is_error: false,
                    artifacts: vec![],
                }],
            });
            provider.generate(next).await.unwrap();
            let requests = server.requests.lock().unwrap();
            let assistant = requests[0].body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["role"] == "assistant")
                .unwrap();
            if expected {
                assert_eq!(assistant["reasoning_details"], details);
            } else {
                assert!(assistant.get("reasoning_details").is_none());
            }
        }
    }
}

#[tokio::test]
async fn cache_write_ttl_survives_partial_usage_frames() {
    use kivio_agent_sdk::model::{
        anthropic::{AnthropicMessagesConfig, AnthropicMessagesProvider},
        openai::{OpenAiChatConfig, OpenAiChatProvider},
    };
    let wire = [
        json!({"choices":[{"delta":{"content":"ok"}}],"usage":{"prompt_tokens":100,"prompt_tokens_details":{"cached_tokens":60,"cache_write_tokens":30,"cache_write_tokens_details":{"ttl_30m_tokens":20,"ttl_12h_tokens":10}}}}),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"completion_tokens":5}}),
    ].into_iter().map(event).collect::<String>();
    let server = Server::start(vec![Response::Sse(wire)]).await;
    let provider = OpenAiChatProvider::new(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        OpenAiChatConfig::new(server.endpoint.clone()),
    );
    let usage = provider
        .stream(request(), &mut |_| Ok(()))
        .await
        .unwrap()
        .usage
        .unwrap();
    assert_eq!(usage.cache_creation_input_tokens, Some(30));
    assert_eq!(usage.cache_creation_input_tokens_long, Some(10));
    let wire = [
        json!({"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input_tokens":10,"cache_read_input_tokens":60,"cache_creation_input_tokens":30,"cache_creation":{"ephemeral_5m_input_tokens":20,"ephemeral_1h_input_tokens":10}}}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
    ].into_iter().map(event).collect::<String>();
    let server = Server::start(vec![Response::Sse(wire)]).await;
    let provider = AnthropicMessagesProvider::new(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        AnthropicMessagesConfig::new(server.endpoint.clone()),
    );
    let usage = provider
        .stream(request(), &mut |_| Ok(()))
        .await
        .unwrap()
        .usage
        .unwrap();
    assert_eq!(usage.cache_creation_input_tokens, Some(30));
    assert_eq!(usage.cache_creation_input_tokens_long, Some(10));
}

#[tokio::test]
async fn responses_failed_and_cancelled_never_emit_success() {
    for (status, kind) in [
        ("failed", ModelErrorKind::Other),
        ("cancelled", ModelErrorKind::Cancelled),
    ] {
        let response = json!({"status":status,"output":[]});
        let server = Server::start(vec![
            Response::Json(response.clone()),
            Response::Sse(event(
                json!({"type":format!("response.{status}"),"response":response}),
            )),
        ])
        .await;
        let provider = OpenAiResponsesProvider::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            OpenAiResponsesConfig::new(server.endpoint.clone()),
        );
        let mut events = vec![];
        for result in [
            provider.generate(request()).await,
            provider
                .stream(request(), &mut |part| {
                    events.push(part);
                    Ok(())
                })
                .await,
        ] {
            assert_eq!(result.unwrap_err().kind, kind);
        }
        assert!(!events
            .iter()
            .any(|part| matches!(part, StreamPart::Finish { .. })));
    }
}
