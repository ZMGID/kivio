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
async fn responses_only_delivers_confirmed_tool_calls() {
    let item = json!({"id":"fc_a","call_id":"call_a","type":"function_call",
        "name":"lookup","arguments":"{}","status":"completed"});
    let added = event(json!({"type":"response.output_item.added","item":{
        "id":"fc_a","call_id":"call_a","name":"lookup","type":"function_call","arguments":""}}));
    let done = event(json!({"type":"response.output_item.done","item":item}));
    let partial = event(
        json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":"{}"}),
    );
    let completed = |output| {
        event(
            json!({"type":"response.completed","response":{"status":"completed","output":output}}),
        )
    };
    for (stream, expected) in [
        (added.clone() + &partial + &completed(json!([])), false),
        (done.clone() + &completed(json!([item])), true),
        (added + &completed(json!([item])), true),
        (completed(json!([item])), true),
    ] {
        let server = Server::start_at("/responses", vec![Response::Sse(stream)]).await;
        let provider = OpenAiResponsesProvider::new(
            reqwest::Client::new(),
            OpenAiResponsesConfig::new(server.endpoint.clone()),
        );
        let mut events = vec![];
        let result = provider
            .stream(request(), &mut |part| {
                events.push(part);
                Ok(())
            })
            .await;
        if expected {
            let output = result.unwrap();
            assert_eq!(output.finish_reason.unwrap().legacy_name(), "tool_calls");
            assert_eq!(output.tool_calls.len(), 1);
            assert_eq!(output.tool_calls[0].id, "call_a");
            assert_eq!(output.tool_calls[0].arguments, json!({}));
        } else {
            assert!(
                result.is_err(),
                "unconfirmed arguments must not be executable"
            );
        }
        assert_eq!(
            events
                .iter()
                .filter(|part| matches!(part, StreamPart::ToolCallDone { .. }))
                .count(),
            usize::from(expected)
        );
    }
}

