use kivio_agent_sdk::{
    agent::{
        events::{AgentEvent, AgentEventKind, AgentEventSink, AgentRunOutcome, EventDelivery},
        host::SessionInput,
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        run_agent_loop,
        session::NoSessionPolicy,
        types::AgentRunStatus,
        AgentAdmissionError, AgentFailure, AgentHost, AgentHostFuture, AgentRunError,
    },
    model::{GenerateOutput, ModelError, ModelRole, ModelUsage, PendingToolCall},
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Host};
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};

struct Model(Mutex<VecDeque<Result<GenerateOutput, ModelError>>>);
impl ModelRuntime for Model {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            sink.request_started(&call);
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request")
        })
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<AgentRunOutcome>>);
impl AgentEventSink for Events {
    fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery {
        if let AgentEventKind::RunEnded { outcome } = &event.kind {
            self.0.lock().unwrap().push(outcome.clone());
        }
        EventDelivery::Accepted
    }
}

fn tool_answer() -> GenerateOutput {
    let mut output = GenerateOutput::text(String::new(), None);
    output.tool_calls.push(PendingToolCall {
        id: "echo-1".into(),
        function_name: "echo".into(),
        arguments: json!({"text":"already executed"}),
        arguments_raw: r#"{"text":"already executed"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    output.usage = Some(ModelUsage {
        input_tokens: Some(12),
        output_tokens: Some(3),
        ..Default::default()
    });
    output
}

#[tokio::test]
async fn model_failure_retains_completed_work_usage_and_provider_identity() {
    let failure = ModelError::from_provider_http(
        429,
        r#"{"error":{"code":"rate_limit"}}"#,
        "slow down".into(),
    );
    let model = Model(Mutex::new(
        vec![Ok(tool_answer()), Err(failure.clone())].into(),
    ));
    let control = Control::default();
    let executor = EchoTool::new(true);
    let events = Events::default();
    let error = run_agent_loop(
        config(&model, initial_history()),
        &Host,
        &executor,
        &mut NoSessionPolicy,
        &events,
        handle(&control),
    )
    .await
    .unwrap_err();
    let AgentRunError::Failed { cause, partial } = error else {
        panic!("started failure")
    };
    assert!(matches!(&cause, AgentFailure::Model { source, .. } if source == &failure));
    assert_eq!(partial.status, AgentRunStatus::Failed(cause.clone()));
    assert_eq!(executor.call_count(), 1);
    assert_eq!(partial.tool_records.len(), 1);
    assert_eq!(partial.usage.as_ref().unwrap().input_tokens, Some(12));
    assert_eq!(partial.history.context().len(), 4);
    assert_eq!(partial.generated_history.len(), 2);
    assert_eq!(
        partial
            .history
            .context()
            .last()
            .unwrap()
            .model_message()
            .unwrap()
            .role,
        ModelRole::Tool
    );
    assert_eq!(
        *events.0.lock().unwrap(),
        vec![AgentRunOutcome::Failed(cause)]
    );
}

struct SaveFailure;
impl AgentHost for SaveFailure {
    fn checkpoint_runtime<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a kivio_agent_sdk::agent::SessionHistory,
        _: bool,
    ) -> AgentHostFuture<'a, Result<Vec<SessionInput>, String>> {
        Box::pin(async { Err("disk full".into()) })
    }
}

struct BoundaryFailure(bool);
impl AgentHost for BoundaryFailure {
    fn checkpoint_runtime<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a kivio_agent_sdk::agent::SessionHistory,
        finishing: bool,
    ) -> AgentHostFuture<'a, Result<Vec<SessionInput>, String>> {
        Box::pin(async move {
            if self.0 && finishing {
                Err("final save failed".into())
            } else {
                Ok(vec![])
            }
        })
    }
    fn wait_for_child_results<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: u64,
    ) -> AgentHostFuture<'a, Result<bool, String>> {
        Box::pin(async { Err("child wait failed".into()) })
    }
}

