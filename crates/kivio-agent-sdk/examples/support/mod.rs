pub mod http;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

use kivio_agent_sdk::{
    agent::{
        content::{AgentToolCall, AgentToolStatus},
        control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission},
        events::{AgentRunIdentity, InputKind},
        execute::{PreparedToolCall, ToolFuture, ToolPreparation, ToolProgress},
        AgentHost, AgentHostFuture, AgentRunConfig, RuntimeMessage, SteeringMessage,
        ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{ModelRole, ModelTool, PendingToolCall},
    tools::{schema::validate_arguments, ToolDefinition, ToolOutput},
};
use serde_json::{json, Value};
use tokio::sync::Notify;

#[derive(Default)]
struct RunState {
    stopped: bool,
    steering: Vec<SteeringMessage>,
    follow_up: Vec<SteeringMessage>,
}

/// Do not reuse this mailbox for another run. Drop it after handles and run cleanup finish.
#[derive(Default)]
pub struct Control {
    state: Mutex<RunState>,
    changed: Notify,
}

impl AgentRunControl for Control {
    fn active(&self) -> bool {
        !self.state.lock().unwrap().stopped
    }

    fn stop(&self) -> StopAdmission {
        let mut state = self.state.lock().unwrap();
        if state.stopped {
            return StopAdmission::Inactive;
        }
        state.stopped = true;
        state.steering.clear();
        state.follow_up.clear();
        self.changed.notify_waiters();
        StopAdmission::Requested
    }

    fn submit(&self, kind: InputKind, message: SteeringMessage) -> InputAdmission {
        let mut state = self.state.lock().unwrap();
        if state.stopped {
            return InputAdmission::Inactive;
        }
        match kind {
            InputKind::Steer => state.steering.push(message),
            InputKind::FollowUp => state.follow_up.push(message),
        }
        InputAdmission::Accepted
    }

    fn take(&self, kind: InputKind) -> Vec<SteeringMessage> {
        let mut state = self.state.lock().unwrap();
        if state.stopped {
            return Vec::new();
        }
        std::mem::take(match kind {
            InputKind::Steer => &mut state.steering,
            InputKind::FollowUp => &mut state.follow_up,
        })
    }

    fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
        Box::pin(async {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.active() {
                changed.await;
            }
        })
    }
}

pub struct Host;
impl AgentHost for Host {}

pub fn handle(control: &Control) -> AgentRunHandle<'_> {
    AgentRunHandle::new(
        AgentRunIdentity {
            conversation_id: "example",
            run_id: "run",
            message_id: "reply",
            generation: 1,
        },
        control,
    )
}

pub fn config(
    runtime: &dyn kivio_agent_sdk::agent::provider_runtime::ModelRuntime,
    history: Vec<RuntimeMessage>,
) -> AgentRunConfig<'_> {
    AgentRunConfig {
        provider_runtime: runtime,
        conversation_id: "example".into(),
        run_id: "run".into(),
        message_id: "reply".into(),
        generation: 1,
        provider_id: "explicit-host-provider".into(),
        provider_label: "OpenAI compatible".into(),
        api_format: "chat".into(),
        model: "test-model".into(),
        runtime_messages: history,
        tools: vec![echo_definition()],
        blocked_tool_calls: vec![],
        thinking_enabled: false,
        thinking_level: None,
        builtin_web_search: false,
        max_output_tokens: 128,
        initial_cache_usage: None,
        report_context: false,
    }
}

pub fn initial_history() -> Vec<RuntimeMessage> {
    vec![
        RuntimeMessage::system("Use echo when needed."),
        RuntimeMessage::text(ModelRole::User, "echo hello"),
    ]
}

pub fn echo_definition() -> ToolDefinition {
    ToolDefinition::new(ModelTool {
        name: "echo".into(),
        description: "Return the supplied text.".into(),
        input_schema: json!({"type":"object","properties":{"text":{"type":"string"}},
            "required":["text"],"additionalProperties":false}),
    })
}

/// Explicit host authorization for exactly one registered tool; no desktop approval UI.
pub struct EchoTool {
    pub allowed: bool,
    pub calls: AtomicUsize,
}

impl EchoTool {
    pub fn new(allowed: bool) -> Self {
        Self {
            allowed,
            calls: AtomicUsize::new(0),
        }
    }
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ToolExecutor for EchoTool {
    fn prepare<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        _ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        mut record: AgentToolCall,
    ) -> ToolFuture<'a, ToolPreparation> {
        Box::pin(async move {
            let validated = (|| {
                if call.function_name != "echo" || tool.model.name != "echo" {
                    return Err("Unregistered tool".to_owned());
                }
                if !self.allowed {
                    return Err("Host denied echo".to_owned());
                }
                let arguments = serde_json::from_str::<Value>(&call.arguments_raw)
                    .map_err(|error| format!("Invalid JSON: {error}"))?;
                validate_arguments(&echo_definition().model.input_schema, &arguments)?;
                Ok(arguments)
            })();
            match validated {
                Ok(arguments) => ToolPreparation::Ready(PreparedToolCall {
                    record,
                    arguments,
                    timeout_ms: Some(1000),
                    timeout_message: "echo timed out".into(),
                    output_limit: None,
                    drain_on_cancel: false,
                    feedback: vec![],
                }),
                Err(error) => {
                    record.status = AgentToolStatus::Error;
                    record.error = Some(error.clone());
                    progress.updated(&record);
                    ToolPreparation::Complete(record, error, vec![])
                }
            }
        })
    }

    fn call<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        _tool: &'a ToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::text(
                arguments["text"].as_str().expect("prepare validated text"),
            ))
        })
    }
}
