use super::{
    messages::{
        build_assistant_message, build_partial_assistant_snapshot, upsert_assistant_message,
    },
    reply_runtime::ChatReplyGuard,
};
use crate::chat::{
    agent::loop_::tests::{
        test_app_state, test_run_config, MockModelServer, MockResponse, RecordingExecutor,
    },
    agent::{
        self,
        content::{AgentToolCall, ContentBlock},
        control::{AgentRunHandle, InputAdmission, StopAdmission},
        events::{
            AgentEvent, AgentEventKind, AgentEventSink, AgentRunIdentity, AgentRunOutcome,
            EventDelivery,
        },
        history::RuntimeMessage,
        AgentHost, AgentHostFuture, ToolExecutorFuture,
    },
    agent_presentation,
    protocol::ChatRunEvent,
    runtime_state::ChatRunControl,
    session_policy::run_session,
    tool_runtime::{ToolDispatcher, ToolExecutionContext},
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

struct DesktopHost {
    journal: std::path::PathBuf,
    events: Mutex<Vec<ChatRunEvent>>,
    lifecycle: Mutex<Vec<(u64, &'static str)>>,
    outcomes: Mutex<Vec<AgentRunOutcome>>,
    approve: bool,
    closed: bool,
}
impl DesktopHost {
    fn new(journal: std::path::PathBuf) -> Self {
        Self {
            journal,
            events: Mutex::default(),
            lifecycle: Mutex::default(),
            outcomes: Mutex::default(),
            approve: true,
            closed: false,
        }
    }
}
impl AgentEventSink for DesktopHost {
    fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery {
        if self.closed {
            return EventDelivery::Closed;
        }
        let label = match &event.kind {
            AgentEventKind::RunStarted => "run-start",
            AgentEventKind::MessageEnded { .. } => "message-end",
            AgentEventKind::ToolStarted { .. } => "tool-start",
            AgentEventKind::ToolEnded { .. } => "tool-end",
            AgentEventKind::StepEnded { .. } => "step-end",
            AgentEventKind::RunEnded { outcome } => {
                self.outcomes.lock().unwrap().push(outcome.clone());
                "run-end"
            }
            _ => "fact",
        };
        self.lifecycle.lock().unwrap().push((event.sequence, label));
        self.events
            .lock()
            .unwrap()
            .extend(agent_presentation::project_event(event));
        EventDelivery::Accepted
    }
}
impl AgentHost for DesktopHost {
    fn persist_partial_assistant<'a>(
        &'a self,
        _: &'a str,
        message_id: &'a str,
        tools: &'a [AgentToolCall],
        blocks: &'a [ContentBlock],
        history: &'a [RuntimeMessage],
        _history: &'a kivio_agent_sdk::agent::SessionHistory,
    ) -> AgentHostFuture<'a, ()> {
        Box::pin(async move {
            let (tools, blocks) = agent_presentation::project_snapshot(tools, blocks, Some("run"));
            if let Some(draft) =
                build_partial_assistant_snapshot(message_id, &tools, &blocks, history)
            {
                crate::chat::draft_journal::append_draft_at_path(&self.journal, &draft).unwrap();
            }
        })
    }
}

impl crate::chat::tool_runtime::ApplicationHost for DesktopHost {
    fn request_tool_approval<'a>(
        &'a self,
        _: &'a ToolExecutionContext<'a>,
        _: &'a AgentToolCall,
    ) -> AgentHostFuture<'a, bool> {
        Box::pin(async { self.approve })
    }

    fn request_session_consent<'a>(
        &'a self,
        _: &'a ToolExecutionContext<'a>,
    ) -> AgentHostFuture<'a, bool> {
        Box::pin(async { self.approve })
    }

    fn request_user_response<'a>(
        &'a self,
        _: &'a ToolExecutionContext<'a>,
        _: &'a AgentToolCall,
        _: crate::chat::ask_user::AskUserPromptPayload,
    ) -> AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult> {
        Box::pin(async {
            crate::chat::ask_user::AskUserResponseResult {
                phase: "cancelled".into(),
                answers: Default::default(),
            }
        })
    }
}
#[derive(Default)]
struct CountingExecutor {
    inner: RecordingExecutor,
    calls: AtomicUsize,
}
impl ToolDispatcher for CountingExecutor {
    fn call<'a>(
        &'a self,
        progress: &'a crate::chat::agent::execute::ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a crate::mcp::ChatToolDefinition,
        arguments: serde_json::Value,
    ) -> ToolExecutorFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.call(progress, ctx, tool, arguments)
    }
}
fn tool_step() -> MockResponse {
    MockResponse::Sse(vec![
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"read-1","function":{"name":"read","arguments":"{\"path\":\"/tmp/sdk04.txt\"}"}}]}}]}).to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string(), "[DONE]".into(),
    ])
}
fn answer(text: &str) -> MockResponse {
    MockResponse::Sse(vec![
        json!({"choices":[{"delta":{"content":text}}]}).to_string(),
        "[DONE]".into(),
    ])
}
fn identity(generation: u64) -> AgentRunIdentity<'static> {
    AgentRunIdentity {
        conversation_id: "conversation",
        run_id: "run",
        message_id: "message",
        generation,
    }
}

