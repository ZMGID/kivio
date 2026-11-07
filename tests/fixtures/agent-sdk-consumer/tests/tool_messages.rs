use kivio_agent_sdk::{
    agent::{
        content::{AgentToolCall, AgentToolStatus},
        context::AgentContext,
        execute::{PreparedToolCall, ToolFuture, ToolPreparation, ToolProgress},
        rounds::{execute_tool_round, ToolRoundContext},
        ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{MessagePart, ModelArtifact, ModelMessage, ModelRole, PendingToolCall},
    tools::{ToolDefinition, ToolOutput},
};
use kivio_agent_sdk_consumer::{echo_definition, handle, Control, EchoTool, Events, Host};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

fn image(id: &str) -> ModelMessage {
    ModelMessage {
        role: ModelRole::User,
        content: vec![MessagePart::Image {
            mime_type: "image/png".into(),
            data: "AAAA".into(),
            path: Some(id.into()),
            detail: Some("high".into()),
        }],
    }
}

fn artifact() -> ModelArtifact {
    ModelArtifact {
        id: Some("artifact".into()),
        name: "result.png".into(),
        mime_type: "image/png".into(),
        data_url: "data:image/png;base64,AAAA".into(),
        size_bytes: Some(3),
        path: Some("result.png".into()),
    }
}

struct ImageTool;
impl ToolExecutor for ImageTool {
    fn before_tool_call<'a>(
        &'a self,
        _progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        _tool: &'a ToolDefinition,
        call: PendingToolCall,
        mut record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            if call.arguments["complete"] == true {
                record.status = AgentToolStatus::Success;
                record.artifacts = vec![artifact()];
                return Ok(ToolPreparation::Complete(
                    record,
                    "prepared output".into(),
                    vec![image(ctx.tool_call_id)],
                ));
            }
            Ok(ToolPreparation::Ready(PreparedToolCall {
                record,
                arguments: call.arguments,
                timeout_ms: Some(1000),
                timeout_message: "timed out".into(),
                output_limit: None,
                drain_on_cancel: false,
                feedback: vec![],
            }))
        })
    }

    fn call<'a>(
        &'a self,
        _progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        _tool: &'a ToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            Ok(ToolOutput {
                content: "tool output".into(),
                is_error: arguments["error"] == true,
                structured_content: Some(json!({"partial": true})),
                host_data: vec![artifact()],
                follow_up_messages: vec![image(ctx.tool_call_id)],
            })
        })
    }
}

fn call(id: &str, name: &str, arguments: Value) -> PendingToolCall {
    PendingToolCall {
        id: id.into(),
        function_name: name.into(),
        arguments_raw: arguments.to_string(),
        arguments,
        arguments_parse_error: None,
        signature: None,
    }
}

fn round_context() -> ToolRoundContext<'static> {
    ToolRoundContext {
        conversation_id: "conversation",
        run_id: "run",
        message_id: "message",
        generation: 1,
        round: 1,
        finish_reason: None,
    }
}

#[tokio::test]
async fn typed_tool_results_preserve_errors_artifacts_and_images_across_batches() {
    let control = Control::default();
    let events = Events::default();
    let context = AgentContext::new(&Host, &events, handle(&control));
    let mut parallel = echo_definition();
    parallel.parallel_safe = true;
    let mut serial = parallel.clone();
    serial.model.name = "serial".into();
    serial.label.name = "serial".into();
    serial.parallel_safe = false;
    let mut blocked = parallel.clone();
    blocked.model.name = "blocked".into();
    blocked.label.name = "blocked".into();
    let mut calls = (0..13)
        .map(|n| call(&format!("call_{n}"), "echo", json!({"error": n == 0})))
        .collect::<Vec<_>>();
    calls.push(call("serial", "serial", json!({})));
    calls.push(call("unknown", "missing", json!({})));
    let mut invalid = call("invalid", "echo", json!({}));
    invalid.arguments_raw = "{".into();
    invalid.arguments_parse_error = Some("incomplete JSON".into());
    calls.push(invalid);
    calls.push(call("blocked", "blocked", json!({})));
    let expected_ids = calls.iter().map(|call| call.id.clone()).collect::<Vec<_>>();
    let result = execute_tool_round(
        &context,
        &ImageTool,
        round_context(),
        &[parallel, serial],
        &[blocked],
        calls,
    )
    .await;

    assert!(!result.cancelled);
    assert_eq!(result.response_messages.len(), 17 + 14);
    for (n, message) in result.response_messages[..17].iter().enumerate() {
        assert_eq!(message.role, ModelRole::Tool);
        let [MessagePart::ToolResult {
            tool_call_id,
            is_error,
            artifacts,
            ..
        }] = message.content.as_slice()
        else {
            panic!("missing typed tool result");
        };
        assert_eq!(tool_call_id, &expected_ids[n]);
        assert_eq!(*is_error, n == 0 || n >= 14);
        assert_eq!(artifacts, &if n < 14 { vec![artifact()] } else { vec![] });
    }
    let expected_images = expected_ids[..14]
        .iter()
        .map(|id| image(id))
        .collect::<Vec<_>>();
    assert_eq!(result.response_messages[17..], expected_images);
    assert_eq!(result.tool_records[0].status, AgentToolStatus::Error);
    assert_eq!(result.tool_records[0].artifacts, vec![artifact()]);
    assert_eq!(
        result.tool_records[0].structured_content,
        Some(json!({"partial": true}))
    );
    assert!(result.tool_records[1..14]
        .iter()
        .all(|record| record.status == AgentToolStatus::Success));
}

