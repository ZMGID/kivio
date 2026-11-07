use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use kivio_agent_sdk::{
    agent::{
        content::{AgentToolCall, AgentToolStatus},
        context::AgentContext,
        control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission},
        events::{AgentEvent, AgentEventKind, AgentRunIdentity, EventDelivery, InputKind},
        execute::{
            execute_tool_call, limit_tool_text_for_model, PreparedToolCall, ToolCallResult,
            ToolPreparation, ToolProgress, ToolResultUpdate,
        },
        host::{AgentHost, AgentHostFuture, SpeculativeTaskScope},
        rounds::{execute_tool_round, ToolRoundContext},
        SteeringMessage, ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{ModelArtifact, ModelMessage, ModelRole, ModelTool, PendingToolCall},
    tools::{
        StaticTool, ToolAuthorization, ToolDefinition, ToolError, ToolFuture, ToolOutput,
        ToolRegistry,
    },
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

struct Host;
impl AgentHost for Host {}
impl AgentRunControl for Host {
    fn active(&self) -> bool {
        true
    }
    fn stop(&self) -> StopAdmission {
        StopAdmission::Requested
    }
    fn submit(&self, _: InputKind, _: SteeringMessage) -> InputAdmission {
        InputAdmission::Unsupported
    }
    fn take(&self, _: InputKind) -> Vec<SteeringMessage> {
        Vec::new()
    }
    fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

fn identity() -> AgentRunIdentity<'static> {
    AgentRunIdentity {
        conversation_id: "conversation",
        run_id: "run",
        message_id: "message",
        generation: 1,
    }
}

fn context() -> ToolExecutionContext<'static> {
    ToolExecutionContext {
        conversation_id: "conversation",
        run_id: "run",
        message_id: "message",
        generation: 1,
        round: 1,
        tool_call_id: "call",
    }
}

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition::new(ModelTool {
        name: name.into(),
        description: "test".into(),
        input_schema: json!(false),
    })
}

fn pending(name: &str, arguments: Value) -> PendingToolCall {
    PendingToolCall {
        id: "call".into(),
        function_name: name.into(),
        arguments_raw: arguments.to_string(),
        arguments,
        arguments_parse_error: None,
        signature: None,
    }
}

struct Authorization {
    allowed: bool,
    calls: Arc<AtomicUsize>,
}
impl ToolAuthorization for Authorization {
    fn authorize<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.allowed {
                Ok(())
            } else {
                Err(ToolError::Execution("denied".into()))
            }
        })
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Args {
    values: Vec<u32>,
}
struct Sum(Arc<AtomicUsize>);
impl StaticTool for Sum {
    type Args = Args;
    type Output = u32;
    type Error = &'static str;
    type HostData = Vec<ModelArtifact>;