#[tokio::test]
async fn sdk04_desktop_real_loop_projects_checkpoints_and_saved_history() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockModelServer::start(vec![
        tool_step(),
        MockResponse::SseInterrupt(vec![
            json!({"choices":[{"delta":{"content":"临时🙂残片\n"}}]}).to_string(),
        ]),
        answer("完成🙂"),
    ]);
    let state = test_app_state();
    let mut config = test_run_config(&state, &server.base_url);
    config.retry_attempts = 2;
    config.generation = state.chat_runtime().begin_generation("conversation");
    let reply = ChatReplyGuard::try_new(&state, "conversation", "run", config.generation).unwrap();
    let control = ChatRunControl::new(state.chat_runtime(), "conversation", config.generation);
    let handle = AgentRunHandle::new(identity(config.generation), &control);
    let host = DesktopHost::new(dir.path().join("draft.jsonl"));
    let executor = CountingExecutor::default();
    let result = run_session(config, &host, &executor, &host, handle.clone())
        .await
        .unwrap();
    assert_eq!(result.content, "完成🙂");
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        host.outcomes.lock().unwrap().as_slice(),
        &[AgentRunOutcome::Completed]
    );
    let lifecycle = host.lifecycle.lock().unwrap();
    assert!(lifecycle.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert_eq!(lifecycle.last().unwrap().1, "run-end");
    let at = |label| {
        lifecycle
            .iter()
            .position(|(_, event)| *event == label)
            .unwrap()
    };
    assert!(at("message-end") < at("tool-start"));
    assert!(at("tool-start") < at("tool-end") && at("tool-end") < at("step-end"));
    drop(lifecycle);
    assert_eq!(handle.stop(), StopAdmission::Inactive);
    assert_eq!(
        handle.steer(agent::SteeringMessage::new("late".into(), "late").unwrap()),
        InputAdmission::Inactive
    );
    assert!(state.chat_runtime().has_active_reply("conversation"));
    assert!(!state
        .chat_runtime()
        .try_reserve_send("conversation", "next"));

    let drafts = crate::chat::draft_journal::latest_drafts_per_message(
        &std::fs::read_to_string(&host.journal).unwrap(),
    );
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        drafts[0].tool_calls[0].result_preview.as_deref(),
        Some("result:read")
    );
    assert!(!drafts[0].model_messages.is_empty());
    let (tools, blocks) =
        agent_presentation::project_snapshot(&result.tool_records, &result.segments, Some("run"));
    let saved = build_assistant_message(
        "message".into(),
        result.content,
        result.reasoning,
        Vec::new(),
        tools,
        result.api_messages,
        blocks,
        None,
        Some("send"),
        Some(agent_presentation::run_status(&result.status)),
        result.usage,
        result.last_step_usage,
        None,
        None,
    );
    let mut conversation: crate::chat::Conversation = serde_json::from_value(json!({"id":"conversation","title":"SDK04","provider_id":"test","model":"test","messages":[],"created_at":0,"updated_at":0})).unwrap();
    upsert_assistant_message(&mut conversation, drafts[0].clone());
    upsert_assistant_message(&mut conversation, saved);
    let path = dir.path().join("conversation.json");
    crate::chat::storage::write_conversation_file_at_path(&path, conversation.clone()).unwrap();
    let loaded = crate::chat::storage::read_conversation_file(&path, "conversation").unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].content, "完成🙂");
    assert!(!loaded.messages[0].model_messages.is_empty());
    assert_eq!(
        loaded.messages[0].tool_calls[0].result_preview.as_deref(),
        Some("result:read")
    );
    let before_failed_save = std::fs::read(&path).unwrap();
    let blocked = dir.path().join("regular-file");
    std::fs::write(&blocked, "block").unwrap();
    assert!(crate::chat::storage::write_conversation_file_at_path(
        &blocked.join("failed.json"),
        conversation
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before_failed_save);
    let events = host.events.lock().unwrap();
    assert!(events.iter().any(|event| matches!(event, ChatRunEvent::StreamAttemptDiscarded { text_chars, .. } if *text_chars > 0)));
    assert!(events.iter().any(|event| matches!(event, ChatRunEvent::ToolUpdated { tool } if tool.id == "read-1" && tool.result_preview.as_deref() == Some("result:read"))));
    assert!(!events.iter().any(|event| matches!(
        event,
        ChatRunEvent::RunCompleted { .. }
            | ChatRunEvent::RunFailed { .. }
            | ChatRunEvent::RunCancelled { .. }
    )));
    let mut protocol = crate::chat::protocol::ChatProtocolHub::default();
    protocol
        .register("conversation", "run", "message", 0)
        .unwrap();
    for event in events.iter().cloned() {
        protocol.push("run", event).unwrap();
    }
    let live = protocol
        .running_snapshot("conversation", "run", "message")
        .unwrap();
    assert_eq!(live.content, loaded.messages[0].content);
    let expected_tools: Vec<_> = loaded.messages[0]
        .tool_calls
        .iter()
        .map(|tool| {
            crate::chat::protocol::ChatToolPayload::from_record(
                tool,
                agent::execute::truncate_chars(&tool.arguments, 800),
            )
        })
        .collect();
    assert_eq!(live.tools, expected_tools);
    let expected_blocks: Vec<_> = loaded.messages[0]
        .segments
        .iter()
        .map(crate::chat::protocol::ChatSegmentPayload::from)
        .collect();
    let mut normalized_live_blocks = live.segments.clone();
    for block in &mut normalized_live_blocks {
        if block.text.as_deref() == Some("") {
            block.text = None;
        }
    }
    assert_eq!(normalized_live_blocks, expected_blocks);

    drop(events);
    drop(reply);
    assert!(!state.chat_runtime().has_active_reply("conversation"));
    assert_eq!(server.captured_bodies().len(), 3);
    eprintln!(
        "SDK_CONTRACT {}",
        json!({"host":"desktop","requests":server.captured_bodies().len(),"tool_calls":executor.calls.load(Ordering::SeqCst),"saved_messages":loaded.messages.len(),"draft_messages":drafts.len(),"released":!state.chat_runtime().has_active_reply("conversation")})
    );
}

