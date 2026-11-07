use super::*;
use crate::chat::agent::content::{AgentToolCall, AgentToolStatus};
use crate::chat::agent::events::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentRunOutcome, EventDelivery,
};
use crate::chat::agent::execute::ToolExecutorFuture;
use crate::chat::agent::loop_::tests::{
    test_app_state, test_run_config, MockModelServer, MockResponse, RecordingExecutor,
};
use crate::chat::session_policy::run_session;
use crate::chat::tool_runtime::{ToolDispatcher, ToolExecutionContext};

struct JournalHost {
    path: std::path::PathBuf,
    progress: Mutex<Progress>,
    journal: tokio::sync::Mutex<Journal>,
    outcomes: Mutex<Vec<AgentRunOutcome>>,
    incoming: Mutex<Vec<RuntimeMessage>>,
}

impl JournalHost {
    fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            progress: Mutex::default(),
            journal: tokio::sync::Mutex::default(),
            outcomes: Mutex::default(),
            incoming: Mutex::default(),
        }
    }
}

impl AgentEventSink for JournalHost {
    fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery {
        self.progress.lock().unwrap().apply_event(event);
        if let AgentEventKind::RunEnded { outcome } = &event.kind {
            self.outcomes.lock().unwrap().push(outcome.clone());
        }
        EventDelivery::Accepted
    }
}

impl AgentHost for JournalHost {
    fn requires_tool_completion(&self) -> bool {
        true
    }
    fn checkpoint_runtime<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        history: &'a kivio_agent_sdk::agent::SessionHistory,
        _: bool,
    ) -> AgentHostFuture<'a, Result<Vec<SessionInput>, String>> {
        Box::pin(async move {
            let records = self.progress.lock().unwrap().records.clone();
            self.journal
                .lock()
                .await
                .save_history(&self.path, history.context(), &records)
                .await?;
            let incoming = std::mem::take(&mut *self.incoming.lock().unwrap());
            if !incoming.is_empty() {
                let mut saved = history.context().to_vec();
                saved.extend(incoming.iter().cloned());
                self.journal
                    .lock()
                    .await
                    .save_history(&self.path, &saved, &records)
                    .await?;
            }
            Ok(incoming.into_iter().map(SessionInput::append).collect())
        })
    }
}

impl crate::chat::tool_runtime::ApplicationHost for JournalHost {
    fn request_tool_approval<'a>(
        &'a self,
        _: &'a ToolExecutionContext<'a>,
        _: &'a AgentToolCall,
    ) -> AgentHostFuture<'a, bool> {
        Box::pin(async { true })
    }

    fn request_session_consent<'a>(
        &'a self,
        _: &'a ToolExecutionContext<'a>,
    ) -> AgentHostFuture<'a, bool> {
        Box::pin(async { true })
    }

    fn request_user_response<'a>(
        &'a self,
        _: &'a ToolExecutionContext<'a>,
        _: &'a AgentToolCall,
        _: AskUserPromptPayload,
    ) -> AgentHostFuture<'a, AskUserResponseResult> {
        Box::pin(async {
            AskUserResponseResult {
                phase: "cancelled".into(),
                answers: HashMap::new(),
            }
        })
    }
}

fn record(dir: &std::path::Path) -> registry::AgentRecord {
    serde_json::from_value(json!({"id":"A","root":"r","parentId":"Main","generation":1,
        "displayName":"A","agentType":"task","status":"idle","depth":1,"childConversationId":"subagent-r-A",
        "runId":"first","sessionFile":dir.join("A.jsonl"),"outputFile":dir.join("A.md"),
        "lastActivity":0,"activity":null,"published":true})).unwrap()
}

fn identity() -> AgentRunIdentity<'static> {
    AgentRunIdentity {
        conversation_id: "conversation",
        run_id: "run",
        message_id: "message",
        generation: 1,
    }
}

fn fact(kind: AgentEventKind<'_>) -> AgentEvent<'_> {
    AgentEvent {
        identity: identity(),
        invocation_id: 1,
        sequence: 1,
        kind,
    }
}

fn provider() -> MockModelServer {
    MockModelServer::start(vec![
        MockResponse::Sse(vec![
            json!({"choices":[{"delta":{"reasoning_content":"thinking","content":"A😀中"}}]}).to_string(),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"read-1","function":{"name":"read","arguments":"{\"path\":\"/tmp/test.txt\"}"}}]},"finish_reason":"tool_calls"}]}).to_string(),
            "[DONE]".into(),
        ]),
        MockResponse::Sse(vec![json!({"choices":[{"delta":{"content":"完成😀"},"finish_reason":"stop"}]}).to_string(), "[DONE]".into()]),
    ])
}