#[tokio::test]
async fn boundary_failure_retains_the_answer_that_already_completed() {
    for fail_save in [true, false] {
        let model = Model(Mutex::new(
            vec![Ok(GenerateOutput::text("completed answer".into(), None))].into(),
        ));
        let control = Control::default();
        let error = run_agent_loop(
            config(&model, initial_history()),
            &BoundaryFailure(fail_save),
            &EchoTool::new(true),
            &mut NoSessionPolicy,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap_err();
        let AgentRunError::Failed { partial, .. } = error else {
            panic!("started failure")
        };
        assert_eq!(
            partial.history.context().last().unwrap().text_content(),
            "completed answer"
        );
        assert_eq!(
            partial.generated_history.last().unwrap().text_content(),
            "completed answer"
        );
    }
}

struct DraftFailure(Mutex<bool>);
impl ModelRuntime for DraftFailure {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            sink.request_started(&call);
            if !std::mem::replace(&mut *self.0.lock().unwrap(), true) {
                return Ok(tool_answer());
            }
            sink.emit(kivio_agent_sdk::model::StreamPart::ToolCallStart {
                id: "incomplete".into(),
                name: "echo".into(),
            })?;
            sink.emit(kivio_agent_sdk::model::StreamPart::ToolCallDelta {
                id: "incomplete".into(),
                delta: r#"{"text":""#.into(),
            })?;
            Err(ModelError::new("connection lost while drafting"))
        })
    }
}

#[tokio::test]
async fn draft_failure_keeps_previously_completed_tool_records() {
    let model = DraftFailure(Mutex::new(false));
    let control = Control::default();
    let executor = EchoTool::new(true);
    let result = run_agent_loop(
        config(&model, initial_history()),
        &Host,
        &executor,
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert!(matches!(result.status, AgentRunStatus::Failed(_)));
    assert_eq!(executor.call_count(), 1);
    assert_eq!(result.tool_records.len(), 2);
    assert_eq!(result.tool_records[0].id, "echo-1");
    assert_eq!(result.tool_records[1].id, "incomplete");
}

#[tokio::test]
async fn host_failure_preserves_input_and_does_not_become_a_provider_error() {
    let model = Model(Mutex::new(VecDeque::new()));
    let control = Control::default();
    let events = Events::default();
    let error = run_agent_loop(
        config(&model, initial_history()),
        &SaveFailure,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &events,
        handle(&control),
    )
    .await
    .unwrap_err();
    let AgentRunError::Failed { cause, partial } = error else {
        panic!("started failure")
    };
    assert_eq!(
        cause,
        AgentFailure::Host {
            operation: "checkpoint_runtime",
            message: "disk full".into()
        }
    );
    assert_eq!(partial.history.context(), initial_history());
    assert!(partial.generated_history.is_empty());
    assert_eq!(
        *events.0.lock().unwrap(),
        vec![AgentRunOutcome::Failed(cause)]
    );
}

#[tokio::test]
async fn identity_rejection_does_not_start_or_end_a_run() {
    let model = Model(Mutex::new(VecDeque::new()));
    let control = Control::default();
    let events = Events::default();
    let mut cfg = config(&model, initial_history());
    cfg.generation += 1;
    let error = run_agent_loop(
        cfg,
        &Host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &events,
        handle(&control),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        AgentRunError::Admission(AgentAdmissionError::IdentityMismatch)
    ));
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn http_retry_after_survives_the_run_error_boundary() {
    use kivio_agent_sdk::model::openai::{OpenAiChatConfig, OpenAiChatProvider};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (header, seconds) in [
        ("3", Some(3)),
        ("Wed, 21 Oct 2015 07:28:00 GMT", Some(0)),
        ("invalid", None),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/chat", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            socket.read(&mut buffer).await.unwrap();
            let body = r#"{"error":{"code":"rate_limit","message":"slow down"}}"#;
            socket.write_all(format!("HTTP/1.1 429 Too Many Requests\r\nContent-Length: {}\r\nRetry-After: {header}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let provider = OpenAiChatProvider::new(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
            OpenAiChatConfig::new(endpoint),
        );
        let control = Control::default();
        let error = run_agent_loop(
            config(&provider, initial_history()),
            &Host,
            &EchoTool::new(true),
            &mut NoSessionPolicy,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap_err();
        let AgentRunError::Failed {
            cause: AgentFailure::Model { source, .. },
            ..
        } = error
        else {
            panic!("provider failure")
        };
        assert_eq!(source.status, Some(429));
        assert_eq!(source.provider_code.as_deref(), Some("rate_limit"));
        assert_eq!(
            source.retry_after.map(|duration| duration.as_secs()),
            seconds
        );
        server.await.unwrap();
    }
}
