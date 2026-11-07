use kivio_agent_sdk::{
    agent::{
        content::{AgentToolCall, AgentToolStatus},
        context::AgentContext,
        control::AgentRunControl,
        events::{AgentEvent, AgentEventKind, EventDelivery},
        execute::{ToolFuture, ToolPreparation, ToolProgress, ToolResultUpdate},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        rounds::{execute_tool_round, ToolRoundContext, ToolRoundResult},
        run_agent_loop,
        session::NoSessionPolicy,
        AgentFailure, AgentRunError, ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{
        GenerateOutput, MessagePart, ModelArtifact, ModelError, ModelMessage, ModelRole, ModelTool,
        PendingToolCall,
    },
    tools::{
        schema::validate_arguments, StaticTool, ToolAuthorization, ToolDefinition, ToolError,
        ToolOutput, ToolRegistry,
    },
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, Events, Host};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Args {
    path: String,
}

struct Write(Arc<Mutex<Vec<String>>>);
impl StaticTool for Write {
    type Args = Args;
    type Output = Value;
    type Error = &'static str;
    type HostData = Vec<ModelArtifact>;
    fn execute<'a>(
        &'a self,
        args: Args,
    ) -> ToolFuture<'a, Result<ToolOutput<Self::HostData>, Self::Error>> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push(format!("execute:{}", args.path));
            if args.path == "slow" {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok(ToolOutput {
                content: format!("raw:{}", args.path),
                structured_content: Some(json!({"written":args.path})),
                host_data: vec![ModelArtifact {
                    id: None,
                    name: "raw-artifact".into(),
                    mime_type: "text/plain".into(),
                    data_url: "data:text/plain,raw".into(),
                    size_bytes: None,
                    path: None,
                }],
                follow_up_messages: vec![ModelMessage::text(ModelRole::User, "original follow-up")],
                is_error: false,
            })
        })
    }
}
struct Authorization(Arc<Mutex<Vec<String>>>);
impl ToolAuthorization for Authorization {
    fn authorize<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        args: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            let path = args["path"].as_str().unwrap();
            self.0.lock().unwrap().push(format!("authorize:{path}"));
            if path == "blocked" {
                Err(ToolError::Execution("host denied final path".into()))
            } else {
                Ok(())
            }
        })
    }
}