#[tokio::test]
async fn responses_recovers_missing_visible_text_from_the_terminal_snapshot() {
    let terminal = event(
        json!({"type":"response.completed","response":{"status":"completed","output":[
            {"type":"message","id":"msg_a","content":[{"type":"output_text","text":"hello "},{"type":"refusal","refusal":"world"}]}
        ]}}),
    );
    for prefix in ["", "hel", "hello "] {
        let delta =
            event(json!({"type":"response.output_text.delta","item_id":"msg_a","delta":prefix}));
        let server = Server::start_at("/responses", vec![Response::Sse(delta + &terminal)]).await;
        let provider = OpenAiResponsesProvider::new(
            reqwest::Client::new(),
            OpenAiResponsesConfig::new(server.endpoint.clone()),
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
        assert_eq!(output.text, "hello world");
        assert_eq!(visible, output.text);
    }
}

#[tokio::test]
async fn responses_buffered_rejects_an_unfinished_call_in_a_completed_turn() {
    let unfinished = json!({"type":"function_call","id":"fc_a","call_id":"call_a",
        "name":"lookup","arguments":"{}","status":"in_progress"});
    let server = Server::start_at("/responses", vec![
        Response::Json(json!({"status":"completed","output":[unfinished]})),
        Response::Json(json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[unfinished]})),
    ]).await;
    let provider = OpenAiResponsesProvider::new(
        reqwest::Client::new(),
        OpenAiResponsesConfig::new(server.endpoint.clone()),
    );
    assert!(provider.generate(request()).await.is_err());
    let truncated = provider.generate(request()).await.unwrap();
    assert_eq!(truncated.finish_reason.unwrap().legacy_name(), "length");
    assert_eq!(truncated.tool_calls.len(), 1);
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
async fn responses_reasoning_replays_only_to_its_source() {
    use kivio_agent_sdk::model::{
        model_messages_from_openai_messages, openai_messages_from_model_messages,
        responses::ResponsesFormat, MessagePart,
    };
    let reasoning = json!({
        "type":"reasoning", "id":"rs_1", "summary":[], "encrypted_content":"opaque"
    });
    let call = json!({
        "type":"function_call", "id":"fc_1", "call_id":"call_1", "name":"echo", "arguments":"{}"
    });
    let first = json!({"status":"completed","output":[reasoning, call]});
    let final_response = || {
        Response::Json(json!({"status":"completed","output":[
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}
        ]}))
    };
    for (format, model, streamed, should_replay) in [
        (ResponsesFormat::Xai, "grok-4.7", true, true),
        (ResponsesFormat::OpenAi, "gpt-5.6", false, true),
        (ResponsesFormat::Xai, "grok-4.3", true, false),
    ] {
        let server = Server::start_at(
            "/v1/responses",
            vec![
                if streamed {
                    Response::Sse(
                        [
                            json!({"type":"response.output_item.added","item":call}),
                            json!({"type":"response.output_item.done","item":call}),
                            json!({"type":"response.completed","response":first}),
                        ]
                        .into_iter()
                        .map(event)
                        .collect(),
                    )
                } else {
                    Response::Json(first.clone())
                },
                final_response(),
                final_response(),
                final_response(),
                final_response(),
            ],
        )
        .await;
        let other = Server::start_at("/v1/responses", vec![final_response()]).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let provider = |server: &Server, format, provider_id: &str| {
            let mut config = OpenAiResponsesConfig::new(server.endpoint.clone());
            config.options.format = format;
            config.options.provider_id = Some(provider_id.into());
            OpenAiResponsesProvider::new(client.clone(), config)
        };
        let mut next = request();
        next.model = model.into();
        let original = provider(&server, format, "original");
        let output = if streamed {
            original.stream(next.clone(), &mut |_| Ok(())).await
        } else {
            original.generate(next.clone()).await
        }
        .unwrap();
        let history = openai_messages_from_model_messages(&[output.replay_message()]);
        next.messages
            .extend(model_messages_from_openai_messages(history));
        next.messages.push(ModelMessage {
            role: ModelRole::Tool,
            content: vec![MessagePart::ToolResult {
                tool_call_id: "call_1".into(),
                content: "ok".into(),
                is_error: false,
                artifacts: vec![],
            }],
        });
        assert_eq!(original.generate(next.clone()).await.unwrap().text, "done");
        provider(&server, format, "other")
            .generate(next.clone())
            .await
            .unwrap();
        let mut switched_model = next.clone();
        switched_model.model = "another-model".into();
        original.generate(switched_model).await.unwrap();
        let switched_format = match format {
            ResponsesFormat::OpenAi => ResponsesFormat::Xai,
            ResponsesFormat::Xai => ResponsesFormat::OpenAi,
        };
        provider(&server, switched_format, "original")
            .generate(next.clone())
            .await
            .unwrap();
        provider(&other, format, "original")
            .generate(next)
            .await
            .unwrap();
        let requests = server.requests.lock().unwrap();
        let replay = requests[1].body["input"].as_array().unwrap();
        let reasoning_items: Vec<_> = replay
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .collect();
        if should_replay {
            assert_eq!(reasoning_items, vec![&reasoning], "{model}");
            let index = replay.iter().position(|item| item == &reasoning).unwrap();
            assert_eq!(replay[index + 1]["type"], "function_call");
            assert_eq!(replay[index + 2]["type"], "function_call_output");
        } else {
            assert!(reasoning_items.is_empty(), "{model}");
        }
        let other_requests = other.requests.lock().unwrap();
        for request in requests[2..].iter().chain(other_requests.iter()) {
            let input = request.body["input"].as_array().unwrap();
            assert!(
                input.iter().all(|item| item["type"] != "reasoning"),
                "{model}"
            );
            assert!(
                input
                    .iter()
                    .any(|item| item["type"] == "function_call_output"),
                "{model}"
            );
        }
    }
}

