use kivio_agent_sdk::{
    agent::{
        control::{AgentRunHandle, InputAdmission},
        history::request_from_history,
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        run_agent_loop,
        session::{PolicyFuture, RunSnapshot, SessionPolicy, TurnDecision},
        AgentFailure, AgentRunError, RuntimeMessage, SteeringMessage,
    },
    model::{GenerateOutput, ModelCapabilities, ModelError, ModelRole, PendingToolCall},
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Events, Host};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct Model {
    outputs: Mutex<VecDeque<GenerateOutput>>,
    calls: Mutex<Vec<ModelCall>>,
    log: Arc<Mutex<Vec<&'static str>>>,
}
impl Model {
    fn new(outputs: Vec<GenerateOutput>, log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            calls: Mutex::new(vec![]),
            log,
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
            self.log.lock().unwrap().push("provider");
            sink.request_started(&call);
            self.calls.lock().unwrap().push(call);
            Ok(self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected provider request"))
        })
    }
}
fn text(text: &str) -> GenerateOutput {
    GenerateOutput::text(text.into(), None)
}
fn tool() -> GenerateOutput {
    let mut output = text("");
    output.tool_calls.push(PendingToolCall {
        id: "echo-1".into(),
        function_name: "echo".into(),
        arguments: json!({"text":"executed"}),
        arguments_raw: r#"{"text":"executed"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    output
}

struct Hooks {
    log: Arc<Mutex<Vec<&'static str>>>,
    finish_count: usize,
    fail: Option<&'static str>,
    stop: bool,
    switch_model: bool,
    capabilities: Option<ModelCapabilities>,
}
impl Hooks {
    fn new(log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            log,
            finish_count: 0,
            fail: None,
            stop: false,
            switch_model: false,
            capabilities: None,
        }
    }
}
impl SessionPolicy for Hooks {
    fn prepare_request<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        call: &'a mut ModelCall,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.log.lock().unwrap().push("prepare");
            if self.fail == Some("prepare_request") {
                return Err("prepare failed".into());
            }
            call.options.temperature = Some(0.3);
            if self.switch_model {
                call.model = "other-model".into();
            }
            call.capabilities = self.capabilities.clone();
            call.history
                .push(RuntimeMessage::system("request-only system"));
            Ok(())
        })
    }
    fn transform_context<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move {
            self.log.lock().unwrap().push("transform");
            if self.fail == Some("transform_context") {
                return Err("transform failed".into());
            }
            let mut view = history.to_vec();
            view.push(RuntimeMessage::text(ModelRole::User, "temporary view"));
            Ok(view)
        })
    }
    fn finish_turn<'a>(
        &'a mut self,
        run: RunSnapshot<'a>,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async move {
            self.log.lock().unwrap().push("finish");
            self.finish_count += 1;
            assert!(matches!(
                run.history.last().and_then(RuntimeMessage::role),
                Some(ModelRole::Assistant | ModelRole::Tool)
            ));
            if self.fail == Some("finish_turn") {
                return Err("finish failed".into());
            }
            Ok(if self.stop {
                TurnDecision::Stop
            } else {
                TurnDecision::Default
            })
        })
    }
}

#[tokio::test]
async fn request_hooks_run_in_order_without_changing_canonical_history() {
    let log = Arc::default();
    let model = Model::new(vec![text("answer")], Arc::clone(&log));
    let mut policy = Hooks::new(Arc::clone(&log));
    let control = Control::default();
    let result = run_agent_loop(
        config(&model, initial_history()),
        &Host,
        &EchoTool::new(true),
        &mut policy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        vec!["prepare", "transform", "provider", "finish"]
    );
    let calls = model.calls.lock().unwrap();
    let call = &calls[0];
    assert_eq!(call.options.temperature, Some(0.3));
    assert_eq!(
        call.history.last().unwrap().text_content(),
        "temporary view"
    );
    let request = request_from_history(
        &call.history,
        &call.model,
        call.tools.clone(),
        call.options.clone(),
        call.metadata.clone(),
    );
    assert_eq!(
        request.system,
        "Use echo when needed.\n\nrequest-only system"
    );
    assert_eq!(result.history.context().len(), 3);
    assert!(!result
        .history
        .context()
        .iter()
        .any(|entry| entry.text_content().contains("temporary")
            || entry.text_content().contains("request-only")));
}