#[derive(Clone, Copy)]
enum Before {
    Pass,
    Reject,
    Rewrite(&'static str),
    InvalidRewrite,
    Fail,
    Cancel,
    CancelPending,
    Managed,
}
#[derive(Clone, Copy)]
enum After {
    Keep,
    Replace,
    ClearFollowUps,
    Fail,
    Cancel,
    CancelPending,
}
struct Policy {
    inner: ToolRegistry,
    log: Arc<Mutex<Vec<String>>>,
    before: Before,
    after: After,
    cancel: Option<Arc<Control>>,
}

fn reject(mut record: AgentToolCall, status: AgentToolStatus, message: String) -> ToolPreparation {
    record.status = status;
    record.error = Some(message.clone());
    ToolPreparation::Complete(record, message, vec![])
}

impl ToolExecutor for Policy {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        mut call: PendingToolCall,
        mut record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            if let Err(error) = validate_arguments(&tool.model.input_schema, &call.arguments) {
                return Ok(reject(record, AgentToolStatus::Error, error));
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("before:{}", ctx.tool_call_id));
            match self.before {
                Before::Reject => {
                    return Ok(reject(
                        record,
                        AgentToolStatus::Skipped,
                        "policy rejected".into(),
                    ))
                }
                Before::Fail => return Err("before failed".into()),
                Before::Rewrite(path) => call.arguments = json!({"path":path}),
                Before::InvalidRewrite => call.arguments = json!({"path":3}),
                Before::Cancel | Before::CancelPending => {
                    progress.control.stop();
                    if matches!(self.before, Before::CancelPending) {
                        return std::future::pending().await;
                    }
                    return Err("late before failure".into());
                }
                _ => {}
            }
            call.arguments_raw = call.arguments.to_string();
            record.arguments = call.arguments_raw.clone();
            let mut prepared = self
                .inner
                .before_tool_call(progress, ctx, tool, call, record)
                .await?;
            if let ToolPreparation::Ready(ready) = &mut prepared {
                ready.drain_on_cancel = matches!(self.before, Before::Managed);
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
        Box::pin(async move {
            if let Some(control) = &self.cancel {
                control.stop();
                tokio::task::yield_now().await;
            }
            self.inner.call(progress, ctx, tool, args).await
        })
    }
    fn after_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: Value,
        _: &'a str,
        _: Vec<String>,
    ) -> ToolFuture<'a, Result<ToolResultUpdate, String>> {
        Box::pin(async move {
            self.log
                .lock()
                .unwrap()
                .push(format!("after:{}", ctx.tool_call_id));
            match self.after {
                After::Fail => Err("after failed".into()),
                After::Cancel | After::CancelPending => {
                    progress.control.stop();
                    if matches!(self.after, After::CancelPending) {
                        return std::future::pending().await;
                    }
                    Ok(ToolResultUpdate {
                        content: Some("late output".into()),
                        follow_up_messages: Some(vec![]),
                    })
                }
                After::Replace => Ok(ToolResultUpdate {
                    content: Some("public output".into()),
                    follow_up_messages: Some(vec![ModelMessage::text(
                        ModelRole::User,
                        "replacement follow-up",
                    )]),
                }),
                After::ClearFollowUps => Ok(ToolResultUpdate {
                    content: None,
                    follow_up_messages: Some(vec![]),
                }),
                After::Keep => Ok(ToolResultUpdate::default()),
            }
        })
    }
}