#[tokio::test]
async fn deepseek_responses_replays_its_plaintext_reasoning() {
    use kivio_agent_sdk::model::{
        model_messages_from_openai_messages, openai_messages_from_model_messages,
        responses::{self, OpenAiResponsesOptions},
        MessagePart,
    };
    let reasoning = json!({
        "type":"reasoning", "id":"rs_ds",
        "content":[{"type":"reasoning_text","text":"Look for the result."}],
        "summary":[]
    });
    let message = json!({
        "type":"message", "id":"msg_ds", "role":"assistant",
        "content":[{"type":"output_text","text":"Here it is."}]
    });
    let first = json!({"status":"completed","output":[reasoning, message]});
    let sse = [
        json!({"type":"response.reasoning_text.delta","item_id":"rs_ds","delta":"Look for the result."}),
        json!({"type":"response.output_item.done","item":reasoning}),
        json!({"type":"response.output_text.delta","item_id":"msg_ds","delta":"Here it is."}),
        json!({"type":"response.completed","response":first}),
    ].into_iter().map(event).collect::<String>();
    let endpoint = "https://api.deepseek.com/responses".parse().unwrap();
    let options = OpenAiResponsesOptions {
        provider_id: Some("deepseek".into()),
        endpoint: Some(endpoint),
        prompt_cache_key: Some("conversation".into()),
        prompt_cache_retention: Some("24h".into()),
        ..Default::default()
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for streamed in [false, true] {
        // The transport fixture records HTTP payloads on loopback. The codec
        // receives the official logical endpoint, as in the desktop transport.
        let server = Server::start_at(
            "/responses",
            vec![
                if streamed {
                    Response::Sse(sse.clone())
                } else {
                    Response::Json(first.clone())
                },
                Response::Json(first.clone()),
                Response::Json(first.clone()),
                Response::Json(first.clone()),
            ],
        )
        .await;
        let post = |request: &GenerateRequest, stream, options: &OpenAiResponsesOptions| {
            client
                .post(server.endpoint.clone())
                .json(&responses::request_body(request, stream, options))
                .send()
        };
        let mut request = request();
        request.model = "deepseek-flash".into();
        request.options.thinking_enabled = true;
        request.options.thinking_level = Some("low".into());
        let raw = post(&request, streamed, &options)
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let mut output = responses::decode_responses_body_at_endpoint(
            &raw,
            "DeepSeek",
            &request.model,
            options.endpoint.as_ref(),
        )
        .unwrap();
        responses::bind_output_provenance(
            &mut output,
            options.format,
            options.provider_id.as_deref(),
            options.endpoint.as_ref(),
        );
        let history = openai_messages_from_model_messages(&[output.replay_message()]);
        let mut restored = model_messages_from_openai_messages(history);
        let image = |data: &str| MessagePart::Image {
            mime_type: "image/png".into(),
            data: data.into(),
            path: None,
            detail: None,
        };
        restored[0].content.push(image("YXNzaXN0YW50"));
        request.messages.extend(restored);
        request.messages.push(ModelMessage {
            role: ModelRole::User,
            content: vec![image("dXNlcg==")],
        });
        assert!(!kivio_agent_sdk::model::openai::request_body(
            &request,
            false,
            &Default::default()
        )
        .to_string()
        .contains("Look for the result."));
        post(&request, false, &options)
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        for target in [
            "https://api.openai.com/v1/responses",
            "https://gateway.example/responses",
        ] {
            post(
                &request,
                false,
                &OpenAiResponsesOptions {
                    endpoint: Some(target.parse().unwrap()),
                    ..options.clone()
                },
            )
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        }
        let recorded = server.requests.lock().unwrap();
        let input = recorded[1].body["input"].as_array().unwrap();
        let reasoning_items: Vec<_> = input
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .collect();
        assert_eq!(reasoning_items, vec![&reasoning]);
        assert_eq!(output.reasoning.as_deref(), Some("Look for the result."));
        assert!(input
            .iter()
            .any(|item| item["role"] == "user" && item["content"][0]["type"] == "input_image"));
        assert!(input.iter().all(|item| item["role"] != "assistant"
            || item["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|part| part["type"] != "input_image")));
        for field in ["include", "prompt_cache_key", "prompt_cache_retention"] {
            assert!(recorded[0].body.get(field).is_none(), "{field}");
        }
        assert_eq!(recorded[0].body["reasoning"], json!({"effort":"low"}));
        for request in &recorded[2..] {
            assert!(!request.body.to_string().contains("Look for the result."));
        }
    }
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
