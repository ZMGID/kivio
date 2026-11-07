use std::{
    collections::VecDeque,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

use kivio_agent_sdk::{
    agent::{
        content::{AgentToolCall, AgentToolStatus, ContentKind},
        control::{AgentRunHandle, InputAdmission, StopAdmission},
        events::{
            AgentEvent, AgentEventKind, AgentEventSink, AgentRunIdentity, EventDelivery, InputKind,
        },
        execute::{ToolFuture, ToolPreparation, ToolProgress, ToolResultUpdate},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        run_agent_loop,
        session::{NoSessionPolicy, PolicyFuture, RunSnapshot, SessionPolicy, TurnDecision},
        types::{AgentRunResult, AgentRunStatus},
        AgentAdmissionError, AgentFailure, AgentHost, AgentHostFuture, AgentRunError, AgentSession,
        AgentSessionConfig, RuntimeMessage, SessionStatus, SteeringMessage, ToolExecutionContext,
        ToolExecutor, ToolExecutorFuture,
    },
    model::{GenerateOutput, MessagePart, ModelError, ModelRole, PendingToolCall},
    tools::{ToolDefinition, ToolOutput, ToolRegistry},
};
use kivio_agent_sdk_consumer::{
    config, echo_definition, handle, initial_history, Control, EchoTool, Events, Host,
};
use serde_json::{json, Value};
use tokio::sync::Notify;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .expect("session did not settle")
}

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}
impl Gate {
    async fn hold(&self) {
        self.entered.notify_one();
        self.release.notified().await;
    }
}

struct Step {
    output: GenerateOutput,
    gate: Option<Arc<Gate>>,
}
struct Model {
    steps: Mutex<VecDeque<Step>>,
    calls: Mutex<Vec<ModelCall>>,
}
impl Model {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            calls: Mutex::new(vec![]),
        })
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
            self.calls.lock().unwrap().push(call);
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model request");
            if let Some(gate) = step.gate {
                gate.hold().await;
            }
            Ok(step.output)
        })
    }
}
fn text(value: &str) -> Step {
    Step {
        output: GenerateOutput::text(value.into(), None),
        gate: None,
    }
}
fn held(value: &str, gate: &Arc<Gate>) -> Step {
    Step {
        output: text(value).output,
        gate: Some(gate.clone()),
    }
}
fn tool() -> Step {
    let mut step = text("");
    step.output.tool_calls.push(PendingToolCall {
        id: "echo-1".into(),
        function_name: "echo".into(),
        arguments: json!({"text":"executed"}),
        arguments_raw: r#"{"text":"executed"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    step
}
fn messages(history: &[RuntimeMessage]) -> Vec<String> {
    history.iter().map(RuntimeMessage::text_content).collect()
}
fn input(id: &str, value: &str) -> SteeringMessage {
    SteeringMessage::new(id.into(), value).unwrap()
}
fn admission(error: AgentRunError, expected: AgentAdmissionError) {
    assert!(matches!(error, AgentRunError::Admission(actual) if actual == expected));
}

struct PanickingTool {
    delegate: EchoTool,
    synchronous: bool,
    gate: Option<Arc<Gate>>,
    after: Mutex<Vec<String>>,
}
impl ToolExecutor for PanickingTool {
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
            if let ToolPreparation::Ready(ready) = &mut prepared {
                ready.drain_on_cancel = self.gate.is_some();
            }
            Ok(prepared)
        })
    }
    fn call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        args: Value,
    ) -> ToolExecutorFuture<'a> {
        assert!(
            !(ctx.tool_call_id == "echo-2" && self.synchronous),
            "tool construction panic"
        );
        Box::pin(async move {
            assert_ne!(ctx.tool_call_id, "echo-2", "tool polling panic");
            if ctx.tool_call_id == "echo-1" {
                if let Some(gate) = &self.gate {
                    gate.hold().await;
                }
            }
            self.delegate.call(progress, ctx, tool, args).await
        })
    }
    fn after_tool_call<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: Value,
        _: &'a str,
        _: Vec<String>,
    ) -> ToolFuture<'a, Result<ToolResultUpdate, String>> {
        Box::pin(async move {
            self.after.lock().unwrap().push(ctx.tool_call_id.into());
            Ok(ToolResultUpdate::default())
        })
    }
}

