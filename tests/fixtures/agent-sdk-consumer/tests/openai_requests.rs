use kivio_agent_sdk::model::{
    openai::{self, OpenAiChatOptions},
    responses::{self, OpenAiResponsesOptions},
    GenerateRequest, ModelMessage, ModelRole, ModelTool,
};
use serde_json::{json, Value};

fn request(model: &str) -> GenerateRequest {
    GenerateRequest {
        model: model.into(),
        system: "Use the available tools when helpful".into(),
        messages: vec![ModelMessage::text(ModelRole::User, "Find the document")],
        tools: vec![],
        options: Default::default(),
        metadata: Default::default(),
    }
}

fn bodies(request: &GenerateRequest, endpoint: &str, extra_body: Value) -> [Value; 2] {
    let endpoint: reqwest::Url = endpoint.parse().unwrap();
    [
        openai::request_body(
            request,
            true,
            &OpenAiChatOptions {
                endpoint: Some(endpoint.clone()),
                temperature: Some(0.4),
                extra_body: extra_body.clone(),
                ..Default::default()
            },
        ),
        responses::request_body(
            request,
            true,
            &OpenAiResponsesOptions {
                endpoint: Some(endpoint),
                temperature: Some(0.4),
                extra_body,
                ..Default::default()
            },
        ),
    ]
}

const OFFICIAL: &str = "https://api.openai.com/v1";

#[test]
fn streaming_responses_omit_chat_usage_option_and_keep_obfuscation_control() {
    let request = request("gpt-5.4");
    let options = json!({"stream_options":{"include_usage":true,"include_obfuscation":false}});
    let [chat, responses] = bodies(&request, OFFICIAL, options.clone());
    assert_eq!(chat["stream_options"]["include_usage"], true);
    assert_eq!(
        responses["stream_options"],
        json!({"include_obfuscation":false})
    );
    let [_, relay] = bodies(&request, "https://gateway.example/v1", options.clone());
    assert_eq!(relay["stream_options"], options["stream_options"]);
}

#[test]
fn official_models_only_receive_supported_reasoning_and_sampling_defaults() {
    for (model, thinking, level, effort, sampling) in [
        ("gpt-4.1", false, None, None, true),
        ("o3-2025-04-16", false, None, None, false),
        ("gpt-5-mini", false, None, None, false),
        ("gpt-5.4", true, None, None, true),
        ("gpt-5.4", true, Some("high"), Some("high"), false),
        ("gpt-5.5", true, None, None, false),
        ("gpt-5.6-terra", false, None, Some("none"), true),
        ("gpt-6.1-terra", true, Some("high"), Some("high"), false),
        ("gpt-6.2-astra", false, None, Some("low"), false),
        ("gpt-6.2-sol", true, Some("minimal"), Some("low"), false),
    ] {
        let mut request = request(model);
        request.options.thinking_enabled = thinking;
        request.options.thinking_level = level.map(str::to_owned);
        let [chat, response] = bodies(
            &request,
            OFFICIAL,
            json!({"top_p":0.75,"top_logprobs":1,"include":["message.output_text.logprobs","web_search_call.action.sources"]}),
        );
        assert_eq!(
            chat["max_completion_tokens"], request.options.max_tokens,
            "{model}"
        );
        assert!(chat.get("max_tokens").is_none(), "{model}");
        assert_eq!(chat["reasoning_effort"].as_str(), effort, "{model}");
        assert_eq!(response["reasoning"]["effort"].as_str(), effort, "{model}");
        for body in [&chat, &response] {
            for field in ["temperature", "top_p", "top_logprobs"] {
                assert_eq!(body.get(field).is_some(), sampling, "{model}: {field}");
            }
        }
        if model == "gpt-4.1" {
            assert!(response.get("reasoning").is_none());
        } else if !sampling && !thinking {
            assert_eq!(response["reasoning"]["summary"], "auto", "{model}");
            assert!(
                response["include"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("reasoning.encrypted_content")),
                "{model}"
            );
        }
    }
}