    fn execute<'a>(
        &'a self,
        args: Args,
    ) -> ToolFuture<'a, Result<ToolOutput<Self::HostData, u32>, Self::Error>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            let sum = args.values.iter().sum::<u32>();
            Ok(ToolOutput {
                content: sum.to_string(),
                structured_content: Some(sum),
                host_data: vec![ModelArtifact {
                    id: None,
                    name: "artifact".into(),
                    mime_type: "text/plain".into(),
                    data_url: "data:text/plain,private".into(),
                    size_bytes: None,
                    path: None,
                }],
                follow_up_messages: vec![ModelMessage::text(ModelRole::User, "follow-up")],
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn static_registration_generates_schema_and_returns_typed_output_after_authorization() {
    let calls = Arc::new(AtomicUsize::new(0));
    let authorizations = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry
        .register_static(
            definition("sum"),
            Sum(calls.clone()),
            Arc::new(Authorization {
                allowed: true,
                calls: authorizations.clone(),
            }),
        )
        .unwrap();
    let definitions = registry.definitions();
    assert_eq!(
        definitions[0].model.input_schema["properties"]["values"]["type"],
        "array"
    );
    let host = Host;
    let sink = |_: &AgentEvent<'_>| EventDelivery::Accepted;
    let host = AgentContext::new(&host, &sink, AgentRunHandle::new(identity(), &host));
    let ToolCallResult {
        record,
        content,
        follow_up_messages: follow_ups,
        ..
    } = execute_tool_call(
        &host,
        &registry,
        &context(),
        &definitions[0],
        pending("sum", json!({"values":[2,3]})),
    )
    .await;
    assert_eq!(record.status, AgentToolStatus::Success);
    assert_eq!(content, "5");
    assert_eq!(record.structured_content, Some(json!(5)));
    assert_eq!(record.artifacts[0].name, "artifact");
    assert_eq!(
        follow_ups,
        vec![ModelMessage::text(ModelRole::User, "follow-up")]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(authorizations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn schema_and_authorization_rejections_never_execute_the_static_tool() {
    for (arguments, allowed, expected_authorizations, status) in [
        (json!({"values":[-1]}), true, 0, AgentToolStatus::Error),
        (
            json!({"values":[1],"extra":true}),
            true,
            0,
            AgentToolStatus::Error,
        ),
        (json!({"values":[1]}), false, 1, AgentToolStatus::Skipped),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let authorizations = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register_static(
                definition("sum"),
                Sum(calls.clone()),
                Arc::new(Authorization {
                    allowed,
                    calls: authorizations.clone(),
                }),
            )
            .unwrap();
        let definitions = registry.definitions();
        let host = Host;
        let statuses = Mutex::new(Vec::new());
        let sink = |event: &AgentEvent<'_>| {
            if let AgentEventKind::ToolUpdated(record) = &event.kind {
                statuses.lock().unwrap().push(record.status.clone());
            }
            EventDelivery::Accepted
        };
        let host = AgentContext::new(&host, &sink, AgentRunHandle::new(identity(), &host));
        let ToolCallResult {
            record,
            follow_up_messages: follow_ups,
            ..
        } = execute_tool_call(
            &host,
            &registry,
            &context(),
            &definitions[0],
            pending("sum", arguments),
        )
        .await;
        assert_eq!(record.status, status);
        assert_eq!(statuses.lock().unwrap().last(), Some(&status));
        assert!(record.error.is_some());
        assert!(follow_ups.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            authorizations.load(Ordering::SeqCst),
            expected_authorizations
        );
    }
}

#[derive(Default)]
struct Dynamic {
    calls: AtomicUsize,
    after: AtomicUsize,
    speculation: AtomicUsize,
}
impl ToolExecutor for Dynamic {
    fn before_tool_call<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            assert_eq!(tool.model.input_schema, json!(false));
            Ok(ToolPreparation::Ready(PreparedToolCall {
                record,
                arguments: call.arguments,
                timeout_ms: None,
                timeout_message: "managed".into(),
                output_limit: Some(2),
                drain_on_cancel: true,
                feedback: vec!["hook".into()],
            }))
        })
    }
    fn call<'a>(
        &'a self,
        _progress: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(arguments, json!({"after_approval":true}));
            Ok(ToolOutput::text("dynamic output"))
        })
    }
    fn after_tool_call<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: Value,
        content: &'a str,
        feedback: Vec<String>,
    ) -> ToolFuture<'a, Result<ToolResultUpdate, String>> {
        Box::pin(async move {
            self.after.fetch_add(1, Ordering::SeqCst);
            assert_eq!(feedback, vec!["hook"]);
            Ok(ToolResultUpdate {
                content: Some(format!("{content} feedback")),
                follow_up_messages: None,
            })
        })
    }
    fn unknown_feedback(&self, _: &str) -> Option<String> {
        Some("unknown feedback".into())
    }
    fn blocked_feedback(&self, _: &ToolDefinition) -> String {
        "blocked feedback".into()
    }
    fn start_speculation(
        &self,
        _: &ToolExecutionContext<'_>,
        tools: &[ToolDefinition],
    ) -> Option<Box<dyn SpeculativeTaskScope>> {
        assert!(tools.iter().any(|tool| tool.speculative));
        self.speculation.fetch_add(1, Ordering::SeqCst);
        None
    }
}