async fn transcript(record: &registry::AgentRecord, cwd: &std::path::Path) {
    create_transcript(&record.session_file, &record.output_file)
        .await
        .unwrap();
    session::append_record(&record.session_file, &session::test_contract(cwd))
        .await
        .unwrap();
    session::append_record(
        &record.session_file,
        &json!({"type":"turn_start","jobId":"job","ownerId":"Main","runId":"first"}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn child_checkpoint_input_survives_final_history_with_parent_persistence_flag() {
    let dir = tempfile::tempdir().unwrap();
    let record = record(dir.path());
    transcript(&record, dir.path()).await;
    let host = JournalHost::new(record.session_file.clone());
    *host.incoming.lock().unwrap() = import_history(vec![json!({
        "role":"user", "content":"nested child report", "subagent_parent_persisted":true,
    })]);
    let server = MockModelServer::start(vec![MockResponse::Sse(vec![
        json!({"choices":[{"delta":{"content":"report received"},"finish_reason":"stop"}]})
            .to_string(),
        "[DONE]".into(),
    ])]);
    let state = test_app_state();
    let generation = state.chat_runtime().begin_generation("conversation");
    let control = crate::chat::runtime_state::ChatRunControl::new(
        state.chat_runtime(),
        "conversation",
        generation,
    );
    let config = test_run_config(&state, &server.base_url);
    let mut history = config.runtime_messages.clone();
    let result = run_session(
        config.into_application(),
        &host,
        &RecordingExecutor::default(),
        &host,
        AgentRunHandle::new(identity(), &control),
    )
    .await
    .unwrap();
    history.extend(result.api_messages);
    let reports: Vec<_> = history
        .iter()
        .filter(|message| message["content"] == "nested child report")
        .collect();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["subagent_parent_persisted"], true);
    host.journal
        .lock()
        .await
        .save_history(&record.session_file, &import_history(history), &[])
        .await
        .unwrap();
    let bytes = tokio::fs::read(&record.session_file).await.unwrap();
    let replayed = session::replay(&bytes, "r", "A").unwrap();
    assert_eq!(
        replayed
            .history
            .iter()
            .filter(|message| message["content"] == "nested child report")
            .count(),
        1
    );
}

#[tokio::test]
async fn real_loop_omp_projection_journal_replay_and_slot_release() {
    let dir = tempfile::tempdir().unwrap();
    let record = record(dir.path());
    let sessions = session::Sessions::default();
    let slot = sessions.slot(&record).unwrap();
    assert!(slot.begin(&record, "first"));
    let worker_guard = slot.worker.lock().await;
    transcript(&record, dir.path()).await;
    let host = JournalHost::new(record.session_file.clone());
    let server = provider();
    let state = test_app_state();
    let generation = state.chat_runtime().begin_generation("conversation");
    assert_eq!(generation, 1);
    let control = crate::chat::runtime_state::ChatRunControl::new(
        state.chat_runtime(),
        "conversation",
        generation,
    );
    let config = test_run_config(&state, &server.base_url);
    let initial_history = config.runtime_messages.clone();
    let executor = RecordingExecutor::default();
    let result = run_session(
        config.into_application(),
        &host,
        &executor,
        &host,
        AgentRunHandle::new(identity(), &control),
    )
    .await
    .unwrap();
    assert!(matches!(
        result.status,
        crate::chat::agent::types::AgentRunStatus::Completed
    ));
    let progress = host.progress.lock().unwrap();
    assert!(progress.text.contains("A😀中"));
    assert!(progress.text.contains("完成😀"));
    assert_eq!(progress.tool_count, 1);
    assert_eq!(progress.records.len(), 1);
    assert_eq!(
        progress.records[0].result_preview.as_deref(),
        Some("result:read")
    );
    assert_eq!(progress.recent_tools.len(), 1);
    drop(progress);
    let mut history = initial_history;
    history.extend(result.api_messages.clone());
    let history = import_history(history);
    let records = host.progress.lock().unwrap().records.clone();
    host.journal
        .lock()
        .await
        .save_history(&record.session_file, &history, &records)
        .await
        .unwrap();
    assert_eq!(
        *host.outcomes.lock().unwrap(),
        vec![AgentRunOutcome::Completed]
    );
    assert!(!slot.begin(&record, "second"));
    session::append_record(&record.session_file, &json!({"type":"turn_end","runId":"first","progress":{"status":"completed"},"model":"test-model"})).await.unwrap();
    let bytes = tokio::fs::read(&record.session_file).await.unwrap();
    let replayed = session::replay(&bytes, "r", "A").unwrap();
    assert!(replayed.interrupted.is_none());
    assert_eq!(replayed.last_progress.unwrap()["status"], "completed");
    let tool = replayed
        .history
        .iter()
        .find(|entry| entry["role"] == "tool")
        .unwrap();
    assert_eq!(tool["content"], "result:read");
    assert_eq!(tool["is_error"], false);
    assert!(replayed
        .history
        .iter()
        .any(|entry| entry["role"] == "assistant"
            && entry["content"]
                .as_str()
                .is_some_and(|text| text.contains("完成😀"))));
    drop(worker_guard);
    assert!(slot.finish(1, "first"));
    assert!(slot.begin(&record, "second"));
    assert!(!slot.finish(1, "first"));
    assert!(slot.owns(1, "second"));
    assert!(slot.finish(1, "second"));
    let tool_calls = executor
        .events()
        .iter()
        .filter(|event| event.starts_with("start:"))
        .count();
    let saved_tools = replayed
        .history
        .iter()
        .filter(|entry| entry["role"] == "tool")
        .count();
    assert_eq!(
        (server.captured_bodies().len(), tool_calls, saved_tools),
        (2, 1, 1)
    );
    eprintln!(
        "SDK_CONTRACT {}",
        json!({"host":"omp","requests":server.captured_bodies().len(),"tool_calls":tool_calls,"saved_tools":saved_tools,"released":!slot.owns(1,"second")})
    );
}

#[tokio::test]
async fn omp_journal_failure_is_sticky_and_preserves_history_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.jsonl");
    let history = import_history(vec![
        json!({"role":"system","content":"system"}),
        json!({"role":"user","content":"task"}),
    ]);
    let state = test_app_state();
    let generation = state.chat_runtime().begin_generation("conversation");
    let control = crate::chat::runtime_state::ChatRunControl::new(
        state.chat_runtime(),
        "conversation",
        generation,
    );
    let host = JournalHost::new(missing.clone());
    let config = test_run_config(&state, "http://127.0.0.1:1/v1");
    let result = run_session(
        config.into_application(),
        &host,
        &RecordingExecutor::default(),
        &host,
        AgentRunHandle::new(identity(), &control),
    )
    .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Cannot open subagent transcript"));
    assert!(matches!(
        host.outcomes.lock().unwrap().as_slice(),
        [AgentRunOutcome::Failed(_)]
    ));
    assert!(host.journal.lock().await.failed);
    let mut journal = Journal::default();
    assert!(journal.save_history(&missing, &history, &[]).await.is_err());
    assert!(journal.failed);
    assert!(journal.history.is_empty());
    tokio::fs::write(&missing, "").await.unwrap();
    assert!(journal
        .save_history(&missing, &history, &[])
        .await
        .unwrap_err()
        .contains("previously failed"));
    assert!(tokio::fs::read(&missing).await.unwrap().is_empty());
}

#[test]
fn omp_preview_and_rollback_preserve_utf8_and_completed_rounds() {
    let mut progress = Progress::default();
    progress.apply_event(&fact(AgentEventKind::Content {
        delta: &"😀中".repeat(3000),
        reasoning_delta: None,
        block: None,
    }));
    assert!(progress.text.len() <= 8192);
    assert!(progress.text.is_char_boundary(0));
    progress.apply_event(&fact(AgentEventKind::Content {
        delta: "keep😀中",
        reasoning_delta: None,
        block: None,
    }));
    let before = progress.text.clone();
    progress.apply_event(&fact(AgentEventKind::AttemptDiscarded {
        text_chars: 2,
        reasoning_chars: 0,
        block_ids: vec![],
        tool_ids: vec![],
    }));
    assert_eq!(
        progress.text,
        before
            .chars()
            .take(before.chars().count() - 2)
            .collect::<String>()
    );
    for round in 0..7 {
        let mut call = AgentToolCall {
            id: "reused".into(),
            name: "read".into(),
            source: "native".into(),
            server_id: None,
            arguments: "{}".into(),
            status: AgentToolStatus::Running,
            output: None,
            output_limit: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round,
            sensitive: false,
            artifacts: vec![],
            structured_content: None,
            draft: None,
        };
        progress.apply_event(&fact(AgentEventKind::ToolUpdated(&call)));
        progress.apply_event(&fact(AgentEventKind::ToolUpdated(&call)));
        call.status = AgentToolStatus::Success;
        call.output = Some(format!("result-{round}"));
        progress.apply_event(&fact(AgentEventKind::ToolUpdated(&call)));
        progress.apply_event(&fact(AgentEventKind::ToolUpdated(&call)));
    }
    assert_eq!(progress.tool_count, 7);
    assert_eq!(progress.records.len(), 7);
    assert_eq!(progress.recent_tools.len(), 5);
    progress.apply_event(&fact(AgentEventKind::AttemptDiscarded {
        text_chars: 0,
        reasoning_chars: 0,
        block_ids: vec![],
        tool_ids: vec!["reused".into()],
    }));
    assert_eq!(progress.records.len(), 7);
}

struct CleanupExecutor {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl ToolDispatcher for CleanupExecutor {
    fn call<'a>(
        &'a self,
        _progress: &'a crate::chat::agent::execute::ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        _: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            self.started.notify_one();
            self.release.notified().await;
            crate::chat::tool_output::from_mcp(
                tool,
                crate::mcp::types::McpToolCallResult {
                    content: "cleanup complete".into(),
                    is_error: false,
                    raw: Value::Null,
                    artifacts: vec![],
                    structured_content: None,
                    follow_up_user_messages: vec![],
                },
            )
        })
    }
}