#[test]
fn explicit_options_follow_official_wire_constraints_and_pass_through_gateways() {
    let mut request = request("gpt-6.2-sol");
    request.options.thinking_enabled = false;
    let [chat, response] = bodies(
        &request,
        "https://api.openai.com.gateway.example/v1",
        Value::Null,
    );
    assert_eq!(chat["reasoning_effort"], "none");
    assert_eq!(chat["max_tokens"], request.options.max_tokens);
    assert_eq!(response["reasoning"]["effort"], "none");
    assert_eq!(chat["temperature"], 0.4);
    assert_eq!(response["temperature"], 0.4);

    request.options.provider_options = json!({
        "temperature":0.9,
        "top_p":0.3,
        "reasoning":{"effort":"minimal","summary":"concise"},
        "reasoning_effort":"none",
        "include":["message.output_text.logprobs","web_search_call.action.sources"],
        "max_tokens":32,
        "max_output_tokens":2
    });
    let [chat, response] = bodies(&request, OFFICIAL, Value::Null);
    assert_eq!(chat["reasoning_effort"], "low");
    assert_eq!(response["reasoning"]["effort"], "low");
    assert_eq!(response["reasoning"]["summary"], "concise");
    assert_eq!(
        response["include"],
        json!(["web_search_call.action.sources"])
    );
    assert_eq!(chat["max_completion_tokens"], 32);
    assert!(chat.get("max_tokens").is_none());
    assert_eq!(response["max_output_tokens"], 16);
    for body in [&chat, &response] {
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
    }

    request.model = "gpt-5.4".into();
    request.options.provider_options =
        json!({"temperature":0.9,"reasoning":{"effort":"none"},"reasoning_effort":"none"});
    let [chat, response] = bodies(&request, OFFICIAL, Value::Null);
    assert_eq!(chat["temperature"], 0.9);
    assert_eq!(response["temperature"], 0.9);
}

#[test]
fn openai_responses_preserves_optional_tool_properties_and_minimum_output_budget() {
    let mut request = request("gpt-5.4");
    request.options.max_tokens = 1;
    let schema =
        json!({"type":"object","properties":{"directory":{"type":"string"}},"required":[]});
    request.tools.push(ModelTool {
        name: "list_files".into(),
        description: "List files from an optional directory".into(),
        input_schema: schema.clone(),
    });
    let [_, response] = bodies(&request, OFFICIAL, Value::Null);
    assert_eq!(response["tools"][0]["parameters"], schema);
    assert_eq!(response["tools"][0]["strict"], false);
    assert_eq!(response["max_output_tokens"], 16);
    let [_, response] = bodies(&request, "https://gateway.example/v1", Value::Null);
    assert!(response["tools"][0].get("strict").is_none());
    assert_eq!(response["max_output_tokens"], 1);
}