#[tokio::test]
async fn direct_executor_panics_are_contained_during_future_construction_and_polling() {
    for synchronous in [true, false] {
        let mut step = tool();
        step.output.tool_calls[0].id = "echo-2".into();
        let model = Model::new(vec![step]);
        let executor = PanickingTool {
            delegate: EchoTool::new(true),
            synchronous,
            gate: None,
            after: Mutex::new(vec![]),
        };
        let control = Control::default();
        let error = bounded(run_agent_loop(
            config(model.as_ref(), initial_history()),
            &Host,
            &executor,
            &mut NoSessionPolicy,
            &Events::default(),
            handle(&control),
        ))
        .await
        .unwrap_err();
        let AgentRunError::Failed { cause, partial } = error else {
            panic!("expected a tool execution failure")
        };
        assert!(matches!(
            cause,
            AgentFailure::Host {
                operation: "tool.call",
                ..
            }
        ));
        assert_eq!(partial.tool_records.len(), 1);
        assert_eq!(partial.tool_records[0].status, AgentToolStatus::Error);
        partial.history.snapshot().unwrap();
        assert!(executor.after.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn tool_execution_panics_preserve_results_and_allow_continuation_without_replay() {
    for synchronous in [true, false] {
        for parallel in [false, true] {
            let count = if parallel { 13 } else { 3 };
            let mut step = tool();
            let template = step.output.tool_calls[0].clone();
            step.output.tool_calls = (1..=count)
                .map(|index| PendingToolCall {
                    id: format!("echo-{index}"),
                    ..template.clone()
                })
                .collect();
            let model = Model::new(vec![step, text("continued")]);
            let executor = Arc::new(PanickingTool {
                delegate: EchoTool::new(true),
                synchronous,
                gate: parallel.then(|| Arc::new(Gate::default())),
                after: Mutex::new(vec![]),
            });
            let mut cfg = AgentSessionConfig::new(model.clone(), "test");
            let mut definition = echo_definition();
            definition.parallel_safe = parallel;
            cfg.tools
                .register_dynamic(definition.clone(), executor.clone())
                .unwrap();
            let session = AgentSession::new(cfg);
            let run = session.prompt("run tools");
            if let Some(gate) = &executor.gate {
                bounded(gate.entered.notified()).await;
                tokio::task::yield_now().await;
                assert_eq!(session.status(), SessionStatus::Running);
                gate.release.notify_one();
            }
            let AgentRunError::Failed { cause, partial } = bounded(run).await.unwrap_err() else {
                panic!("expected a tool execution failure")
            };
            assert_eq!(partial.tool_records.len(), count);
            assert!(
                matches!(cause, AgentFailure::Host { operation: "tool.call", message } if message.contains("panicked"))
            );
            assert_eq!(partial.tool_records[0].status, AgentToolStatus::Success);
            assert_eq!(partial.tool_records[0].output.as_deref(), Some("executed"));
            assert_eq!(partial.tool_records[1].status, AgentToolStatus::Error);
            assert!(partial.tool_records[1]
                .error
                .as_deref()
                .unwrap()
                .contains("panicked"));
            assert_eq!(
                partial.tool_records[count - 1].status,
                AgentToolStatus::Skipped
            );
            let executed = if parallel { 11 } else { 1 };
            assert_eq!(executor.delegate.call_count(), executed);
            let after = executor.after.lock().unwrap().clone();
            assert_eq!(after.len(), executed);
            assert!(!after.iter().any(|id| id == "echo-2"));
            assert_eq!(model.calls.lock().unwrap().len(), 1);
            let context = session.context();
            let paired = context
                .iter()
                .filter_map(RuntimeMessage::model_message)
                .flat_map(|message| &message.content)
                .filter_map(|part| match part {
                    MessagePart::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                paired,
                (1..=count)
                    .map(|index| format!("echo-{index}"))
                    .collect::<Vec<_>>()
            );
            let snapshot =
                serde_json::from_slice(&serde_json::to_vec(&session.snapshot().unwrap()).unwrap())
                    .unwrap();
            assert_eq!(
                bounded(session.continue_run()).await.unwrap().content,
                "continued"
            );
            let restored_model = Model::new(vec![text("restored")]);
            let mut restored_cfg =
                AgentSessionConfig::from_snapshot(restored_model.clone(), snapshot).unwrap();
            restored_cfg
                .tools
                .register_dynamic(definition, executor.clone())
                .unwrap();
            let restored = AgentSession::new(restored_cfg);
            assert_eq!(
                bounded(restored.continue_run()).await.unwrap().content,
                "restored"
            );
            assert_eq!(restored_model.calls.lock().unwrap()[0].history, context);
            assert_eq!(executor.delegate.call_count(), executed);
        }
    }
}

#[tokio::test]
async fn minimal_session_owns_two_turns_without_a_host_or_executor() {
    let model = Model::new(vec![text("first answer"), text("second answer")]);
    let session = AgentSession::new(AgentSessionConfig::new(model.clone(), "test-model"));
    let first = session.prompt("first question");
    assert_eq!(session.status(), SessionStatus::Running);
    assert_eq!(messages(&session.context()), ["first question"]);
    assert_eq!(bounded(first).await.unwrap().content, "first answer");
    let second = bounded(session.prompt("second question")).await.unwrap();
    assert_eq!(second.content, "second answer");
    assert_eq!(session.status(), SessionStatus::Idle);
    assert_eq!(
        messages(&session.context()),
        [
            "first question",
            "first answer",
            "second question",
            "second answer"
        ]
    );
    assert_eq!(second.history.context(), session.context());
    assert_eq!(
        messages(&model.calls.lock().unwrap()[1].history),
        ["first question", "first answer", "second question"]
    );
}

#[tokio::test]
async fn invalid_prompts_leave_an_idle_session_unchanged() {
    let model = Model::new(vec![]);
    let session = AgentSession::new(AgentSessionConfig::new(model.clone(), "test"));
    for prompt in [
        RuntimeMessage::from(" \n "),
        RuntimeMessage::text(ModelRole::Assistant, "not user input"),
        RuntimeMessage::system("not user input"),
    ] {
        admission(
            bounded(session.prompt(prompt)).await.unwrap_err(),
            AgentAdmissionError::EmptyInput,
        );
    }
    assert!(session.context().is_empty());
    assert_eq!(session.status(), SessionStatus::Idle);
    assert_eq!(session.abort(), StopAdmission::Inactive);
    assert_eq!(
        session.steer(input("idle", "ignored")),
        InputAdmission::Inactive
    );
    assert_eq!(
        session.follow_up(input("idle", "ignored")),
        InputAdmission::Inactive
    );
    assert!(model.calls.lock().unwrap().is_empty());
}

#[test]
fn prompt_without_a_runtime_is_rejected_without_mutating_history() {
    let model = Model::new(vec![]);
    let session = AgentSession::new(AgentSessionConfig::new(model.clone(), "test"));
    let run = session.prompt("question");
    assert!(session.context().is_empty());
    assert_eq!(session.status(), SessionStatus::Idle);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    admission(
        runtime.block_on(run).unwrap_err(),
        AgentAdmissionError::NoRuntime,
    );
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn busy_rejection_does_not_append_a_second_prompt() {
    let gate = Arc::new(Gate::default());
    let model = Model::new(vec![held("answer", &gate)]);
    let session = AgentSession::new(AgentSessionConfig::new(model.clone(), "test"));
    let run = session.prompt("accepted");
    bounded(gate.entered.notified()).await;
    admission(
        bounded(session.prompt("rejected")).await.unwrap_err(),
        AgentAdmissionError::Busy,
    );
    admission(
        bounded(session.continue_run()).await.unwrap_err(),
        AgentAdmissionError::Busy,
    );
    assert_eq!(messages(&session.context()), ["accepted"]);
    gate.release.notify_one();
    bounded(run).await.unwrap();
    assert_eq!(messages(&session.context()), ["accepted", "answer"]);
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn replacing_tools_while_idle_changes_the_next_run() {
    let model = Model::new(vec![tool(), text("first"), tool(), text("second")]);
    let first_executor = Arc::new(EchoTool::new(true));
    let second_executor = Arc::new(EchoTool::new(true));
    let mut cfg = AgentSessionConfig::new(model.clone(), "test");
    cfg.tools
        .register_dynamic(echo_definition(), first_executor.clone())
        .unwrap();
    let session = AgentSession::new(cfg);
    bounded(session.prompt("first question")).await.unwrap();

    let mut replacement = ToolRegistry::new();
    replacement
        .register_dynamic(echo_definition(), second_executor.clone())
        .unwrap();
    let mut extra = echo_definition();
    extra.key = "extra".into();
    extra.model.name = "extra".into();
    extra.label.name = "extra".into();
    replacement
        .register_dynamic(extra, second_executor.clone())
        .unwrap();
    session.set_tools(replacement).unwrap();
    bounded(session.prompt("second question")).await.unwrap();

    assert_eq!(first_executor.call_count(), 1);
    assert_eq!(second_executor.call_count(), 1);
    let calls = model.calls.lock().unwrap();
    assert_eq!(
        calls[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["echo"]
    );
    assert_eq!(
        calls[2]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["echo", "extra"]
    );
}

struct CommitHost {
    gate: Arc<Gate>,
    committed: AtomicBool,
    fail: bool,
}
impl AgentHost for CommitHost {
    fn commit_run<'a>(
        &'a self,
        _: AgentRunIdentity<'a>,
        _: &'a AgentRunResult,
    ) -> AgentHostFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.gate.hold().await;
            self.committed.store(true, Ordering::SeqCst);
            if self.fail {
                Err("disk full".into())
            } else {
                Ok(())
            }
        })
    }
}

struct ManagedTool {
    delegate: EchoTool,
    gate: Arc<Gate>,
    cleaned: AtomicBool,
    observed_stop: AtomicBool,
}
impl ToolExecutor for ManagedTool {
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
            if let ToolPreparation::Ready(ready) = &mut prepared {
                ready.drain_on_cancel = true;
                ready.timeout_ms = None;
            }
            Ok(prepared)
        })
    }
    fn call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            self.gate.hold().await;
            self.observed_stop
                .store(!progress.control.active(), Ordering::SeqCst);
            self.cleaned.store(true, Ordering::SeqCst);
            Ok(ToolOutput::text("cleanup complete"))
        })
    }
}