#[tokio::test]
async fn sdk04_closed_observation_does_not_authorize_a_tool() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockModelServer::start(vec![tool_step(), answer("declined")]);
    let state = test_app_state();
    let mut config = test_run_config(&state, &server.base_url);
    config.generation = state.chat_runtime().begin_generation("conversation");
    config.effective_chat_tools.approval_policy = "always_confirm".into();
    let control = ChatRunControl::new(state.chat_runtime(), "conversation", config.generation);
    let handle = AgentRunHandle::new(identity(config.generation), &control);
    let mut host = DesktopHost::new(dir.path().join("draft.jsonl"));
    host.approve = false;
    host.closed = true;
    let executor = CountingExecutor::default();
    let result = run_session(config, &host, &executor, &host, handle.clone())
        .await
        .unwrap();
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(result
        .tool_records
        .iter()
        .all(|call| call.status != agent::content::AgentToolStatus::Success));
    assert!(!handle.active());
    assert!(host.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sdk04_dropped_and_failed_real_loops_have_one_typed_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_app_state();
    let server = MockModelServer::start(vec![MockResponse::SseThenHang(Vec::new())]);
    let host = DesktopHost::new(dir.path().join("drop.jsonl"));
    let mut config = test_run_config(&state, &server.base_url);
    config.tools.clear();
    config.generation = state.chat_runtime().begin_generation("conversation");
    let control = ChatRunControl::new(state.chat_runtime(), "conversation", config.generation);
    let handle = AgentRunHandle::new(identity(config.generation), &control);
    let executor = CountingExecutor::default();
    let mut future = Box::pin(run_session(config, &host, &executor, &host, handle.clone()));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut future)
            .await
            .is_err()
    );
    drop(future);
    assert_eq!(
        host.outcomes.lock().unwrap().as_slice(),
        &[AgentRunOutcome::Dropped]
    );
    assert_eq!(host.lifecycle.lock().unwrap().last().unwrap().1, "run-end");
    assert!(!handle.active());

    let server = MockModelServer::start(vec![MockResponse::Status(
        400,
        "{\"error\":{\"message\":\"bad request\"}}".into(),
    )]);
    let host = DesktopHost::new(dir.path().join("failure.jsonl"));
    let mut config = test_run_config(&state, &server.base_url);
    config.tools.clear();
    config.generation = state.chat_runtime().begin_generation("conversation");
    let control = ChatRunControl::new(state.chat_runtime(), "conversation", config.generation);
    let handle = AgentRunHandle::new(identity(config.generation), &control);
    assert!(run_session(config, &host, &executor, &host, handle)
        .await
        .is_err());
    let outcomes = host.outcomes.lock().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(
        matches!(&outcomes[0], AgentRunOutcome::Failed(error) if error.to_string().contains("bad request"))
    );
}
