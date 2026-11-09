use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use kivio_agent_sdk::{
    agent::{
        content::{AgentToolCall, AgentToolStatus},
        context::AgentContext,
        control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission},
        events::{AgentEvent, AgentEventKind, AgentRunIdentity, EventDelivery, InputKind},
        execute::{
            execute_tool_call, PreparedToolCall, ToolFuture, ToolPreparation, ToolProgress,
            ToolResultUpdate,
        },
        AgentHost, AgentHostFuture, SteeringMessage, ToolExecutionContext, ToolExecutor,
        ToolExecutorFuture, ToolInvocation,
    },
    model::{ModelTool, PendingToolCall},
    tools::{ToolDefinition, ToolOutput},
};
use serde_json::{json, Value};
use tokio::sync::Notify;

#[derive(Default)]
struct Host {
    stopped: AtomicBool,
    changed: Notify,
}
impl AgentHost for Host {}
impl AgentRunControl for Host {
    fn active(&self) -> bool {
        !self.stopped.load(Ordering::SeqCst)
    }
    fn stop(&self) -> StopAdmission {
        self.stopped.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
        StopAdmission::Requested
    }
    fn submit(&self, _: InputKind, _: SteeringMessage) -> InputAdmission {
        InputAdmission::Unsupported
    }
    fn take(&self, _: InputKind) -> Vec<SteeringMessage> {
        vec![]
    }
    fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
        Box::pin(async {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active() {
                notified.await;
            }
        })
    }
}

#[derive(Clone, Copy)]
enum Behavior {
    Progress,
    Wait,
    Managed,
    PanicFactory,
    PanicPoll,
}
struct Executor {
    behavior: Behavior,
    timeout: Option<u64>,
    entered: Notify,
    dropped: AtomicUsize,
    sent: AtomicUsize,
    record: Mutex<Option<AgentToolCall>>,
    parent_task: Option<tokio::task::Id>,
}
impl Executor {
    fn new(behavior: Behavior, timeout: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            behavior,
            timeout,
            entered: Notify::new(),
            dropped: AtomicUsize::new(0),
            sent: AtomicUsize::new(0),
            record: Mutex::new(None),
            parent_task: tokio::task::try_id(),
        })
    }
}
struct Dropped(Arc<Executor>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl ToolExecutor for Executor {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress,
        _: &'a ToolExecutionContext,
        _: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            *self.record.lock().unwrap() = Some(record.clone());
            if matches!(self.behavior, Behavior::Progress) {
                for _ in 0..40 {
                    progress.updated(&record).await;
                }
            }
            Ok(ToolPreparation::Ready(PreparedToolCall {
                record,
                arguments: call.arguments,
                timeout_ms: self.timeout,
                timeout_message: "timed out".into(),
                output_limit: None,
                drain_on_cancel: matches!(self.behavior, Behavior::Managed),
                feedback: vec![],
            }))
        })
    }
    fn call(self: Arc<Self>, invocation: ToolInvocation) -> ToolExecutorFuture<'static> {
        assert_ne!(
            tokio::task::try_id(),
            self.parent_task,
            "factory must run in the owned task"
        );
        if matches!(self.behavior, Behavior::PanicFactory) {
            panic!("factory panic");
        }
        Box::pin(async move {
            assert_ne!(
                tokio::task::try_id(),
                self.parent_task,
                "poll must run in the owned task"
            );
            let _dropped = Dropped(self.clone());
            self.entered.notify_one();
            match self.behavior {
                Behavior::Progress => {
                    let record = self.record.lock().unwrap().clone().unwrap();
                    for _ in 0..80 {
                        invocation.progress.updated(&record).await;
                        self.sent.fetch_add(1, Ordering::SeqCst);
                    }
                }
                Behavior::Wait => std::future::pending::<()>().await,
                Behavior::Managed => {
                    invocation.progress.control.wait_inactive().await;
                    let record = self.record.lock().unwrap().clone().unwrap();
                    for _ in 0..80 {
                        invocation.progress.updated(&record).await;
                    }
                    tokio::task::yield_now().await;
                }
                Behavior::PanicPoll => panic!("poll panic"),
                Behavior::PanicFactory => unreachable!(),
            }
            Ok(ToolOutput::text("finished"))
        })
    }
    fn after_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress,
        _: &'a ToolExecutionContext,
        _: &'a ToolDefinition,
        _: Value,
        _: &'a str,
        _: Vec<String>,
    ) -> ToolFuture<'a, Result<ToolResultUpdate, String>> {
        Box::pin(async move {
            if matches!(self.behavior, Behavior::Progress) {
                let record = self.record.lock().unwrap().clone().unwrap();
                for _ in 0..80 {
                    progress.updated(&record).await;
                }
            }
            Ok(ToolResultUpdate::default())
        })
    }
}
fn inputs() -> (ToolExecutionContext, ToolDefinition, PendingToolCall) {
    (
        ToolExecutionContext {
            conversation_id: "c".into(),
            run_id: "r".into(),
            message_id: "m".into(),
            generation: 1,
            round: 1,
            tool_call_id: "call".into(),
        },
        ToolDefinition::new(ModelTool {
            name: "test".into(),
            description: String::new(),
            input_schema: json!({}),
        }),
        PendingToolCall {
            id: "call".into(),
            function_name: "test".into(),
            arguments: json!({}),
            arguments_raw: "{}".into(),
            arguments_parse_error: None,
            signature: None,
        },
    )
}
fn handle(host: Arc<Host>) -> AgentRunHandle {
    AgentRunHandle::new(
        AgentRunIdentity {
            conversation_id: "c",
            run_id: "r",
            message_id: "m",
            generation: 1,
        },
        host,
    )
}