#[tokio::test]
async fn stopped_managed_tool_keeps_slot_busy_until_cleanup_and_journal_finish() {
    let dir = tempfile::tempdir().unwrap();
    let record = record(dir.path());
    let slot = session::Sessions::default().slot(&record).unwrap();
    assert!(slot.begin(&record, "first"));
    let worker_guard = slot.worker.lock().await;
    transcript(&record, dir.path()).await;
    let host = JournalHost::new(record.session_file.clone());
    let server = provider();
    let state = test_app_state();
    let generation = state.chat_runtime().begin_generation("conversation");
    let control = crate::chat::runtime_state::ChatRunControl::new(
        state.chat_runtime(),
        "conversation",
        generation,
    );
    let handle = AgentRunHandle::new(identity(), &control);
    let executor = CleanupExecutor {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    };
    let config = test_run_config(&state, &server.base_url);
    let initial_history = config.runtime_messages.clone();
    let run = run_session(config.into_application(), &host, &executor, &host, handle.clone());
    tokio::pin!(run);
    tokio::select! {
        _ = executor.started.notified() => {},
        result = &mut run => panic!("loop ended before managed tool started: {result:?}"),
    }
    assert_eq!(handle.stop(), StopAdmission::Requested);
    assert_eq!(
        handle.steer(SteeringMessage::new("late".into(), "late").unwrap()),
        InputAdmission::Inactive
    );
    assert!(!slot.begin(&record, "second"));
    assert!(slot.worker.try_lock().is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(25), run.as_mut())
            .await
            .is_err()
    );
    assert!(host.outcomes.lock().unwrap().is_empty());
    executor.release.notify_one();
    let result = run.await.unwrap();
    assert!(matches!(
        result.status,
        crate::chat::agent::types::AgentRunStatus::Cancelled
    ));
    assert_eq!(
        *host.outcomes.lock().unwrap(),
        vec![AgentRunOutcome::Cancelled]
    );
    let records = host.progress.lock().unwrap().records.clone();
    assert!(records
        .iter()
        .any(|record| record.result_preview.as_deref() == Some("cleanup complete")));
    let mut history = initial_history;
    history.extend(result.api_messages);
    host.journal
        .lock()
        .await
        .save_history(&record.session_file, &import_history(history), &records)
        .await
        .unwrap();
    session::append_record(&record.session_file, &json!({"type":"turn_end","runId":"first","progress":{"status":"aborted"},"model":"test-model"})).await.unwrap();
    assert!(!slot.begin(&record, "second"));
    let replayed = session::replay(
        &tokio::fs::read(&record.session_file).await.unwrap(),
        "r",
        "A",
    )
    .unwrap();
    assert_eq!(replayed.last_progress.unwrap()["status"], "aborted");
    assert!(replayed
        .history
        .iter()
        .any(|entry| entry["role"] == "tool" && entry["content"] == "cleanup complete"));
    drop(worker_guard);
    assert!(slot.finish(1, "first"));
    assert!(slot.begin(&record, "second"));
    assert!(!slot.finish(1, "first"));
    assert!(slot.owns(1, "second"));
    assert_eq!(host.outcomes.lock().unwrap().len(), 1);
    assert!(slot.finish(1, "second"));
    let cleaned_tools = replayed
        .history
        .iter()
        .filter(|entry| entry["role"] == "tool" && entry["content"] == "cleanup complete")
        .count();
    assert_eq!((server.captured_bodies().len(), cleaned_tools), (1, 1));
    eprintln!(
        "SDK_CONTRACT {}",
        json!({"host":"omp-cancel","requests":server.captured_bodies().len(),"cleaned_saved_tools":cleaned_tools,"terminals":host.outcomes.lock().unwrap().len(),"released":!slot.owns(1,"second")})
    );
}