#[tokio::test]
async fn abort_and_dropped_run_keep_admission_busy_until_tool_drain_and_commit_finish() {
    for drop_run in [false, true] {
        let tool_gate = Arc::new(Gate::default());
        let commit_gate = Arc::new(Gate::default());
        let executor = Arc::new(ManagedTool {
            delegate: EchoTool::new(true),
            gate: tool_gate.clone(),
            cleaned: AtomicBool::new(false),
            observed_stop: AtomicBool::new(false),
        });
        let host = Arc::new(CommitHost {
            gate: commit_gate.clone(),
            committed: AtomicBool::new(false),
            fail: false,
        });
        let model = Model::new(vec![tool(), tool(), text("fresh answer")]);
        let mut cfg = AgentSessionConfig::new(model.clone(), "test");
        cfg.tools
            .register_dynamic(echo_definition(), executor.clone())
            .unwrap();
        cfg.host = host.clone();
        let session = AgentSession::new(cfg);
        let replacement_executor = Arc::new(EchoTool::new(true));
        let mut replacement = ToolRegistry::new();
        replacement
            .register_dynamic(echo_definition(), replacement_executor.clone())
            .unwrap();
        let run = session.prompt("start");
        bounded(tool_gate.entered.notified()).await;
        assert_eq!(session.status(), SessionStatus::Running);
        assert_eq!(
            session.set_tools(replacement.snapshot()),
            Err(AgentAdmissionError::Busy)
        );
        let mut run = Some(run);
        if drop_run {
            drop(run.take());
        } else {
            assert_eq!(session.abort(), StopAdmission::Requested);
        }
        assert_eq!(session.status(), SessionStatus::StopRequested);
        assert_eq!(
            session.set_tools(replacement.snapshot()),
            Err(AgentAdmissionError::Busy)
        );
        let mut inactive = Box::pin(session.wait_inactive());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut inactive)
                .await
                .is_err()
        );
        assert!(!executor.cleaned.load(Ordering::SeqCst));
        admission(
            bounded(session.prompt("too early")).await.unwrap_err(),
            AgentAdmissionError::Busy,
        );
        tool_gate.release.notify_one();
        bounded(commit_gate.entered.notified()).await;
        assert!(executor.cleaned.load(Ordering::SeqCst));
        assert!(executor.observed_stop.load(Ordering::SeqCst));
        assert!(!host.committed.load(Ordering::SeqCst));
        assert_eq!(session.status(), SessionStatus::Finishing);
        assert_eq!(
            session.set_tools(replacement.snapshot()),
            Err(AgentAdmissionError::Busy)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut inactive)
                .await
                .is_err()
        );
        admission(
            bounded(session.prompt("still early")).await.unwrap_err(),
            AgentAdmissionError::Busy,
        );
        commit_gate.release.notify_one();
        bounded(inactive).await;
        assert!(host.committed.load(Ordering::SeqCst));
        assert_eq!(session.status(), SessionStatus::Idle);
        if let Some(run) = run {
            assert_eq!(
                bounded(run).await.unwrap().status,
                AgentRunStatus::Cancelled
            );
        }
        assert!(session
            .context()
            .iter()
            .any(|message| message.role() == Some(ModelRole::Tool)));
        let fresh = session.prompt("fresh");
        bounded(tool_gate.entered.notified()).await;
        tool_gate.release.notify_one();
        bounded(commit_gate.entered.notified()).await;
        commit_gate.release.notify_one();
        let fresh = bounded(fresh).await.unwrap();
        assert_eq!(fresh.content, "fresh answer");
        assert_eq!(
            fresh.tool_records[0].output.as_deref(),
            Some("cleanup complete")
        );
        assert_eq!(replacement_executor.call_count(), 0);
        assert_eq!(model.calls.lock().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn inactive_wait_is_bound_to_the_generation_captured_before_polling() {
    let first_gate = Arc::new(Gate::default());
    let second_gate = Arc::new(Gate::default());
    let model = Model::new(vec![held("one", &first_gate), held("two", &second_gate)]);
    let session = AgentSession::new(AgentSessionConfig::new(model, "test"));
    let first = session.prompt("first");
    bounded(first_gate.entered.notified()).await;
    let first_inactive = session.wait_inactive();
    first_gate.release.notify_one();
    bounded(first).await.unwrap();
    let second = session.prompt("second");
    bounded(second_gate.entered.notified()).await;
    bounded(first_inactive).await;
    assert_eq!(session.status(), SessionStatus::Running);
    second_gate.release.notify_one();
    bounded(second).await.unwrap();
}

struct RejectInput;
impl SessionPolicy for RejectInput {
    fn input<'a>(
        &'a mut self,
        _: &'a str,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async { Err("input hook failed".into()) })
    }
}