#[test]
fn official_responses_uses_supported_history_message_parts() {
    let mut request = request("gpt-5.4");
    request.messages.extend([
        ModelMessage {
            role: ModelRole::Assistant,
            content: vec![
                kivio_agent_sdk::model::MessagePart::Text {
                    text: "I found the document".into(),
                },
                kivio_agent_sdk::model::MessagePart::ImageUrl {
                    url: "https://example.org/generated.png".into(),
                    detail: None,
                },
                kivio_agent_sdk::model::MessagePart::Text {
                    text: "Here is a preview".into(),
                },
            ],
        },
        ModelMessage {
            role: ModelRole::User,
            content: vec![kivio_agent_sdk::model::MessagePart::ImageUrl {
                url: "https://example.org/page.png".into(),
                detail: None,
            }],
        },
    ]);
    let [_, response] = bodies(&request, OFFICIAL, Value::Null);
    let input = response["input"].as_array().unwrap();
    let assistant = input
        .iter()
        .find(|item| item["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["type"], "message");
    assert_eq!(assistant["status"], "completed");
    let first_id = assistant["id"].as_str().unwrap();
    assert!(first_id.starts_with("msg_") && first_id.len() <= 64);
    assert_eq!(
        assistant["content"][0],
        json!({"type":"output_text","text":"I found the document","annotations":[]})
    );
    assert_eq!(assistant["content"][1]["type"], "output_text");
    assert_eq!(assistant["content"][2]["text"], "Here is a preview");
    let image = &input.last().unwrap()["content"][0];
    assert_eq!(image["type"], "input_image");
    assert_eq!(image["detail"], "auto");
    request
        .messages
        .push(ModelMessage::text(ModelRole::Assistant, "Another preview"));
    let [_, next] = bodies(&request, OFFICIAL, Value::Null);
    let ids: Vec<_> = next["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["role"] == "assistant")
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], first_id);
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn chat_routing_uses_effective_effort_and_preserves_tool_history_route() {
    let endpoint = OFFICIAL.parse().unwrap();
    let relay = "https://api.openai.com.gateway.example/v1".parse().unwrap();
    let mut request = request("gpt-5.4-2026-03-05");
    let defaults = json!({"tools":[{"type":"function","function":{"name":"list_files"}}]});
    assert!(!openai::chat_requires_responses(
        &request, &endpoint, &defaults
    ));
    request.options.thinking_level = Some("high".into());
    assert!(openai::chat_requires_responses(
        &request, &endpoint, &defaults
    ));
    assert!(!openai::chat_requires_responses(
        &request, &relay, &defaults
    ));
    request.options.thinking_enabled = false;
    assert!(!openai::chat_requires_responses(
        &request, &endpoint, &defaults
    ));
    assert_eq!(
        bodies(&request, OFFICIAL, defaults.clone())[0]["reasoning_effort"],
        "none"
    );
    request.options.thinking_enabled = true;
    request.options.provider_options = json!({"reasoning_effort":"none"});
    assert!(!openai::chat_requires_responses(
        &request, &endpoint, &defaults
    ));

    request.model = "gpt-6.1-terra".into();
    assert!(openai::chat_requires_responses(
        &request, &endpoint, &defaults
    ));
    request.options.tool_choice = kivio_agent_sdk::model::ToolChoice::None;
    assert!(!openai::chat_requires_responses(
        &request, &endpoint, &defaults
    ));
    request.messages.push(ModelMessage {
        role: ModelRole::Assistant,
        content: vec![kivio_agent_sdk::model::MessagePart::ToolCall {
            id: "call_1".into(),
            name: "list_files".into(),
            arguments: json!({}),
            arguments_raw: "{}".into(),
            signature: None,
        }],
    });
    assert!(openai::chat_requires_responses(
        &request, &endpoint, &defaults
    ));

    request.options.tool_choice = kivio_agent_sdk::model::ToolChoice::Auto;
    request.options.builtin_web_search = true;
    request.model = "gpt-4.1".into();
    assert!(openai::chat_requires_responses(
        &request,
        &endpoint,
        &Value::Null
    ));
    request.model = "gpt-5-search-api".into();
    assert!(!openai::chat_requires_responses(
        &request,
        &endpoint,
        &Value::Null
    ));
}

#[test]
fn migrated_chat_overrides_keep_their_precedence_and_use_responses_shapes() {
    let mut request = request("gpt-5.4");
    request.tools.push(ModelTool {
        name: "typed_tool".into(),
        description: "Typed tool".into(),
        input_schema: json!({"type":"object"}),
    });
    request.options.provider_options = json!({
        "stop": null, "n": 1,
        "reasoning_effort":"high",
        "max_completion_tokens":64,
        "tool_choice":{"type":"function","function":{"name":"list_files"}},
        "response_format":{"type":"json_schema","json_schema":{"name":"listing","schema":{"type":"object"}}}
    });
    let defaults = json!({
        "stop": ["END"],
        "reasoning_effort":"medium",
        "verbosity":"low",
        "stream_options":{"include_usage":true},
        "tools":[{"type":"function","function":{"name":"list_files","parameters":{"type":"object"}}}],
    });
    let extra = responses::translate_chat_route_options(&mut request, &defaults).unwrap();
    let [_, response] = bodies(&request, OFFICIAL, extra);
    assert_eq!(response["reasoning"]["effort"], "high");
    assert_eq!(response["max_output_tokens"], 64);
    assert_eq!(
        response["tool_choice"],
        json!({"type":"function","name":"list_files"})
    );
    assert_eq!(
        response["tools"],
        json!([{"type":"function","name":"list_files","parameters":{"type":"object"},"strict":false}])
    );
    assert_eq!(response["text"]["verbosity"], "low");
    assert_eq!(
        response["text"]["format"],
        json!({"type":"json_schema","name":"listing","schema":{"type":"object"}})
    );
    for chat_field in [
        "reasoning_effort",
        "max_completion_tokens",
        "stream_options",
        "response_format",
        "verbosity",
    ] {
        assert!(response.get(chat_field).is_none(), "{chat_field}");
    }
    request.options.provider_options = json!({"stop":["END"]});
    let error = responses::translate_chat_route_options(&mut request, &defaults).unwrap_err();
    assert_eq!(
        error.kind,
        kivio_agent_sdk::model::ModelErrorKind::InvalidRequest
    );
    assert!(error.message.contains("\x60stop\x60"));
}

#[allow(dead_code)]
mod support;

#[tokio::test]
async fn explicitly_configured_sdk_chat_does_not_switch_endpoints_for_restricted_models() {
    use kivio_agent_sdk::model::{
        openai::{OpenAiChatConfig, OpenAiChatProvider},
        LanguageModelProvider, ModelErrorKind,
    };
    let server = support::Server::start(vec![]).await;
    let mut endpoint = server.endpoint.clone();
    endpoint.set_host(Some("api.openai.com")).unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .resolve(
            "api.openai.com",
            server.endpoint.socket_addrs(|| None).unwrap()[0],
        )
        .timeout(support::LIMIT)
        .build()
        .unwrap();
    let provider = OpenAiChatProvider::new(client, OpenAiChatConfig::new(endpoint));
    let mut request = request("gpt-6.2-sol");
    request.tools.push(ModelTool {
        name: "list_files".into(),
        description: "List the files".into(),
        input_schema: json!({"type":"object"}),
    });
    let error = provider.generate(request).await.unwrap_err();
    assert_eq!(error.kind, ModelErrorKind::InvalidRequest);
    assert!(error.message.contains("Responses endpoint"));
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn official_pro_models_deliver_buffered_responses_through_the_stream_sink() {
    use kivio_agent_sdk::model::{
        responses::{OpenAiResponsesConfig, OpenAiResponsesProvider},
        FinishKind, LanguageModelProvider, ModelErrorKind, StreamPart,
    };
    use support::{Response, Server};

    let server = Server::start_at(
        "/v1/responses",
        vec![
            Response::Json(json!({"status":"completed","output":[
                {"type":"reasoning","summary":[{"type":"summary_text","text":"Considered"}]},
                {"type":"message","content":[{"type":"output_text","text":"Searching"}]},
                {"type":"function_call","id":"fc_1","call_id":"call_1","name":"list_files","arguments":"{\"path\":\"docs\"}"}
            ],"usage":{"input_tokens":17,"output_tokens":9}})),
            Response::Json(json!({"status":"incomplete","incomplete_details":{"reason":"content_filter"},"output":[]})),
            Response::Sse(concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Streamed\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
            ).into()),
        ],
    ).await;
    let mut official = server.endpoint.clone();
    official.set_host(Some("api.openai.com")).unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .resolve(
            "api.openai.com",
            server.endpoint.socket_addrs(|| None).unwrap()[0],
        )
        .timeout(support::LIMIT)
        .build()
        .unwrap();
    let official =
        OpenAiResponsesProvider::new(client.clone(), OpenAiResponsesConfig::new(official));
    let mut pro_request = request("o3-pro");
    pro_request.tools.push(ModelTool {
        name: "list_files".into(),
        description: "List files at a path".into(),
        input_schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
    });
    let mut parts = Vec::new();
    let output = official
        .stream(pro_request, &mut |part| {
            parts.push(part);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(output.text, "Searching");
    assert_eq!(output.tool_calls[0].arguments, json!({"path":"docs"}));
    assert!(matches!(parts.as_slice(), [
        StreamPart::ReasoningDelta { delta: reasoning },
        StreamPart::TextDelta { delta: text },
        StreamPart::ToolCallStart { id, name },
        StreamPart::ToolCallDelta { id: delta_id, delta: args },
        StreamPart::ToolCallDone { call },
        StreamPart::ContextUsage { input_tokens: 17, output_tokens: 9 },
        StreamPart::Finish { reason, full },
    ] if reasoning == "Considered" && text == "Searching" && id == "call_1"
        && name == "list_files" && delta_id == id && args == r#"{"path":"docs"}"#
        && call.id == *id && call.arguments == output.tool_calls[0].arguments
        && reason.kind == FinishKind::ToolCalls && full == &output.text));

    parts.clear();
    let error = official
        .stream(request("o1-pro-2025-03-19"), &mut |part| {
            parts.push(part);
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(error.kind, ModelErrorKind::ContentModeration);
    assert!(parts.is_empty());

    let gateway =
        OpenAiResponsesProvider::new(client, OpenAiResponsesConfig::new(server.endpoint.clone()));
    let output = gateway
        .stream(request("o3-pro"), &mut |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(output.text, "Streamed");
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_ne!(requests[0].body.get("stream"), Some(&Value::Bool(true)));
    assert_ne!(requests[1].body.get("stream"), Some(&Value::Bool(true)));
    assert_eq!(requests[2].body["stream"], true);
}

#[test]
fn official_chat_search_model_keeps_its_native_search_options() {
    let mut request = request("gpt-5-search-api");
    request.options.builtin_web_search = true;
    assert!(!openai::chat_requires_responses(
        &request,
        &OFFICIAL.parse().unwrap(),
        &Value::Null
    ));
    let [chat, _] = bodies(&request, OFFICIAL, Value::Null);
    assert_eq!(chat["web_search_options"], json!({}));
    request.options.provider_options = json!({"web_search_options":{"search_context_size":"low"}});
    assert_eq!(
        bodies(&request, OFFICIAL, Value::Null)[0]["web_search_options"]["search_context_size"],
        "low"
    );
}