#[tokio::test]
async fn preparation_completion_emits_one_terminal_record_without_executor_updates() {
    let control = Control::default();
    let events = Events::default();
    let context = AgentContext::new(&Host, &events, handle(&control));
    let executor = EchoTool::new(false);
    let result = execute_tool_round(
        &context,
        &executor,
        round_context(),
        &[echo_definition()],
        &[],
        vec![call("denied", "echo", json!({"text": "denied"}))],
    )
    .await;

    assert_eq!(executor.call_count(), 0);
    assert_eq!(result.tool_records[0].status, AgentToolStatus::Error);
    assert!(matches!(
        &result.response_messages[0].content[0],
        MessagePart::ToolResult { is_error: true, .. }
    ));
    let updates = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.kind.starts_with("tool-update:"))
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(updates, ["tool-update:Pending", "tool-update:Error"]);
}

#[tokio::test]
async fn preparation_completion_preserves_typed_follow_ups_and_artifacts() {
    let control = Control::default();
    let events = Events::default();
    let context = AgentContext::new(&Host, &events, handle(&control));
    let result = execute_tool_round(
        &context,
        &ImageTool,
        round_context(),
        &[echo_definition()],
        &[],
        vec![call("complete", "echo", json!({"complete": true}))],
    )
    .await;
    assert_eq!(result.response_messages.len(), 2);
    assert_eq!(result.response_messages[1], image("complete"));
    assert!(
        matches!(&result.response_messages[0].content[0], MessagePart::ToolResult { is_error: false, artifacts, .. } if artifacts == &vec![artifact()])
    );
    let completed = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.kind == "tool-update:Success")
        .count();
    assert_eq!(completed, 1);
}

#[tokio::test]
async fn cancellation_during_preparation_does_not_execute_side_effects() {
    struct CancelDuringPreparation {
        calls: AtomicUsize,
        drain_on_cancel: bool,
    }
    impl ToolExecutor for CancelDuringPreparation {
        fn before_tool_call<'a>(
            &'a self,
            progress: &'a ToolProgress<'a>,
            ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ToolDefinition,
            call: PendingToolCall,
            record: AgentToolCall,
        ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
            Box::pin(async move {
                let prepared = ImageTool
                    .before_tool_call(progress, ctx, tool, call, record)
                    .await?;
                progress.control.stop();
                let ToolPreparation::Ready(mut ready) = prepared else {
                    unreachable!()
                };
                ready.drain_on_cancel = self.drain_on_cancel;
                Ok(ToolPreparation::Ready(ready))
            })
        }

        fn call<'a>(
            &'a self,
            _progress: &'a ToolProgress<'a>,
            _ctx: &'a ToolExecutionContext<'a>,
            _tool: &'a ToolDefinition,
            _arguments: Value,
        ) -> ToolExecutorFuture<'a> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(ToolOutput::text("side effect")) })
        }
    }

    for drain_on_cancel in [false, true] {
        let control = Control::default();
        let events = Events::default();
        let context = AgentContext::new(&Host, &events, handle(&control));
        let executor = CancelDuringPreparation {
            calls: AtomicUsize::new(0),
            drain_on_cancel,
        };
        let result = execute_tool_round(
            &context,
            &executor,
            round_context(),
            &[echo_definition()],
            &[],
            vec![call("cancelled", "echo", json!({}))],
        )
        .await;
        assert!(result.cancelled);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        assert_eq!(result.tool_records[0].status, AgentToolStatus::Cancelled);
        assert!(matches!(
            &result.response_messages[0].content[0],
            MessagePart::ToolResult { is_error: true, .. }
        ));
    }
}