#[tokio::test]
async fn input_hook_failure_retains_each_queue_without_replaying_it_in_a_fresh_run() {
    for kind in [InputKind::Steer, InputKind::FollowUp] {
        let gate = Arc::new(Gate::default());
        let model = Model::new(vec![held("first answer", &gate), text("fresh answer")]);
        let cfg = AgentSessionConfig::new(model.clone(), "test").with_policy(RejectInput);
        let session = AgentSession::new(cfg);
        let run = session.prompt("original");
        bounded(gate.entered.notified()).await;
        for id in ["one", "two"] {
            let message = input(id, &format!("queued {id}"));
            let admitted = match kind {
                InputKind::Steer => session.steer(message),
                InputKind::FollowUp => session.follow_up(message),
            };
            assert_eq!(admitted, InputAdmission::Accepted);
        }
        gate.release.notify_one();
        let AgentRunError::Failed { cause, partial } = bounded(run).await.unwrap_err() else {
            panic!("expected hook failure")
        };
        assert!(matches!(cause, AgentFailure::Hook { hook: "input", .. }));
        assert_eq!(partial.history.context(), session.context());
        assert!(!messages(&session.context())
            .iter()
            .any(|value| value.starts_with("queued")));
        let pending = session.unconsumed_inputs();
        assert_eq!(
            pending
                .iter()
                .map(|item| item.message.id.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert!(pending
            .iter()
            .all(|item| item.kind == kind && item.generation == 1));
        bounded(session.prompt("fresh")).await.unwrap();
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        assert!(!messages(&model.calls.lock().unwrap()[1].history)
            .iter()
            .any(|value| value.starts_with("queued")));
        assert_eq!(session.take_unconsumed_inputs().len(), 2);
        assert!(session.unconsumed_inputs().is_empty());
    }
}

#[tokio::test]
async fn failed_commit_reports_failure_and_keeps_the_complete_partial_history() {
    let gate = Arc::new(Gate::default());
    let host = Arc::new(CommitHost {
        gate: gate.clone(),
        committed: AtomicBool::new(false),
        fail: true,
    });
    let mut cfg = AgentSessionConfig::new(Model::new(vec![text("retained answer")]), "test");
    cfg.host = host;
    let session = AgentSession::new(cfg);
    let run = session.prompt("question");
    bounded(gate.entered.notified()).await;
    assert_eq!(
        messages(&session.context()),
        ["question", "retained answer"]
    );
    gate.release.notify_one();
    let AgentRunError::Failed { cause, partial } = bounded(run).await.unwrap_err() else {
        panic!("expected persistence failure")
    };
    assert!(matches!(
        cause,
        AgentFailure::Host {
            operation: "session.commit_run",
            ..
        }
    ));
    assert_eq!(partial.status, AgentRunStatus::Failed(cause));
    assert_eq!(partial.content, "retained answer");
    assert_eq!(partial.history.context(), session.context());
    assert_eq!(session.status(), SessionStatus::Idle);
}

struct PanicCommit;
impl AgentHost for PanicCommit {
    fn commit_run<'a>(
        &'a self,
        _: AgentRunIdentity<'a>,
        _: &'a AgentRunResult,
    ) -> AgentHostFuture<'a, Result<(), String>> {
        panic!("commit construction panicked")
    }
}