#[tokio::test]
async fn task_runs_without_parent_poll_and_bounded_progress_is_flushed_through_hooks() {
    let host = Arc::new(Host::default());
    let records = Mutex::new(Vec::new());
    let sink = |event: &AgentEvent<'_>| {
        if let AgentEventKind::ToolUpdated(record) = &event.kind {
            records.lock().unwrap().push(record.status.clone());
        }
        EventDelivery::Accepted
    };
    let agent = AgentContext::new(host.as_ref(), &sink, handle(host.clone()));
    let probe = Executor::new(Behavior::Progress, None);
    let executor: Arc<dyn ToolExecutor> = probe.clone();
    let (context, tool, call) = inputs();
    let mut running = Box::pin(execute_tool_call(&agent, &executor, &context, &tool, call));
    // Drive the parent only until the owned task starts; afterwards it is no longer polled.
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            biased;
            _ = probe.entered.notified() => {}
            _ = running.as_mut() => panic!("tool finished before the parent stopped polling"),
        }
    })
    .await
    .unwrap();
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        probe.sent.load(Ordering::SeqCst),
        32,
        "producer must wait at channel capacity while parent is not polled"
    );
    let result = tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .unwrap();
    assert_eq!(result.record.status, AgentToolStatus::Success);
    assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
    let records = records.lock().unwrap();
    assert_eq!(records.len(), 203);
    assert_eq!(records.last(), Some(&AgentToolStatus::Success));
}

#[tokio::test]
async fn cancellation_joins_unmanaged_and_drains_managed_calls() {
    for behavior in [Behavior::Wait, Behavior::Managed] {
        let host = Arc::new(Host::default());
        let count = AtomicUsize::new(0);
        let sink = |event: &AgentEvent<'_>| {
            if matches!(event.kind, AgentEventKind::ToolUpdated(_)) {
                count.fetch_add(1, Ordering::SeqCst);
            }
            EventDelivery::Accepted
        };
        let agent = AgentContext::new(host.as_ref(), &sink, handle(host.clone()));
        let probe = Executor::new(behavior, None);
        let executor: Arc<dyn ToolExecutor> = probe.clone();
        let (context, tool, call) = inputs();
        let mut running = Box::pin(execute_tool_call(&agent, &executor, &context, &tool, call));
        assert!(futures::poll!(running.as_mut()).is_pending());
        probe.entered.notified().await;
        host.stop();
        let result = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap();
        assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(
            result.record.status,
            if matches!(behavior, Behavior::Managed) {
                AgentToolStatus::Success
            } else {
                AgentToolStatus::Cancelled
            }
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            if matches!(behavior, Behavior::Managed) {
                83
            } else {
                3
            }
        );
    }
}

#[tokio::test]
async fn timeout_joins_before_terminal_and_future_drop_aborts_owned_task() {
    for timeout in [Some(1), None] {
        let host = Arc::new(Host::default());
        let sink = |_: &AgentEvent<'_>| EventDelivery::Accepted;
        let agent = AgentContext::new(host.as_ref(), &sink, handle(host.clone()));
        let probe = Executor::new(Behavior::Wait, timeout);
        let executor: Arc<dyn ToolExecutor> = probe.clone();
        let (context, tool, call) = inputs();
        let mut running = Box::pin(execute_tool_call(&agent, &executor, &context, &tool, call));
        assert!(futures::poll!(running.as_mut()).is_pending());
        probe.entered.notified().await;
        if timeout.is_some() {
            let result = running.await;
            assert_eq!(result.record.error.as_deref(), Some("timed out"));
            assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
        } else {
            drop(running);
            tokio::time::timeout(Duration::from_secs(2), async {
                while probe.dropped.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn factory_and_poll_panics_become_typed_host_failures() {
    for behavior in [Behavior::PanicFactory, Behavior::PanicPoll] {
        let host = Arc::new(Host::default());
        let sink = |_: &AgentEvent<'_>| EventDelivery::Accepted;
        let agent = AgentContext::new(host.as_ref(), &sink, handle(host.clone()));
        let executor: Arc<dyn ToolExecutor> = Executor::new(behavior, None);
        let (context, tool, call) = inputs();
        let result = execute_tool_call(&agent, &executor, &context, &tool, call).await;
        assert!(matches!(
            result.failure,
            Some(kivio_agent_sdk::agent::error::AgentFailure::Host {
                operation: "tool.call",
                ..
            })
        ));
    }
}
