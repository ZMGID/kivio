mod support;

use kivio_agent_sdk::{
    agent::{RuntimeMessage, SessionHistory, SessionSnapshot},
    model::{
        gemini::{request_body, GeminiConfig, GeminiOptions, GeminiProvider},
        openai_messages_from_generate_request, GenerateRequest, LanguageModelProvider, MessagePart,
        ModelMessage, ModelRole, ModelTool, StreamPart, ToolChoice,
    },
};
use serde_json::json;
use support::{event, Response, Server};

fn request() -> GenerateRequest {
    GenerateRequest {
        model: "gemini-3.8-flash".into(),
        system: String::new(),
        messages: vec![ModelMessage::text(
            ModelRole::User,
            "Search then echo hello",
        )],
        tools: vec![ModelTool {
            name: "echo".into(),
            description: "Echo text".into(),
            input_schema: json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
        }],
        options: kivio_agent_sdk::model::GenerateOptions {
            builtin_web_search: true,
            ..Default::default()
        },
        metadata: Default::default(),
    }
}

#[test]
fn combination_config_uses_effective_tools_and_disable_wins() {
    for overrides in [
        json!(null),
        json!({"toolConfig":{"functionCallingConfig":{"mode":"AUTO"},"retrievalConfig":{"languageCode":"zh"}}}),
        json!({"tools":[{"functionDeclarations":[{"name":"echo"}]},{"googleSearch":{}}]}),
    ] {
        let mut call = request();
        call.options.provider_options = overrides.clone();
        let body = request_body(&call, &GeminiOptions::default());
        assert_eq!(body["toolConfig"]["includeServerSideToolInvocations"], true);
        assert_eq!(
            body["toolConfig"]["functionCallingConfig"]["mode"],
            "VALIDATED"
        );
        if overrides["toolConfig"]["retrievalConfig"].is_object() {
            assert_eq!(body["toolConfig"]["retrievalConfig"]["languageCode"], "zh");
        }
        call.options.tool_choice = ToolChoice::None;
        let disabled = request_body(&call, &GeminiOptions::default());
        assert!(disabled.get("tools").is_none());
        assert_eq!(
            disabled["toolConfig"],
            json!({"functionCallingConfig":{"mode":"NONE"}})
        );
    }
    let mut functions_only = request();
    functions_only.options.builtin_web_search = false;
    let body = request_body(&functions_only, &GeminiOptions::default());
    assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
    assert!(body["toolConfig"]
        .get("includeServerSideToolInvocations")
        .is_none());
    let mut search_only = request();
    search_only.tools.clear();
    assert!(request_body(&search_only, &GeminiOptions::default())
        .get("toolConfig")
        .is_none());
    for mode in ["ANY", "NONE"] {
        let mut call = request();
        call.options.provider_options =
            json!({"toolConfig":{"functionCallingConfig":{"mode":mode}}});
        assert_eq!(
            request_body(&call, &GeminiOptions::default())["toolConfig"]["functionCallingConfig"]
                ["mode"],
            mode
        );
    }
}