#[tokio::test]
async fn synchronous_commit_panic_settles_the_session_and_preserves_history() {
    let mut cfg = AgentSessionConfig::new(Model::new(vec![text("answer")]), "test");
    cfg.host = Arc::new(PanicCommit);
    let session = AgentSession::new(cfg);
    let AgentRunError::Failed { partial, .. } =
        bounded(session.prompt("question")).await.unwrap_err()
    else {
        panic!("expected commit failure")
    };
    assert_eq!(messages(&partial.history.context()), ["question", "answer"]);
    assert_eq!(session.status(), SessionStatus::Idle);
    bounded(session.wait_inactive()).await;
}

struct PanicAfterTools;
impl SessionPolicy for PanicAfterTools {
    fn finish_turn<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async { panic!("finish hook panicked after side effects") })
    }
}

#[tokio::test]
async fn panic_after_tool_execution_keeps_completed_tool_facts() {
    let executor = Arc::new(EchoTool::new(true));
    let mut cfg =
        AgentSessionConfig::new(Model::new(vec![tool()]), "test").with_policy(PanicAfterTools);
    cfg.tools
        .register_dynamic(echo_definition(), executor.clone())
        .unwrap();
    let session = AgentSession::new(cfg);
    let AgentRunError::Failed { partial, .. } =
        bounded(session.prompt("question")).await.unwrap_err()
    else {
        panic!("expected hook failure")
    };
    assert_eq!(executor.call_count(), 1);
    assert_eq!(partial.tool_records.len(), 1);
    assert_eq!(partial.history.context().len(), 3);
    assert_eq!(partial.history.context(), session.context());
    assert_eq!(session.status(), SessionStatus::Idle);
}