fn registry(
    before: Before,
    after: After,
    parallel: bool,
    cancel: Option<Arc<Control>>,
) -> (ToolRegistry, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(vec![]));
    let mut definition = ToolDefinition::new(ModelTool {
        name: "write".into(),
        description: "test write".into(),
        input_schema: json!(false),
    });
    definition.parallel_safe = parallel;
    let mut inner = ToolRegistry::new();
    inner
        .register_static(
            definition,
            Write(log.clone()),
            Arc::new(Authorization(log.clone())),
        )
        .unwrap();
    let mut registry = ToolRegistry::new();
    registry
        .register_dynamic(
            inner.definitions().remove(0),
            Arc::new(Policy {
                inner,
                log: log.clone(),
                before,
                after,
                cancel,
            }),
        )
        .unwrap();
    (registry, log)
}
fn call(id: &str, path: &str) -> PendingToolCall {
    let args = json!({"path":path});
    PendingToolCall {
        id: id.into(),
        function_name: "write".into(),
        arguments_raw: args.to_string(),
        arguments: args,
        arguments_parse_error: None,
        signature: None,
    }
}
async fn run(
    registry: &ToolRegistry,
    control: &Control,
    calls: Vec<PendingToolCall>,
) -> ToolRoundResult {
    let terminals = Mutex::new(vec![]);
    let events = |event: &AgentEvent<'_>| {
        if let AgentEventKind::ToolUpdated(record) = &event.kind {
            if !matches!(
                record.status,
                AgentToolStatus::Pending | AgentToolStatus::Running
            ) {
                terminals
                    .lock()
                    .unwrap()
                    .push((record.id.clone(), record.status.clone()));
            }
        }
        EventDelivery::Accepted
    };
    let context = AgentContext::new(&Host, &events, handle(control));
    let result = execute_tool_round(
        &context,
        registry,
        ToolRoundContext {
            conversation_id: "consumer",
            run_id: "run",
            message_id: "reply",
            generation: 1,
            round: 1,
            finish_reason: None,
        },
        &registry.definitions(),
        &[],
        calls,
    )
    .await;
    for record in &result.tool_records {
        assert_eq!(
            terminals
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, status)| *id == record.id && *status == record.status)
                .count(),
            1
        );
    }
    result
}
fn paired_ids(result: &ToolRoundResult) -> Vec<&str> {
    result
        .response_messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| match part {
            MessagePart::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn before_rejection_is_paired_and_rewritten_arguments_are_validated_and_authorized_once() {
    for (before, status, expected_log) in [
        (Before::Reject, AgentToolStatus::Skipped, vec!["before:a"]),
        (
            Before::InvalidRewrite,
            AgentToolStatus::Error,
            vec!["before:a"],
        ),
        (
            Before::Rewrite("blocked"),
            AgentToolStatus::Skipped,
            vec!["before:a", "authorize:blocked"],
        ),
        (
            Before::Rewrite("changed"),
            AgentToolStatus::Success,
            vec![
                "before:a",
                "authorize:changed",
                "execute:changed",
                "after:a",
            ],
        ),
    ] {
        let (registry, log) = registry(before, After::Keep, false, None);
        let result = run(&registry, &Control::default(), vec![call("a", "original")]).await;
        assert_eq!(paired_ids(&result), vec!["a"]);
        assert_eq!(result.tool_records[0].status, status);
        assert!(result.failure.is_none());
        assert_eq!(*log.lock().unwrap(), expected_log);
    }
}

#[tokio::test]
async fn after_replaces_only_explicit_model_fields_and_keeps_raw_execution_facts() {
    for after in [After::Replace, After::ClearFollowUps] {
        let (registry, log) = registry(Before::Pass, after, false, None);
        let result = run(&registry, &Control::default(), vec![call("a", "original")]).await;
        let record = &result.tool_records[0];
        assert_eq!(record.output.as_deref(), Some("raw:original"));
        assert_eq!(
            record.structured_content,
            Some(json!({"written":"original"}))
        );
        assert_eq!(record.artifacts[0].name, "raw-artifact");
        assert_eq!(paired_ids(&result), vec!["a"]);
        if matches!(after, After::Replace) {
            assert_eq!(result.response_messages[0].text_content(), "public output");
            assert_eq!(
                result.response_messages[1].text_content(),
                "replacement follow-up"
            );
        } else {
            assert_eq!(result.response_messages[0].text_content(), "raw:original");
            assert_eq!(result.response_messages.len(), 1);
        }
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                "before:a",
                "authorize:original",
                "execute:original",
                "after:a"
            ]
        );
    }
}

#[tokio::test]
async fn hook_failures_preserve_facts_pair_remaining_calls_and_never_retry_side_effects() {
    for (before, after, hook, executed) in [
        (Before::Fail, After::Keep, "before_tool_call", false),
        (Before::Pass, After::Fail, "after_tool_call", true),
    ] {
        let (registry, log) = registry(before, after, false, None);
        let result = run(
            &registry,
            &Control::default(),
            vec![call("a", "original"), call("b", "unstarted")],
        )
        .await;
        assert_eq!(paired_ids(&result), vec!["a", "b"]);
        assert!(matches!(result.failure,Some(AgentFailure::Hook{hook:actual,..}) if actual==hook));
        assert!(!result.cancelled);
        assert_eq!(result.tool_records[1].status, AgentToolStatus::Skipped);
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|line| line.starts_with("execute:"))
                .count(),
            usize::from(executed)
        );
        if executed {
            assert_eq!(result.tool_records[0].status, AgentToolStatus::Success);
            assert_eq!(result.response_messages[0].text_content(), "raw:original");
            assert_eq!(
                result.response_messages[2].text_content(),
                "original follow-up"
            );
        }
    }
}

