use kivio_agent_sdk::{
    agent::{
        history::request_from_history,
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        recovery::{RecoveryDecision, RecoveryInput, RecoverySnapshot},
        run_agent_loop,
        session::{NoSessionPolicy, SessionPolicy},
        types::AgentRunStatus,
    },
    model::{
        openai::{request_body, OpenAiChatOptions},
        GenerateOptions, GenerateOutput, MessagePart, ModelCapabilities, ModelError,
        ModelErrorKind, ModelMessage, ModelRole, PendingToolCall, ToolChoice,
    },
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Events, Host};
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::Mutex};

struct Model {
    outputs: Mutex<VecDeque<Result<GenerateOutput, ModelError>>>,
    calls: Mutex<Vec<ModelCall>>,
    bodies: Mutex<Vec<Value>>,
}

impl Model {
    fn new(outputs: Vec<Result<GenerateOutput, ModelError>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            calls: Mutex::new(vec![]),
            bodies: Mutex::new(vec![]),
        }
    }
}

impl ModelRuntime for Model {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            sink.request_started(&call);
            let request = request_from_history(
                &call.history,
                &call.model,
                call.tools.clone(),
                call.options.clone(),
                call.metadata.clone(),
            );
            self.bodies.lock().unwrap().push(request_body(
                &request,
                true,
                &OpenAiChatOptions {
                    temperature: request.options.temperature,
                    extra_body: json!({"temperature": 0.1, "vendor": "provider"}),
                    ..Default::default()
                },
            ));
            self.calls.lock().unwrap().push(call);
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model call")
        })
    }
}

fn options(choice: ToolChoice) -> GenerateOptions {
    GenerateOptions {
        temperature: Some(0.7),
        max_tokens: 321,
        thinking_enabled: true,
        thinking_level: Some("high".into()),
        builtin_web_search: true,
        tool_choice: choice,
        provider_options: json!({
            "vendor": {"effort": "call"},
            "tools": [{"type": "web_search"}],
            "tool_choice": "auto"
        }),
    }
}