#[tokio::test]
async fn combined_tool_context_survives_storage_and_http_continuation() {
    let parts = json!([
        {"text":"Search", "thoughtSignature":"text-signature"},
        {"text":"ing"},
        {"toolCall":{"id":"search-1","toolType":"GOOGLE_SEARCH_WEB","args":{"queries":["example"]}},"thoughtSignature":"search-signature"},
        {"toolResponse":{"id":"search-1","toolType":"GOOGLE_SEARCH_WEB","response":{"result":"found"}},"thoughtSignature":"result-signature"},
        {"thought":true,"text":"", "thoughtSignature":"empty-thought-signature"},
        {"functionCall":{"id":"echo-1","name":"echo","args":{"text":"hello"}},"thoughtSignature":"function-signature"},
        {"functionCall":{"id":"echo-2","name":"echo","args":{"text":"again"}}}
    ]);
    let first =
        json!({"candidates":[{"content":{"role":"model","parts":parts},"finishReason":"STOP"}]});
    let answer =
        json!({"candidates":[{"content":{"parts":[{"text":"done"}]},"finishReason":"STOP"}]});
    for streamed in [false, true] {
        let method = if streamed {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        let server = Server::start_paths(
            "/gemini",
            vec![format!("/gemini/models/gemini-3.8-flash:{method}")],
            if streamed {
                // Separate chunks protect ordering and empty signed parts during streaming.
                let mut wire = String::new();
                for part in parts.as_array().unwrap() {
                    wire.push_str(&event(json!({"candidates":[{"content":{"parts":[part]}}]})));
                }
                wire.push_str(&event(json!({"candidates":[{"finishReason":"STOP"}]})));
                vec![Response::Sse(wire), Response::Sse(event(answer.clone()))]
            } else {
                vec![
                    Response::Json(first.clone()),
                    Response::Json(answer.clone()),
                ]
            },
        )
        .await;
        let mut config = GeminiConfig::new(server.endpoint.clone());
        config.options.provider_id = Some("account-a".into());
        let provider = GeminiProvider::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            config,
        );
        let mut events = vec![];
        let output = if streamed {
            provider
                .stream(request(), &mut |part| {
                    events.push(part);
                    Ok(())
                })
                .await
                .unwrap()
        } else {
            provider.generate(request()).await.unwrap()
        };
        assert_eq!(
            output.tool_calls.len(),
            2,
            "server tools must never execute locally"
        );
        assert_eq!(output.tool_calls[0].function_name, "echo");
        assert_eq!(output.replay_message().text_content(), "Searching");
        if streamed {
            assert_eq!(
                events
                    .iter()
                    .filter(|part| matches!(part, StreamPart::ToolCallDone { .. }))
                    .count(),
                2
            );
        }
        assert!(
            output.tool_calls[1].signature.is_none(),
            "signatures must not move between response parts"
        );
        let runtime = RuntimeMessage::model(output.replay_message());
        let legacy = runtime.to_legacy();
        let imported = RuntimeMessage::from_legacy(legacy[0].clone()).unwrap();
        let results = RuntimeMessage::model(ModelMessage {
            role: ModelRole::Tool,
            content: ["echo-1", "echo-2"]
                .into_iter()
                .map(|id| MessagePart::ToolResult {
                    tool_call_id: id.into(),
                    content: "hello".into(),
                    is_error: false,
                    artifacts: vec![],
                })
                .collect(),
        });
        let history = SessionHistory::from(vec![imported, results]);
        let saved = serde_json::to_vec(&history.snapshot().unwrap()).unwrap();
        let restored = serde_json::from_slice::<SessionSnapshot>(&saved)
            .unwrap()
            .into_history();
        let mut next = request();
        next.messages.extend(
            restored
                .context()
                .iter()
                .filter_map(RuntimeMessage::model_message)
                .cloned(),
        );
        let result = if streamed {
            provider
                .stream(next.clone(), &mut |_| Ok(()))
                .await
                .unwrap()
        } else {
            provider.generate(next.clone()).await.unwrap()
        };
        assert_eq!(result.text, "done");
        let requests = server.requests.lock().unwrap();
        let replay = &requests[1].body["contents"][1]["parts"];
        assert_eq!(
            replay, &parts,
            "all signed native parts must replay in order after restoration"
        );
        assert_eq!(
            requests[1].body["contents"][2]["parts"][0]["functionResponse"]["id"],
            "echo-1"
        );
        drop(requests);
        // Native data must not leak when either origin changes, or to another protocol.
        for (model, account) in [
            ("gemini-4-test", "account-a"),
            ("gemini-3.8-flash", "account-b"),
        ] {
            next.model = model.into();
            let body = request_body(
                &next,
                &GeminiOptions {
                    provider_id: Some(account.into()),
                    ..Default::default()
                },
            );
            let text = body.to_string();
            assert!(!text.contains("Signature"));
            assert!(!text.contains("toolResponse"));
            assert!(!text.contains("toolCall\""));
            assert_eq!(
                body["contents"][1]["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<String>(),
                "Searching"
            );
        }
        let chat = openai_messages_from_generate_request(&next);
        assert_eq!(chat[1]["content"], "Searching");
        for wire in [
            json!(chat),
            kivio_agent_sdk::model::responses::request_body(&next, false, &Default::default()),
            kivio_agent_sdk::model::anthropic::request_body(&next, false, &Default::default()),
        ] {
            let wire = wire.to_string();
            assert!(!wire.contains("gemini_content"));
            assert!(!wire.contains("search-signature"));
            assert!(!wire.contains("GOOGLE_SEARCH_WEB"));
        }
    }
}

#[tokio::test]
async fn older_gemini_rejects_combined_tools_before_sending() {
    let server = Server::start_paths("/gemini", vec!["/gemini/models/gemini-2.5-flash:generateContent".into()], vec![
        Response::Json(json!({"candidates":[{"content":{"parts":[{"text":"unexpected"}]},"finishReason":"STOP"}]})),
    ]).await;
    let provider = GeminiProvider::new(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        GeminiConfig::new(server.endpoint.clone()),
    );
    let mut call = request();
    call.model = "gemini-2.5-flash".into();
    let error = provider
        .generate(call)
        .await
        .expect_err("unsupported combinations must be explained locally");
    assert!(error.message.contains("Gemini 3"));
    assert!(server.requests.lock().unwrap().is_empty());
}
