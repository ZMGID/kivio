use kivio_agent_sdk::model::{
    openai::{request_body, OpenAiChatOptions},
    GenerateOptions, GenerateRequest, ModelCapabilities, ToolChoice,
};
use serde_json::json;

#[test]
fn resolved_options_override_provider_defaults_and_tool_prohibition_wins_last() {
    let mut request = GenerateRequest {
        model: "host-selected-model".into(),
        system: String::new(),
        messages: vec![],
        tools: vec![],
        options: GenerateOptions {
            temperature: Some(0.6),
            max_tokens: 456,
            thinking_level: Some("high".into()),
            tool_choice: ToolChoice::None,
            provider_options: json!({
                "vendor": {"cache": "call"},
                "tools": [{"type": "web_search"}],
                "tool_choice": "auto"
            }),
            ..Default::default()
        },
        metadata: Default::default(),
    };
    let defaults = OpenAiChatOptions {
        temperature: request.options.temperature,
        extra_body: json!({
            "temperature": 0.2,
            "max_tokens": 123,
            "reasoning_effort": "low",
            "vendor": {"cache": "provider"},
            "tools": [{"type": "function"}],
            "tool_choice": "required"
        }),
        ..Default::default()
    };
    let body = request_body(&request, true, &defaults);
    assert_eq!(body["temperature"], 0.6);
    assert_eq!(body["max_tokens"], 456);
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["vendor"], json!({"cache": "call"}));
    assert!(body.get("tools").is_none());
    assert_eq!(body["tool_choice"], "none");

    request.options.tool_choice = ToolChoice::Auto;
    request.options.max_tokens = 0;
    let body = request_body(&request, false, &defaults);
    assert_eq!(body["tools"], json!([{"type": "web_search"}]));
    assert_eq!(body["tool_choice"], "auto");
    assert!(body.get("max_tokens").is_none());
}

#[test]
fn unknown_capabilities_are_distinct_from_unsupported() {
    let unknown: ModelCapabilities = serde_json::from_value(json!({})).unwrap();
    assert_eq!(unknown, ModelCapabilities::default());
    let unsupported: ModelCapabilities = serde_json::from_value(json!({
        "tools": false,
        "reasoning": false,
        "vision": false,
        "builtin_web_search": false
    }))
    .unwrap();
    assert_ne!(unknown, unsupported);
    assert_eq!(unsupported.tools, Some(false));
    assert_eq!(unsupported.reasoning, Some(false));
    assert_eq!(unsupported.vision, Some(false));
    assert_eq!(unsupported.builtin_web_search, Some(false));
}