#[tokio::test]
async fn finish_turn_runs_once_after_tools_and_can_stop_without_another_request() {
    let log = Arc::default();
    let model = Model::new(vec![tool()], Arc::clone(&log));
    let mut policy = Hooks::new(Arc::clone(&log));
    policy.stop = true;
    let executor = EchoTool::new(true);
    let control = Control::default();
    let result = run_agent_loop(
        config(&model, initial_history()),
        &Host,
        &executor,
        &mut policy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(policy.finish_count, 1);
    assert_eq!(executor.call_count(), 1);
    assert_eq!(result.tool_records.len(), 1);
    assert_eq!(result.history.context().len(), 4);
    assert_eq!(
        *log.lock().unwrap(),
        vec!["prepare", "transform", "provider", "finish"]
    );
}

#[tokio::test]
async fn hook_failures_keep_their_source_and_completed_assistant_facts() {
    for hook in ["prepare_request", "transform_context", "finish_turn"] {
        let log = Arc::default();
        let model = Model::new(vec![text("completed answer")], Arc::clone(&log));
        let mut policy = Hooks::new(log);
        policy.fail = Some(hook);
        let control = Control::default();
        let error = run_agent_loop(
            config(&model, initial_history()),
            &Host,
            &EchoTool::new(true),
            &mut policy,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap_err();
        let AgentRunError::Failed { cause, partial } = error else {
            panic!("expected started failure")
        };
        assert!(matches!(cause, AgentFailure::Hook { hook: source, .. } if source == hook));
        if hook == "finish_turn" {
            assert_eq!(
                partial.history.context().last().unwrap().text_content(),
                "completed answer"
            );
            assert_eq!(partial.generated_history.len(), 1);
        } else {
            assert!(model.calls.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn changed_model_uses_its_explicit_capabilities_or_unknown() {
    for capabilities in [
        None,
        Some(ModelCapabilities {
            tools: Some(false),
            ..Default::default()
        }),
    ] {
        let log = Arc::default();
        let model = Model::new(vec![text("answer")], Arc::clone(&log));
        let mut policy = Hooks::new(log);
        policy.switch_model = true;
        policy.capabilities = capabilities.clone();
        let mut cfg = config(&model, initial_history());
        cfg.capabilities.tools = Some(false);
        let control = Control::default();
        let result = run_agent_loop(
            cfg,
            &Host,
            &EchoTool::new(true),
            &mut policy,
            &Events::default(),
            handle(&control),
        )
        .await;
        if capabilities.is_none() {
            result.unwrap();
            assert_eq!(model.calls.lock().unwrap()[0].model, "other-model");
        } else {
            // ToolsUnsupported follows the existing no-tools fallback; validation still happens before dispatch.
            result.unwrap();
            let calls = model.calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].tools.is_empty());
        }
    }
}

struct RejectInput;
impl SessionPolicy for RejectInput {
    fn input<'a>(
        &'a mut self,
        _: &'a str,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async { Err("input rejected".into()) })
    }
}
#[tokio::test]
async fn input_hook_failure_returns_every_taken_unconsumed_message_in_order() {
    let log = Arc::default();
    let model = Model::new(vec![], log);
    let control = Control::default();
    let run = handle(&control);
    for id in ["one", "two"] {
        assert_eq!(
            run.steer(SteeringMessage::new(id.into(), id).unwrap()),
            InputAdmission::Accepted
        );
    }
    let error = run_agent_loop(
        config(&model, initial_history()),
        &Host,
        &EchoTool::new(true),
        &mut RejectInput,
        &Events::default(),
        run,
    )
    .await
    .unwrap_err();
    let AgentRunError::Failed { partial, .. } = error else {
        panic!("expected hook failure")
    };
    assert_eq!(
        partial
            .pending_steering
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert_eq!(partial.history.context().len(), 2);
}

struct Blocking(&'static str);
impl SessionPolicy for Blocking {
    fn start<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move {
            if self.0 == "start" {
                std::future::pending().await
            } else {
                Ok(vec![])
            }
        })
    }
    fn input<'a>(
        &'a mut self,
        _: &'a str,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(std::future::pending())
    }
    fn prepare_request<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: &'a mut ModelCall,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if self.0 == "prepare_request" {
                std::future::pending().await
            } else {
                Ok(())
            }
        })
    }
    fn transform_context<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move {
            if self.0 == "transform_context" {
                std::future::pending().await
            } else {
                Ok(history.to_vec())
            }
        })
    }
    fn finish_turn<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async move {
            if self.0 == "finish_turn" {
                std::future::pending().await
            } else {
                Ok(TurnDecision::Default)
            }
        })
    }
}

