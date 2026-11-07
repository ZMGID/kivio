mod support;

use kivio_agent_sdk::{
    agent::{
        execute::ToolProgress, run_agent_loop, session::NoSessionPolicy, ToolExecutionContext,
    },
    model::ModelArtifact,
    tools::{
        StaticTool, ToolAuthorization, ToolDefinition, ToolError, ToolFuture, ToolOutput,
        ToolRegistry,
    },
};
use kivio_agent_sdk_consumer::{
    config, echo_definition, handle, initial_history, Control, Events, Host,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

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

struct Authorization(bool);
impl ToolAuthorization for Authorization {
    fn authorize<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            if self.0 {
                Ok(())
            } else {
                Err(ToolError::Execution("denied".into()))
            }
        })
    }
}

#[tokio::test]
async fn registered_static_tool_completes_http_roundtrip_without_a_custom_executor() {
    for (arguments, allowed, executions) in [
        (r#"{"text":"hello"}"#, true, 1),
        (r#"{"text":"hello"}"#, false, 0),
        (r#"{"text":7}"#, true, 0),
    ] {
        let server = support::Server::start(vec![
            support::tool("echo", arguments),
            support::answer("done"),
        ])
        .await;
        let provider = server.provider();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register_static(
                echo_definition(),
                Echo(calls.clone()),
                Arc::new(Authorization(allowed)),
            )
            .unwrap();
        let snapshot = registry.snapshot();
        let mut cfg = config(&provider, initial_history());
        cfg.tools = snapshot.definitions();
        let control = Control::default();
        let result = run_agent_loop(
            cfg,
            &Host,
            &snapshot,
            &mut NoSessionPolicy,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap();
        assert_eq!(result.content, "done");
        assert_eq!(calls.load(Ordering::SeqCst), executions);
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(server.count(), 2);
        let requests = server.requests.lock().unwrap();
        assert_eq!(
            requests[0].body["tools"][0]["function"]["parameters"]["properties"]["text"]["type"],
            "string"
        );
        let messages = requests[1].body["messages"].as_array().unwrap();
        assert_eq!(messages.last().unwrap()["role"], "tool");
        assert_eq!(messages.last().unwrap()["tool_call_id"], "call-42");
        if executions == 1 {
            assert_eq!(messages.last().unwrap()["content"], "hello");
        }
    }
}
