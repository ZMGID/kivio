use kivio_agent_sdk::{
    agent::{
        content::AgentToolCall,
        control::AgentRunControl,
        error::RunLimit,
        execute::{ToolFuture, ToolPreparation, ToolProgress},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        recovery::{RecoveryDecision, RecoveryInput, RecoverySnapshot},
        run_agent_loop,
        session::{NoSessionPolicy, PolicyFuture, RunSnapshot, SessionPolicy, TurnDecision},
        types::RunLimits,
        AgentFailure, AgentRunError, ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{GenerateOutput, ModelError, PendingToolCall},
    tools::ToolDefinition,
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Events, Host};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

struct Model {
    outputs: Mutex<VecDeque<Result<GenerateOutput, ModelError>>>,
    calls: AtomicUsize,
}
impl Model {
    fn new(outputs: Vec<Result<GenerateOutput, ModelError>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            calls: AtomicUsize::new(0),
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            sink.request_started(&call);
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model call")
        })
    }
}
fn tool() -> GenerateOutput {
    let mut output = GenerateOutput::text(String::new(), None);
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
fn limits(steps: u32) -> RunLimits {
    RunLimits {
        max_model_steps: NonZeroU32::new(steps),
        timeout: None,
    }
}
fn assert_limit(
    error: AgentRunError,
    limit: RunLimit,
) -> Box<kivio_agent_sdk::agent::types::AgentRunResult> {
    let AgentRunError::Failed { cause, partial } = error else {
        panic!("expected started failure")
    };
    assert_eq!(cause, AgentFailure::LimitExceeded { limit });
    assert_eq!(
        partial.status,
        kivio_agent_sdk::agent::types::AgentRunStatus::Failed(cause)
    );
    partial
}

#[tokio::test]
async fn model_step_limit_keeps_completed_tools_without_a_second_model_call() {
    let model = Model::new(vec![Ok(tool())]);
    let mut cfg = config(&model, initial_history());
    cfg.limits = limits(1);
    let control = Control::default();
    let executor = EchoTool::new(true);
    let error = run_agent_loop(
        cfg,
        &Host,
        &executor,
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap_err();
    let partial = assert_limit(error, RunLimit::ModelSteps);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(executor.call_count(), 1);
    assert_eq!(partial.tool_records.len(), 1);
    assert_eq!(partial.history.context().len(), 4);
}

#[derive(Default)]
struct Recover {
    finishes: usize,
}
impl SessionPolicy for Recover {
    fn finish_turn<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: kivio_agent_sdk::agent::control::AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async move {
            self.finishes += 1;
            Ok(TurnDecision::Default)
        })
    }
    fn recover(&mut self, run: RecoverySnapshot<'_>, _: &ModelError) -> RecoveryDecision {
        RecoveryDecision::Retry(RecoveryInput::Reduced(run.history.to_vec()))
    }
}
#[tokio::test]
async fn collected_recovery_counts_against_the_same_step_limit() {
    for max in [2, 3] {
        let model = Model::new(vec![
            Ok(tool()),
            Err(ModelError::new("provider failed")),
            Ok(GenerateOutput::text("recovered".into(), None)),
        ]);
        let mut cfg = config(&model, initial_history());
        cfg.limits = limits(max);
        let control = Control::default();
        let mut recovery = Recover::default();
        let result = run_agent_loop(
            cfg,
            &Host,
            &EchoTool::new(true),
            &mut recovery,
            &Events::default(),
            handle(&control),
        )
        .await;
        if max == 2 {
            assert_limit(result.unwrap_err(), RunLimit::ModelSteps);
        } else {
            assert_eq!(result.unwrap().content, "recovered");
        }
        assert_eq!(model.calls.load(Ordering::SeqCst), max as usize);
        assert_eq!(recovery.finishes, if max == 2 { 1 } else { 2 });
    }
}

struct Managed<'a> {
    control: &'a Control,
    delegate: EchoTool,
    cleaned: AtomicBool,
}
impl ToolExecutor for Managed<'_> {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            let mut prepared = self
                .delegate
                .before_tool_call(progress, ctx, tool, call, record)
                .await?;
            if let ToolPreparation::Ready(call) = &mut prepared {
                call.drain_on_cancel = true;
            }
            Ok(prepared)
        })
    }
    fn call<'a>(
        &'a self,
        _progress: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            self.control.wait_inactive().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
            self.cleaned.store(true, Ordering::SeqCst);
            Err("cancelled after cleanup".into())
        })
    }
}
#[tokio::test]
async fn timeout_requests_stop_and_waits_for_managed_tool_cleanup() {
    let model = Model::new(vec![Ok(tool())]);
    let mut cfg = config(&model, initial_history());
    cfg.limits.timeout = Some(Duration::from_millis(20));
    let control = Control::default();
    let executor = Managed {
        control: &control,
        delegate: EchoTool::new(true),
        cleaned: AtomicBool::new(false),
    };
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        run_agent_loop(
            cfg,
            &Host,
            &executor,
            &mut NoSessionPolicy,
            &Events::default(),
            handle(&control),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    let partial = assert_limit(error, RunLimit::Timeout);
    assert!(executor.cleaned.load(Ordering::SeqCst));
    assert!(!control.active());
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(partial.tool_records.len(), 1);
    assert_eq!(partial.history.context().len(), 4);
}

struct ReadyModel(AtomicUsize);
impl ModelRuntime for ReadyModel {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            sink.request_started(&call);
            Ok(GenerateOutput::text("answer".into(), None))
        })
    }
}
struct ContinueForever;
impl SessionPolicy for ContinueForever {
    fn finish_turn<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: kivio_agent_sdk::agent::control::AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async { Ok(TurnDecision::Continue(vec![])) })
    }
}
#[tokio::test]
async fn timeout_bounds_always_ready_model_loops_and_zero_prevents_requests() {
    for timeout in [Duration::ZERO, Duration::from_millis(5)] {
        let model = ReadyModel(AtomicUsize::new(0));
        let mut cfg = config(&model, initial_history());
        cfg.tools.clear();
        cfg.limits.timeout = Some(timeout);
        let control = Control::default();
        let error = run_agent_loop(
            cfg,
            &Host,
            &EchoTool::new(true),
            &mut ContinueForever,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap_err();
        let partial = assert_limit(error, RunLimit::Timeout);
        if timeout.is_zero() {
            assert_eq!(model.0.load(Ordering::SeqCst), 0);
        } else {
            assert!(model.0.load(Ordering::SeqCst) > 0);
            assert!(!partial.generated_history.is_empty());
        }
    }
}