#[tokio::test]
async fn dynamic_executor_keeps_validation_order_limits_feedback_and_alias_dispatch() {
    let dynamic = Arc::new(Dynamic::default());
    let mut tool = definition("dynamic");
    tool.label.name = "Display".into();
    tool.aliases.push("alias".into());
    tool.speculative = true;
    let mut registry = ToolRegistry::new();
    registry.register_dynamic(tool, dynamic.clone()).unwrap();
    let definitions = registry.definitions();
    let host = Host;
    let sink = |_: &AgentEvent<'_>| EventDelivery::Accepted;
    let host = AgentContext::new(&host, &sink, AgentRunHandle::new(identity(), &host));
    let result = execute_tool_round(
        &host,
        &registry,
        ToolRoundContext {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation: 1,
            round: 1,
            finish_reason: None,
        },
        &definitions,
        &[],
        vec![pending("alias", json!({"after_approval":true}))],
    )
    .await;
    assert_eq!(result.tool_records[0].status, AgentToolStatus::Success);
    assert_eq!(result.tool_records[0].output_limit, Some(2));
    assert_eq!(
        result.tool_records[0].output.as_deref(),
        Some("dynamic output")
    );
    assert_eq!(
        result.response_messages[0].text_content(),
        format!(
            "{} feedback",
            limit_tool_text_for_model("dynamic output", Some(2))
        )
    );
    assert_eq!(dynamic.calls.load(Ordering::SeqCst), 1);
    assert_eq!(dynamic.after.load(Ordering::SeqCst), 1);
    assert_eq!(
        registry.unknown_feedback("unknown").as_deref(),
        Some("unknown feedback")
    );
    assert_eq!(
        registry.blocked_feedback(&definitions[0]),
        "blocked feedback"
    );
    registry.start_speculation(&context(), &definitions);
    assert_eq!(dynamic.speculation.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn registry_rejects_ambiguous_identifiers_and_snapshot_keeps_its_definitions() {
    let dynamic = Arc::new(Dynamic::default());
    let mut registry = ToolRegistry::new();
    let mut first = definition("read");
    first.key = "internal-read".into();
    first.label.name = "Read file".into();
    first.aliases.push("legacy".into());
    registry.register_dynamic(first, dynamic.clone()).unwrap();
    let snapshot = registry.snapshot();
    for name in ["READ", "Read file", "legacy", "internal-read"] {
        assert!(
            registry
                .register_dynamic(definition(name), dynamic.clone())
                .is_err(),
            "{name}"
        );
    }
    let mut alias_collision = definition("other");
    alias_collision.aliases.push("rEaD".into());
    assert!(registry
        .register_dynamic(alias_collision, dynamic.clone())
        .is_err());
    assert_eq!(registry.definitions().len(), 1);
    registry
        .register_dynamic(definition("write"), dynamic)
        .unwrap();
    assert_eq!(registry.definitions().len(), 2);
    assert_eq!(snapshot.definitions().len(), 1);
    assert_eq!(snapshot.definitions()[0].key, "internal-read");
    let host = Host;
    let sink = |_: &AgentEvent<'_>| EventDelivery::Accepted;
    let host = AgentContext::new(&host, &sink, AgentRunHandle::new(identity(), &host));
    let result = execute_tool_call(
        &host,
        &snapshot,
        &context(),
        &registry.definitions()[1],
        pending("write", json!({"after_approval":true})),
    )
    .await;
    assert!(result.content.contains("not registered"));
}

#[test]
fn distinct_speculative_owners_cannot_receive_each_others_calls() {
    let first = Arc::new(Dynamic::default());
    let second = Arc::new(Dynamic::default());
    let mut registry = ToolRegistry::new();
    for (name, executor) in [("first", first.clone()), ("second", second.clone())] {
        let mut tool = definition(name);
        tool.speculative = true;
        registry.register_dynamic(tool, executor).unwrap();
    }
    assert!(registry
        .start_speculation(&context(), &registry.definitions())
        .is_none());
    assert_eq!(first.speculation.load(Ordering::SeqCst), 0);
    assert_eq!(second.speculation.load(Ordering::SeqCst), 0);
}