fn assert_options(call: &ModelCall, choice: ToolChoice) {
    let expected = options(choice);
    assert_eq!(
        serde_json::to_value(&call.options).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[tokio::test]
async fn planning_keeps_all_options_and_the_callers_tool_choice() {
    for choice in [ToolChoice::Auto, ToolChoice::None] {
        let model = Model::new(vec![Ok(GenerateOutput::text("answer".into(), None))]);
        let mut config = config(&model, initial_history());
        config.options = options(choice);
        config.model = "gpt-unknown-model".into();
        let control = Control::default();
        let result = run_agent_loop(
            config,
            &Host,
            &EchoTool::new(true),
            &mut NoSessionPolicy,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap();
        assert_eq!(result.content, "answer");
        let calls = model.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_options(&calls[0], choice);
        let bodies = model.bodies.lock().unwrap();
        assert_eq!(bodies[0]["temperature"], 0.7);
        assert_eq!(bodies[0]["max_tokens"], 321);
        assert_eq!(bodies[0]["vendor"], json!({"effort": "call"}));
        if choice == ToolChoice::None {
            assert!(bodies[0].get("tools").is_none());
            assert_eq!(bodies[0]["tool_choice"], "none");
        } else {
            assert_eq!(bodies[0]["tools"], json!([{"type": "web_search"}]));
        }
    }
}

#[tokio::test]
async fn plain_answer_preserves_the_callers_tool_prohibition() {
    let model = Model::new(vec![Ok(GenerateOutput::text("answer".into(), None))]);
    let mut config = config(&model, initial_history());
    config.tools.clear();
    config.options = options(ToolChoice::None);
    let control = Control::default();
    let result = run_agent_loop(
        config,
        &Host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "answer");
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_options(&calls[0], ToolChoice::None);
    assert!(calls[0].tools.is_empty());
    let bodies = model.bodies.lock().unwrap();
    assert!(bodies[0].get("tools").is_none());
    assert_eq!(bodies[0]["tool_choice"], "none");
}

#[tokio::test]
async fn plain_auto_allows_native_search_without_function_calling_capability() {
    let model = Model::new(vec![Ok(GenerateOutput::text("search answer".into(), None))]);
    let mut config = config(&model, initial_history());
    config.tools.clear();
    config.capabilities.tools = Some(false);
    config.capabilities.builtin_web_search = Some(true);
    config.options = options(ToolChoice::Auto);
    let control = Control::default();
    let result = run_agent_loop(
        config,
        &Host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "search answer");
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_options(&calls[0], ToolChoice::Auto);
    assert!(calls[0].tools.is_empty());
    let bodies = model.bodies.lock().unwrap();
    assert_eq!(bodies[0]["tools"], json!([{"type": "web_search"}]));
    assert_eq!(bodies[0]["tool_choice"], "auto");
}

struct Retry;
impl SessionPolicy for Retry {
    fn recover(&mut self, run: RecoverySnapshot<'_>, _: &ModelError) -> RecoveryDecision {
        RecoveryDecision::Retry(RecoveryInput::Reduced(run.history.to_vec()))
    }
}

#[tokio::test]
async fn recovery_forces_no_tools_and_retains_options_after_completed_tool_work() {
    let mut tool = GenerateOutput::text(String::new(), None);
    tool.tool_calls = vec![PendingToolCall {
        id: "echo-1".into(),
        function_name: "echo".into(),
        arguments: json!({"text": "tool result"}),
        arguments_raw: r#"{"text":"tool result"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    }];
    let model = Model::new(vec![
        Ok(tool),
        Err(ModelError::with_kind(
            "upstream unavailable",
            ModelErrorKind::Transport,
        )),
        Ok(GenerateOutput::text("recovered".into(), None)),
    ]);
    let mut config = config(&model, initial_history());
    config.options = options(ToolChoice::Auto);
    let control = Control::default();
    let executor = EchoTool::new(true);
    let result = run_agent_loop(
        config,
        &Host,
        &executor,
        &mut Retry,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "recovered");
    assert_eq!(result.status, AgentRunStatus::Recovered);
    assert_eq!(executor.call_count(), 1);
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_options(&calls[0], ToolChoice::Auto);
    assert_options(&calls[1], ToolChoice::Auto);
    assert_options(&calls[2], ToolChoice::None);
    assert!(calls[2].tools.is_empty());
    let bodies = model.bodies.lock().unwrap();
    assert!(bodies[2].get("tools").is_none());
    assert_eq!(bodies[2]["tool_choice"], "none");
}

#[tokio::test]
async fn known_unsupported_requests_fail_before_entering_the_provider() {
    for capability in ["reasoning", "builtin_web_search", "vision", "native_search"] {
        let model = Model::new(vec![]);
        let mut config = config(&model, initial_history());
        config.options = options(ToolChoice::Auto);
        match capability {
            "reasoning" => config.capabilities.reasoning = Some(false),
            "builtin_web_search" | "native_search" => {
                config.capabilities.builtin_web_search = Some(false);
                config.options.builtin_web_search = capability == "builtin_web_search";
            }
            "vision" => {
                config.capabilities.vision = Some(false);
                config
                    .history
                    .push(kivio_agent_sdk::agent::RuntimeMessage::model(
                        ModelMessage {
                            role: ModelRole::User,
                            content: vec![MessagePart::Image {
                                mime_type: "image/png".into(),
                                data: "aW1hZ2U=".into(),
                                path: None,
                                detail: None,
                            }],
                        },
                    ));
            }
            _ => unreachable!(),
        }
        let control = Control::default();
        let error = run_agent_loop(
            config,
            &Host,
            &EchoTool::new(true),
            &mut NoSessionPolicy,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap_err();
        let kivio_agent_sdk::agent::error::AgentRunError::Failed { cause, partial } = error else {
            panic!("capability failure must retain a partial result");
        };
        assert!(
            matches!(cause, kivio_agent_sdk::agent::error::AgentFailure::Model { source, .. }
            if source.kind == ModelErrorKind::InvalidRequest)
        );
        assert!(partial.generated_history.is_empty());
        assert!(model.calls.lock().unwrap().is_empty(), "{capability}");
    }
}

#[tokio::test]
async fn known_unsupported_tools_use_the_existing_plain_fallback() {
    let model = Model::new(vec![Ok(GenerateOutput::text("plain answer".into(), None))]);
    let mut config = config(&model, initial_history());
    config.capabilities.tools = Some(false);
    config.options = options(ToolChoice::Auto);
    let control = Control::default();
    let executor = EchoTool::new(true);
    let result = run_agent_loop(
        config,
        &Host,
        &executor,
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "plain answer");
    assert_eq!(executor.call_count(), 0);
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_options(&calls[0], ToolChoice::None);
    assert!(calls[0].tools.is_empty());
}

#[tokio::test]
async fn non_reasoning_models_accept_the_desktop_default_without_a_reasoning_parameter() {
    let model = Model::new(vec![Ok(GenerateOutput::text("answer".into(), None))]);
    let mut config = config(&model, initial_history());
    config.options.thinking_enabled = true;
    config.options.thinking_level = None;
    config.capabilities.reasoning = Some(false);
    let control = Control::default();
    let result = run_agent_loop(
        config,
        &Host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "answer");
    assert!(model.bodies.lock().unwrap()[0]
        .get("reasoning_effort")
        .is_none());
}

#[tokio::test]
async fn tool_prohibition_disables_native_search_even_when_capabilities_are_false() {
    let model = Model::new(vec![Ok(GenerateOutput::text("answer".into(), None))]);
    let mut config = config(&model, initial_history());
    config.capabilities = ModelCapabilities {
        tools: Some(false),
        reasoning: Some(false),
        builtin_web_search: Some(false),
        vision: Some(false),
    };
    config.options = options(ToolChoice::None);
    config.options.thinking_enabled = false;
    config.options.thinking_level = Some("off".into());
    let control = Control::default();
    let result = run_agent_loop(
        config,
        &Host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "answer");
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let bodies = model.bodies.lock().unwrap();
    assert!(bodies[0].get("tools").is_none());
    assert_eq!(bodies[0]["tool_choice"], "none");
}