#[tokio::test]
async fn timeout_cancels_awaiting_hooks_without_dispatching_late_requests_or_losing_completed_answer(
) {
    for hook in [
        "start",
        "prepare_request",
        "transform_context",
        "finish_turn",
    ] {
        let log = Arc::default();
        let model = Model::new(vec![text("completed")], log);
        let control = Control::default();
        let mut cfg = config(&model, initial_history());
        cfg.limits.timeout = Some(std::time::Duration::from_millis(5));
        let error = run_agent_loop(
            cfg,
            &Host,
            &EchoTool::new(true),
            &mut Blocking(hook),
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap_err();
        let AgentRunError::Failed { cause, partial } = error else {
            panic!("expected timeout")
        };
        assert!(matches!(
            cause,
            AgentFailure::LimitExceeded {
                limit: kivio_agent_sdk::agent::error::RunLimit::Timeout
            }
        ));
        let finished = hook == "finish_turn";
        assert_eq!(model.calls.lock().unwrap().len(), usize::from(finished));
        assert_eq!(
            partial.history.context().len(),
            if finished { 3 } else { 2 }
        );
        if finished {
            assert_eq!(partial.content, "completed");
        }
    }
}

#[tokio::test]
async fn cancelled_input_hooks_return_steering_and_follow_up_queues_in_order() {
    for follow_up in [false, true] {
        let log = Arc::default();
        let model = Model::new(vec![text("completed")], log);
        let control = Control::default();
        let run = handle(&control);
        for id in ["one", "two"] {
            let message = SteeringMessage::new(id.into(), id).unwrap();
            assert_eq!(
                if follow_up {
                    run.follow_up(message)
                } else {
                    run.steer(message)
                },
                InputAdmission::Accepted
            );
        }
        let mut cfg = config(&model, initial_history());
        cfg.limits.timeout = Some(std::time::Duration::from_millis(5));
        let error = run_agent_loop(
            cfg,
            &Host,
            &EchoTool::new(true),
            &mut Blocking("input"),
            &Events::default(),
            run,
        )
        .await
        .unwrap_err();
        let AgentRunError::Failed { partial, .. } = error else {
            panic!("expected timeout")
        };
        let queue = if follow_up {
            &partial.pending_follow_up
        } else {
            &partial.pending_steering
        };
        assert_eq!(
            queue
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(
            partial.history.context().len(),
            if follow_up { 3 } else { 2 }
        );
    }
}

#[tokio::test]
async fn hook_model_change_uses_effective_model_when_projecting_opaque_reasoning() {
    for switch_model in [false, true] {
        let log = Arc::default();
        let model = Model::new(vec![text("answer")], Arc::clone(&log));
        let mut history = initial_history();
        history.push(RuntimeMessage::model(kivio_agent_sdk::model::ModelMessage {
            role: ModelRole::Assistant,
            content: vec![
                kivio_agent_sdk::model::MessagePart::ReasoningItem {
                    model: "test-model".into(),
                    item: json!({"type":"reasoning","id":"signed","encrypted_content":"opaque"}),
                },
                kivio_agent_sdk::model::MessagePart::Text { text: "prior answer".into() },
            ],
        }));
        let mut policy = Hooks::new(log);
        policy.switch_model = switch_model;
        let control = Control::default();
        let result = run_agent_loop(
            config(&model, history),
            &Host,
            &EchoTool::new(true),
            &mut policy,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap();
        let calls = model.calls.lock().unwrap();
        let call = &calls[0];
        let request = request_from_history(
            &call.history,
            &call.model,
            call.tools.clone(),
            call.options.clone(),
            call.metadata.clone(),
        );
        let items = kivio_agent_sdk::model::responses_input_from_model_messages(
            &request.messages,
            Some(&request.model),
        );
        assert_eq!(
            items.iter().any(|item| item["id"] == "signed"),
            !switch_model
        );
        assert!(result
            .history
            .context()
            .iter()
            .filter_map(RuntimeMessage::model_message)
            .any(|message| message.content.iter().any(|part| matches!(
                part,
                kivio_agent_sdk::model::MessagePart::ReasoningItem { .. }
            ))));
    }
}