#[tokio::test]
async fn cancellation_during_hooks_discards_late_values_and_stops_next_calls() {
    for (before, after, executed) in [
        (Before::Cancel, After::Keep, false),
        (Before::CancelPending, After::Keep, false),
        (Before::Pass, After::Cancel, true),
        (Before::Pass, After::CancelPending, true),
    ] {
        let (registry, log) = registry(before, after, false, None);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            run(
                &registry,
                &Control::default(),
                vec![call("a", "original"), call("b", "unstarted")],
            ),
        )
        .await
        .unwrap();
        assert_eq!(paired_ids(&result), vec!["a", "b"]);
        assert!(result.cancelled);
        assert!(result.failure.is_none());
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|line| line.starts_with("execute:"))
                .count(),
            usize::from(executed)
        );
        if executed {
            assert_eq!(result.response_messages[0].text_content(), "raw:original");
        }
    }
}

#[tokio::test]
async fn parallel_hook_failure_drains_completed_tools_and_preserves_model_result_order() {
    let (registry, log) = registry(Before::Managed, After::Fail, true, None);
    let calls = (0..13)
        .map(|index| call(&index.to_string(), if index == 0 { "slow" } else { "fast" }))
        .collect();
    let result = run(&registry, &Control::default(), calls).await;
    assert_eq!(
        paired_ids(&result),
        (0..13).map(|index| index.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(
        result.tool_records[..12]
            .iter()
            .filter(|record| record.status == AgentToolStatus::Success)
            .count(),
        12
    );
    assert_eq!(result.tool_records[12].status, AgentToolStatus::Skipped);
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("execute:"))
            .count(),
        12
    );
    assert!(matches!(
        result.failure,
        Some(AgentFailure::Hook {
            hook: "after_tool_call",
            ..
        })
    ));
}

#[tokio::test]
async fn cancelled_managed_tool_drains_its_real_result_without_starting_the_after_hook() {
    let control = Arc::new(Control::default());
    let (registry, log) = registry(
        Before::Managed,
        After::Replace,
        false,
        Some(control.clone()),
    );
    let result = run(
        &registry,
        &control,
        vec![call("a", "slow"), call("b", "unstarted")],
    )
    .await;
    assert!(result.cancelled);
    assert!(result.failure.is_none());
    assert_eq!(paired_ids(&result), vec!["a", "b"]);
    assert_eq!(result.tool_records[0].status, AgentToolStatus::Success);
    assert_eq!(result.response_messages[0].text_content(), "raw:slow");
    assert_eq!(
        *log.lock().unwrap(),
        vec!["before:a", "authorize:slow", "execute:slow"]
    );
}

struct OneRound(std::sync::atomic::AtomicUsize);
impl ModelRuntime for OneRound {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            assert_eq!(
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                0,
                "no model request may follow a hook failure or cancellation"
            );
            sink.request_started(&call);
            let mut output = GenerateOutput::text(String::new(), None);
            output.tool_calls = vec![self::call("a", "original"), self::call("b", "unstarted")];
            Ok(output)
        })
    }
}