struct ReentrantSink {
    session: Weak<AgentSession>,
    calls: AtomicUsize,
    closed: bool,
}
impl AgentEventSink for ReentrantSink {
    fn on_event(&self, _: &AgentEvent<'_>) -> EventDelivery {
        self.session.upgrade().unwrap().context();
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.closed {
            EventDelivery::Closed
        } else {
            EventDelivery::Accepted
        }
    }
}

#[tokio::test]
async fn subscriptions_support_reentrant_reads_closed_delivery_and_drop() {
    let session = Arc::new(AgentSession::new(AgentSessionConfig::new(
        Model::new(vec![text("one"), text("two")]),
        "test",
    )));
    let open = Arc::new(ReentrantSink {
        session: Arc::downgrade(&session),
        calls: AtomicUsize::new(0),
        closed: false,
    });
    let closed = Arc::new(ReentrantSink {
        session: Arc::downgrade(&session),
        calls: AtomicUsize::new(0),
        closed: true,
    });
    let subscription = session.subscribe(open.clone());
    let _closed_subscription = session.subscribe(closed.clone());
    bounded(session.prompt("first")).await.unwrap();
    assert_eq!(closed.calls.load(Ordering::SeqCst), 1);
    let open_calls = open.calls.load(Ordering::SeqCst);
    assert!(open_calls > 1);
    drop(subscription);
    bounded(session.prompt("second")).await.unwrap();
    assert_eq!(closed.calls.load(Ordering::SeqCst), 1);
    assert_eq!(open.calls.load(Ordering::SeqCst), open_calls);
}

