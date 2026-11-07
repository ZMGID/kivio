//! Run with `cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example session_offline`.
//! No credentials or network. Assertions make this example an executable contract check.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use kivio_agent_sdk::{
    agent::{
        control::AgentRunHandle,
        events::AgentRunIdentity,
        execute::ToolProgress,
        history::request_from_history,
        host::{AgentHost, AgentHostFuture},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        session::{PolicyFuture, RunSnapshot, SessionPolicy},
        types::AgentRunResult,
        AgentRunError, AgentSession, AgentSessionConfig, RuntimeMessage, SessionSnapshot,
        ToolExecutionContext,
    },
    model::{GenerateOutput, ModelArtifact, ModelError, ModelTool, PendingToolCall},
    tools::{StaticTool, ToolAuthorization, ToolDefinition, ToolError, ToolFuture, ToolOutput},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

const REQUEST_NOTE: &str = "This request runs offline.";

struct OfflineModel(Mutex<VecDeque<GenerateOutput>>);
impl ModelRuntime for OfflineModel {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            // Check the effective provider request, not just whether the hook ran.
            let request = request_from_history(
                &call.history,
                &call.model,
                call.tools.clone(),
                call.options.clone(),
                call.metadata.clone(),
            );
            assert!(request.system.contains(REQUEST_NOTE));
            assert_eq!(
                request.tools[0].input_schema["properties"]["text"]["type"],
                "string"
            );
            sink.request_started(&call);
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| "Unexpected model request".into())
        })
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EchoArgs {
    text: String,
}
struct Echo(Arc<AtomicUsize>);
impl StaticTool for Echo {
    type Args = EchoArgs;
    type Output = String;
    type Error = &'static str;
    type HostData = Vec<ModelArtifact>;

    fn execute<'a>(
        &'a self,
        args: EchoArgs,
    ) -> ToolFuture<'a, Result<ToolOutput<Self::HostData, String>, Self::Error>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::text(args.text))
        })
    }
}

struct AllowEcho;
impl ToolAuthorization for AllowEcho {
    fn authorize<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        _: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            if tool.key == "echo" {
                Ok(())
            } else {
                Err(ToolError::Execution("Only echo is authorized".into()))
            }
        })
    }
}

struct RequestNote;
impl SessionPolicy for RequestNote {
    fn prepare_request<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        call: &'a mut ModelCall,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<(), String>> {
        Box::pin(async move {
            call.history.push(RuntimeMessage::system(REQUEST_NOTE));
            Ok(())
        })
    }
}

#[derive(Default)]
struct SnapshotHost(Mutex<Vec<Vec<u8>>>);
impl AgentHost for SnapshotHost {
    fn commit_run<'a>(
        &'a self,
        _: AgentRunIdentity<'a>,
        result: &'a AgentRunResult,
    ) -> AgentHostFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let snapshot = result.history.snapshot()?;
            let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
            // shortcut: Commits stay in memory here; a durable host must atomically write before Ok.
            self.0.lock().unwrap().push(bytes);
            Ok(())
        })
    }
}

fn bind(config: &mut AgentSessionConfig, executions: Arc<AtomicUsize>, host: Arc<SnapshotHost>) {
    config.host = host;
    config
        .tools
        .register_static(
            ToolDefinition::new(ModelTool {
                name: "echo".into(),
                description: "Return the supplied text.".into(),
                input_schema: json!({}), // register_static derives the schema from EchoArgs.
            }),
            Echo(executions),
            Arc::new(AllowEcho),
        )
        .unwrap();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tool_call = GenerateOutput::text(String::new(), None);
    tool_call.tool_calls.push(PendingToolCall {
        id: "echo-1".into(),
        function_name: "echo".into(),
        arguments: json!({"text":"hello"}),
        arguments_raw: r#"{"text":"hello"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    let model = Arc::new(OfflineModel(Mutex::new(VecDeque::from([
        tool_call,
        GenerateOutput::text("Echo returned hello.".into(), None),
        GenerateOutput::text("A separate branch.".into(), None),
    ]))));
    let executions = Arc::new(AtomicUsize::new(0));
    let host = Arc::new(SnapshotHost::default());
    let mut config = AgentSessionConfig::new(model, "offline").with_policy(RequestNote);
    bind(&mut config, executions.clone(), host.clone());
    let session = AgentSession::new(config);
    let first = session.prompt("Echo hello.").await?;
    assert_eq!(first.tool_records.len(), 1);
    assert_eq!(first.tool_records[0].output.as_deref(), Some("hello"));
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let original = session.entries();
    let original_leaf = session.active_leaf();

    // Fork after the tool result, keeping the original answer as an inactive branch.
    session
        .fork(original[original.len() - 2].id)
        .map_err(AgentRunError::Admission)?;
    session.prompt("Take another direction.").await?;
    assert_eq!(session.leaves().len(), 2);
    assert_eq!(&session.entries()[..original.len()], original);
    let branch_leaf = session.active_leaf();
    session
        .switch_branch(original_leaf)
        .map_err(AgentRunError::Admission)?;
    assert_eq!(
        session.context().last().unwrap().text_content(),
        first.content
    );
    session
        .switch_branch(branch_leaf)
        .map_err(AgentRunError::Admission)?;

    // Session snapshots add safe settings to the facts captured by commit_run.
    let bytes = serde_json::to_vec(&session.snapshot().map_err(AgentRunError::Admission)?)?;
    assert!(!String::from_utf8_lossy(&bytes).contains(REQUEST_NOTE));
    let saved: SessionSnapshot = serde_json::from_slice(&bytes)?;
    let old_entries = saved.entries().to_vec();
    let next_model = Arc::new(OfflineModel(Mutex::new(VecDeque::from([
        GenerateOutput::text("Restored without replaying echo.".into(), None),
    ]))));
    let mut config = AgentSessionConfig::from_snapshot(next_model, saved)?;
    assert!(config.tools.definitions().is_empty());
    // Live runtime, host, tools, authorization, and policy must all be rebound.
    let restored_host = Arc::new(SnapshotHost::default());
    bind(&mut config, executions.clone(), restored_host.clone());
    let restored = AgentSession::new(config.with_policy(RequestNote));
    assert_eq!(restored.entries(), old_entries);
    let answer = restored.prompt("Continue this branch.").await?;
    assert_eq!(&restored.entries()[..old_entries.len()], old_entries);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(host.0.lock().unwrap().len(), 2);
    let commits = restored_host.0.lock().unwrap();
    let committed: SessionSnapshot = serde_json::from_slice(&commits[0])?;
    assert_eq!(committed.entries(), restored.entries());
    println!("{} Tool executions: 1. Host commits: 3.", answer.content);
    Ok(())
}
