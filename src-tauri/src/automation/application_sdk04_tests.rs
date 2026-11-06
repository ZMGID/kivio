use super::*;
use crate::chat::agent::control::AgentRunHandle;
use crate::chat::agent::events::{AgentEvent, AgentEventKind, AgentEventSink, AgentRunIdentity, EventDelivery};
use crate::chat::agent::loop_::tests::{test_app_state, test_run_config, MockModelServer, MockResponse, RecordingExecutor, TestHost};

fn event(kind: AgentEventKind<'_>) -> AgentEvent<'_> {
    AgentEvent {
        identity: AgentRunIdentity { conversation_id: "conversation", run_id: "run", message_id: "message", generation: 1 },
        invocation_id: 1, sequence: 1, kind,
    }
}

#[tokio::test]
async fn real_loop_workflow_projection_output_and_history_reload() {
    let server = MockModelServer::start(vec![
        MockResponse::Sse(vec![
            serde_json::json!({"choices":[{"delta":{"reasoning_content":"thinking"}}]}).to_string(),
            serde_json::json!({"choices":[{"delta":{"content":"A😀中"}}]}).to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"read-1","function":{"name":"read","arguments":"{\"path\":\"/tmp/test.txt\"}"}}]},"finish_reason":"tool_calls"}]}).to_string(),
            "[DONE]".into(),
        ]),
        MockResponse::Sse(vec![
            serde_json::json!({"choices":[{"delta":{"content":"完成😀"},"finish_reason":"stop"}]}).to_string(),
            "[DONE]".into(),
        ]),
    ]);
    let state = test_app_state();
    let config = test_run_config(&state, &server.base_url);
    let host = WorkflowAgentHost;
    let control = TestHost::default();
    let events = WorkflowAgentEvents::default();
    let tools = Mutex::new(Vec::new());
    let sink = |fact: &AgentEvent<'_>| {
        events.on_event(fact);
        if let AgentEventKind::ToolUpdated(call) = &fact.kind {
            if call.output.is_some() { tools.lock().unwrap().push(call.output.clone()); }
        }
        EventDelivery::Accepted
    };
    let handle = AgentRunHandle::new(event(AgentEventKind::RunStarted).identity, &control);
    let result = run_agent_loop(config, &host, &RecordingExecutor::default(), &sink, handle).await.unwrap();
    assert!(tools.lock().unwrap().iter().any(|output| output.as_deref() == Some("result:read")));
    assert!(events.text.lock().unwrap().contains("A😀中"));
    let mut fallback_result = result.clone();
    fallback_result.content = " ".into();
    assert_eq!(builtin_agent_node_output(Ok(fallback_result.clone()), "streamed😀".into()).unwrap().text, "streamed😀");
    assert_eq!(builtin_agent_node_output(Ok(fallback_result), String::new()).unwrap_err(), "Agent returned an empty response");
    assert_eq!(builtin_agent_node_output(Err("cancelled".into()), "ignored".into()).unwrap_err(), "cancelled");
    let mut output = builtin_agent_node_output(Ok(result), events.text.lock().unwrap().clone()).unwrap();
    assert!(output.text.contains("完成😀"));
    assert_eq!(output.json["text"], output.text);
    let input = NodeOutput::from_text("task");
    let node = super::super::runner::success_node_record("agent-node", "action.agent", false, &input, &mut output);
    let mut run = super::super::types::AutomationRun {
        id: uuid::Uuid::new_v4().to_string(), automation_id: uuid::Uuid::new_v4().to_string(),
        origin: "manual".into(), status: "running".into(), started_at: "2026-11-07T00:00:00Z".into(),
        finished_at: None, error: None, nodes: vec![node],
    };
    super::super::runner::finish_run_record(&mut run, "success", None);
    let dir = tempfile::tempdir().unwrap();
    super::super::history::write_run_in(dir.path(), &run).unwrap();
    let saved = super::super::history::read_run_file(&dir.path().join(format!("{}.json", run.id))).unwrap();
    assert_eq!(saved.status, "success");
    assert!(saved.finished_at.is_some());
    assert_eq!(saved.nodes[0].status, "success");
    let stored = saved.nodes[0].result.as_ref().unwrap();
    assert_eq!(stored.text, output.text);
    assert_eq!(stored.json, output.json);
    assert_eq!(stored.sources["agent-node"]["text"], output.text);
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "file").unwrap();
    assert!(super::super::history::write_run_in(&blocker, &run).is_err());
}

#[test]
fn workflow_attempt_discard_counts_unicode_scalars() {
    let events = WorkflowAgentEvents::default();
    events.on_event(&event(AgentEventKind::Content { delta: "kept😀中", reasoning_delta: None, block: None }));
    events.on_event(&event(AgentEventKind::AttemptDiscarded {
        text_chars: 2, reasoning_chars: 0, block_ids: vec![], tool_ids: vec![],
    }));
    assert_eq!(*events.text.lock().unwrap(), "kept");
}