fn legacy(value: Value) -> RuntimeMessage {
    RuntimeMessage::from_legacy(value).unwrap()
}

#[tokio::test]
async fn continuation_rejects_invalid_histories_and_resumes_a_finished_tool_round_once() {
    let user = RuntimeMessage::text(ModelRole::User, "question");
    let assistant = legacy(
        json!({"role":"assistant","content":"","tool_calls":[{"id":"echo-1","type":"function","function":{"name":"echo","arguments":"{\"text\":\"executed\"}"}}]}),
    );
    let result =
        legacy(json!({"role":"tool","tool_call_id":"echo-1","content":"already executed"}));
    for history in [
        vec![],
        vec![
            user.clone(),
            RuntimeMessage::text(ModelRole::Assistant, "done"),
        ],
        vec![user.clone(), assistant.clone()],
    ] {
        let model = Model::new(vec![]);
        let mut cfg = AgentSessionConfig::new(model.clone(), "test");
        cfg.history = history.clone().into();
        let session = AgentSession::new(cfg);
        admission(
            bounded(session.continue_run()).await.unwrap_err(),
            AgentAdmissionError::InvalidContinuation,
        );
        assert_eq!(session.context(), history);
        assert!(model.calls.lock().unwrap().is_empty());
    }
    let model = Model::new(vec![text("finished")]);
    let executor = Arc::new(EchoTool::new(true));
    let mut cfg = AgentSessionConfig::new(model.clone(), "test");
    cfg.history = vec![user, assistant, result].into();
    cfg.tools
        .register_dynamic(echo_definition(), executor.clone())
        .unwrap();
    let session = AgentSession::new(cfg);
    let outcome = bounded(session.continue_run()).await.unwrap();
    assert_eq!(outcome.content, "finished");
    assert_eq!(executor.call_count(), 0);
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    assert_eq!(
        messages(&session.context()),
        ["question", "", "already executed", "finished"]
    );
}

#[tokio::test]
async fn duplicate_input_ids_are_consumed_once_across_both_input_kinds() {
    let gate = Arc::new(Gate::default());
    let model = Model::new(vec![held("intermediate", &gate), text("final")]);
    let session = AgentSession::new(AgentSessionConfig::new(model.clone(), "test"));
    let consumed = Arc::new(Mutex::new(vec![]));
    let observed = consumed.clone();
    let _subscription = session.subscribe(Arc::new(move |event: &AgentEvent<'_>| {
        if let AgentEventKind::Content {
            block: Some(block), ..
        } = &event.kind
        {
            if let ContentKind::Input { message, .. } = &block.kind {
                observed.lock().unwrap().push(message.id.clone());
            }
        }
        EventDelivery::Accepted
    }));
    let run = session.prompt("question");
    bounded(gate.entered.notified()).await;
    assert_eq!(
        session.steer(input("same", "once")),
        InputAdmission::Accepted
    );
    assert_eq!(
        session.steer(input("same", "twice")),
        InputAdmission::Duplicate
    );
    assert_eq!(
        session.follow_up(input("same", "third")),
        InputAdmission::Duplicate
    );
    gate.release.notify_one();
    let outcome = bounded(run).await.unwrap();
    assert_eq!(*consumed.lock().unwrap(), ["same"]);
    assert_eq!(
        messages(&outcome.history.context())
            .iter()
            .filter(|value| *value == "once")
            .count(),
        1
    );
    assert!(!messages(&outcome.history.context())
        .iter()
        .any(|value| value == "twice" || value == "third"));
    assert!(session.unconsumed_inputs().is_empty());
    assert_eq!(model.calls.lock().unwrap().len(), 2);
}