#[tokio::test]
async fn loop_hook_failure_returns_partial_paired_history_and_cancellation_never_requests_again() {
    for (before, after, hook) in [
        (Before::Fail, After::Keep, Some("before_tool_call")),
        (Before::Pass, After::Fail, Some("after_tool_call")),
        (Before::Pass, After::CancelPending, None),
    ] {
        let model = OneRound(std::sync::atomic::AtomicUsize::new(0));
        let (registry, _) = registry(before, after, false, None);
        let mut cfg = config(&model, initial_history());
        cfg.tools = registry.definitions();
        let events = Events::default();
        let control = Control::default();
        let result = run_agent_loop(
            cfg,
            &Host,
            &registry,
            &mut NoSessionPolicy,
            &events,
            handle(&control),
        )
        .await;
        let partial = match (hook, result) {
            (
                Some(hook),
                Err(AgentRunError::Failed {
                    cause: AgentFailure::Hook { hook: actual, .. },
                    partial,
                }),
            ) => {
                assert_eq!(actual, hook);
                *partial
            }
            (None, Ok(result)) => result,
            _ => panic!("unexpected run result"),
        };
        let ids = partial
            .history
            .context()
            .iter()
            .filter_map(|message| message.model_message())
            .flat_map(|message| message.content.iter())
            .filter_map(|part| match part {
                MessagePart::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(partial.tool_records.len(), 2);
        if matches!(after, After::Fail | After::CancelPending) {
            assert_eq!(
                partial.tool_records[0].output.as_deref(),
                Some("raw:original")
            );
        }
        assert_eq!(
            events
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.kind.starts_with("end:"))
                .count(),
            1
        );
        assert_eq!(model.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

struct PanickingHook {
    inner: ToolRegistry,
    before: bool,
    synchronous: bool,
}
impl ToolExecutor for PanickingHook {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        assert!(
            !(self.before && self.synchronous),
            "before construction panic"
        );
        Box::pin(async move {
            assert!(!self.before, "before polling panic");
            self.inner
                .before_tool_call(progress, ctx, tool, call, record)
                .await
        })
    }
    fn call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        args: Value,
    ) -> ToolExecutorFuture<'a> {
        self.inner.call(progress, ctx, tool, args)
    }
    fn after_tool_call<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: Value,
        _: &'a str,
        _: Vec<String>,
    ) -> ToolFuture<'a, Result<ToolResultUpdate, String>> {
        assert!(self.before || !self.synchronous, "after construction panic");
        Box::pin(async move {
            assert!(self.before, "after polling panic");
            Ok(ToolResultUpdate::default())
        })
    }
}

#[tokio::test]
async fn hook_panics_keep_paired_partial_history_and_never_rerun_executed_tools() {
    for before in [true, false] {
        for synchronous in [true, false] {
            let model = OneRound(std::sync::atomic::AtomicUsize::new(0));
            let (inner, log) = registry(Before::Pass, After::Keep, false, None);
            let mut cfg = config(&model, initial_history());
            cfg.tools = inner.definitions();
            let executor = PanickingHook {
                inner,
                before,
                synchronous,
            };
            let control = Control::default();
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
            let AgentRunError::Failed {
                cause: AgentFailure::Hook { hook, message },
                partial,
            } = error
            else {
                panic!("expected a sourced hook failure")
            };
            assert_eq!(
                hook,
                if before {
                    "before_tool_call"
                } else {
                    "after_tool_call"
                }
            );
            assert!(message.contains("panicked"));
            let results = partial
                .history
                .context()
                .iter()
                .filter_map(|message| message.model_message())
                .flat_map(|message| message.content.iter())
                .filter_map(|part| match part {
                    MessagePart::ToolResult {
                        tool_call_id,
                        content,
                        ..
                    } => Some((tool_call_id.as_str(), content.as_str())),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                results.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                vec!["a", "b"]
            );
            assert_eq!(partial.tool_records[1].status, AgentToolStatus::Skipped);
            assert_eq!(
                log.lock()
                    .unwrap()
                    .iter()
                    .filter(|line| line.starts_with("execute:"))
                    .count(),
                usize::from(!before)
            );
            if before {
                assert_eq!(partial.tool_records[0].status, AgentToolStatus::Skipped);
                assert!(results[0].1.contains("panicked"));
            } else {
                let record = &partial.tool_records[0];
                assert_eq!(record.status, AgentToolStatus::Success);
                assert_eq!(record.output.as_deref(), Some("raw:original"));
                assert_eq!(
                    record.structured_content,
                    Some(json!({"written":"original"}))
                );
                assert_eq!(record.artifacts[0].name, "raw-artifact");
                assert_eq!(results[0].1, "raw:original");
                assert!(partial
                    .history
                    .context()
                    .iter()
                    .any(|message| message.text_content() == "original follow-up"));
            }
            assert_eq!(model.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }
}
