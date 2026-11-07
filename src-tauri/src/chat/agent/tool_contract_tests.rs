use super::*;
use crate::chat::agent::content::AgentToolStatus;
use crate::chat::session_policy::run_session;
use crate::chat::tool_runtime::{ToolDispatcher, ToolExecutionContext};
use crate::chat::tools::{call_typed, input_schema, StaticTool, ToolFuture, ToolOutput};
use crate::mcp::ChatToolDefinition;

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CountArgs {
    input: CountInput,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CountInput {
    value: u32,
}

struct Count;

impl StaticTool for Count {
    type Args = CountArgs;
    type Output = u32;
    type Error = &'static str;
    type HostData = Vec<crate::chat::model::ModelArtifact>;

    fn execute<'a>(
        &'a self,
        args: CountArgs,
    ) -> ToolFuture<'a, Result<ToolOutput<Self::HostData, u32>, Self::Error>> {
        Box::pin(async move {
            if args.input.value == 0 {
                return Err("zero rejected by typed tool");
            }
            let mut output = ToolOutput::text(format!("count={}", args.input.value));
            output.structured_content = Some(args.input.value);
            Ok(output)
        })
    }
}

#[derive(Default)]
struct RegisteredExecutor(Mutex<Vec<String>>);

impl ToolDispatcher for RegisteredExecutor {
    fn call<'a>(
        &'a self,
        _progress: &'a crate::chat::agent::execute::ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
    ) -> super::super::super::execute::ToolExecutorFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(tool.name.clone());
            match tool.name.as_str() {
                "count" => call_typed(&Count, arguments)
                    .await
                    .map_err(|error| error.to_string()),
                "dynamic" => crate::chat::tool_output::from_mcp(
                    tool,
                    crate::mcp::result::parse_tool_result(serde_json::json!({
                        "content": [{"type":"text", "text":"dynamic answer"}],
                        "structuredContent": {"answer": 42},
                    })),
                ),
                "prepare_failure" => crate::chat::tool_output::from_mcp(
                    tool,
                    crate::mcp::types::McpToolCallResult {
                        content: "must not become success".into(),
                        follow_up_user_messages: vec![
                            serde_json::json!({"role":"assistant", "content":"bad follow-up"}),
                        ],
                        ..Default::default()
                    },
                ),
                "slow" => {
                    sleep(Duration::from_secs(60)).await;
                    Ok(ToolOutput::text("must time out"))
                }
                _ => panic!("an unregistered tool reached the executor"),
            }
        })
    }
}

fn definition(name: &str) -> ChatToolDefinition {
    ChatToolDefinition {
        id: format!("test_{name}"),
        name: name.into(),
        description: "Explicitly registered test tool".into(),
        source: "registered".into(),
        server_id: None,
        server_name: None,
        input_schema: if name == "count" {
            input_schema::<Count>()
        } else {
            serde_json::json!({"type":"object"})
        },
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

#[tokio::test]
async fn registered_typed_and_dynamic_tools_share_the_real_loop() {
    let mut dynamic = definition("dynamic");
    dynamic.source = "mcp".into();
    dynamic.server_id = Some("fixture".into());
    let calls = vec![
        (
            "typed",
            "count".to_string(),
            serde_json::json!({"input":{"value":"7"}}),
        ),
        ("dynamic", dynamic.openai_tool_name(), serde_json::json!({})),
        (
            "invalid",
            "count".into(),
            serde_json::json!({"input":{"value":-1}}),
        ),
        (
            "error",
            "count".into(),
            serde_json::json!({"input":{"value":0}}),
        ),
        ("unknown", "not_registered".into(), serde_json::json!({})),
        (
            "unselected_native",
            "bash".into(),
            serde_json::json!({"command":"must never execute"}),
        ),
        ("prepare", "prepare_failure".into(), serde_json::json!({})),
        ("timeout", "slow".into(), serde_json::json!({})),
    ];
    let wire_calls: Vec<_> = calls.iter().map(|(id, name, arguments)| serde_json::json!({
        "id":id, "type":"function", "function":{"name":name, "arguments":arguments.to_string()}
    })).collect();
    let server =
        MockModelServer::start(vec![
        MockResponse::Sse(sse_from_completion_json(&serde_json::json!({"choices":[{
            "message":{"role":"assistant","tool_calls":wire_calls}, "finish_reason":"tool_calls"
        }]}).to_string())),
        final_answer_sse("finished"),
    ]);
    let state = test_app_state();
    let mut config = test_run_config(&state, &server.base_url);
    config.tools = vec![
        definition("count"),
        dynamic,
        definition("prepare_failure"),
        definition("slow"),
    ];
    config.settings.chat_tools.tool_timeout_ms = 10;
    let executor = RegisteredExecutor::default();
    let host = TestHost::default();
    let identity = AgentRunIdentity {
        conversation_id: "conversation",
        run_id: "run",
        message_id: "message",
        generation: 1,
    };
    let handle = AgentRunHandle::new(identity, &host);
    let result = run_session(config, &host, &executor, &host, handle)
        .await
        .unwrap();
    assert_eq!(result.content, "finished");
    assert_eq!(
        *executor.0.lock().unwrap(),
        vec!["count", "dynamic", "count", "prepare_failure", "slow"]
    );
    for id in ["typed", "dynamic"] {
        assert!(result
            .tool_records
            .iter()
            .any(|record| record.id == id && matches!(record.status, AgentToolStatus::Success)));
    }
    for id in ["invalid", "error", "unknown", "prepare", "timeout"] {
        assert!(
            result
                .tool_records
                .iter()
                .any(|record| record.id == id && matches!(record.status, AgentToolStatus::Error)),
            "{id}"
        );
    }
    let dynamic = result
        .tool_records
        .iter()
        .find(|record| record.id == "dynamic")
        .unwrap();
    assert_eq!(
        dynamic.structured_content,
        Some(serde_json::json!({"answer":42}))
    );
    let bodies = server.captured_bodies();
    assert_eq!(bodies.len(), 2);
    let next: Value = serde_json::from_str(&bodies[1]).unwrap();
    let messages = next["messages"].as_array().unwrap();
    for (id, _, _) in &calls {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["tool_call_id"] == *id)
                .count(),
            1,
            "{id}"
        );
    }
    let response = |id: &str| {
        messages
            .iter()
            .find(|message| message["tool_call_id"] == id)
            .unwrap()["content"]
            .as_str()
            .unwrap()
    };
    assert_eq!(response("typed"), "count=7");
    assert!(response("dynamic").contains("structuredContent: {\"answer\":42}"));
    assert!(response("prepare").contains("Tool follow-up must be a user message"));
    assert!(response("timeout").contains("超时"));
}
