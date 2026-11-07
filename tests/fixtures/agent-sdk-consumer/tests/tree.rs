use std::{
    collections::VecDeque,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use kivio_agent_sdk::{
    agent::{
        content::AgentToolCall,
        control::StopAdmission,
        events::AgentRunIdentity,
        execute::{ToolFuture, ToolPreparation, ToolProgress},
        history::request_from_history,
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        session::CompactionPlan,
        types::{AgentRunResult, AgentRunStatus},
        AgentAdmissionError, AgentHost, AgentHostFuture, AgentRunError, AgentSession,
        AgentSessionConfig, EntryId, RuntimeMessage, SessionEntry, SessionHistory, SessionStatus,
        ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{
        gemini::{request_body, GeminiOptions},
        GenerateOutput, MessagePart, ModelError, ModelMessage, ModelRole, PendingToolCall,
        ProviderSignature,
    },
    tools::{ToolDefinition, ToolOutput},
};
use kivio_agent_sdk_consumer::{echo_definition, EchoTool};
use serde_json::{json, Value};
use tokio::sync::Notify;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("session did not settle")
}

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}
impl Gate {
    async fn hold(&self) {
        self.entered.notify_one();
        self.release.notified().await;
    }
}

struct Model {
    outputs: Mutex<VecDeque<GenerateOutput>>,
    calls: Mutex<Vec<ModelCall>>,
}
impl Model {
    fn new(outputs: Vec<GenerateOutput>) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(outputs.into()),
            calls: Mutex::new(Vec::new()),
        })
    }
}
impl ModelRuntime for Model {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            sink.request_started(&call);
            self.calls.lock().unwrap().push(call);
            Ok(self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request"))
        })
    }
}

fn answer(text: &str) -> GenerateOutput {
    GenerateOutput::text(text.into(), None)
}

fn tool_call() -> GenerateOutput {
    let mut output = answer("");
    output.tool_calls.push(PendingToolCall {
        id: "old-call".into(),
        function_name: "echo".into(),
        arguments: json!({"text":"old result"}),
        arguments_raw: r#"{"text":"old result"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    output
}

fn picture(text: &str, path: &str) -> RuntimeMessage {
    RuntimeMessage::model(ModelMessage {
        role: ModelRole::User,
        content: vec![
            MessagePart::Text { text: text.into() },
            MessagePart::Image {
                mime_type: "image/png".into(),
                data: "aW1hZ2U=".into(),
                path: Some(path.into()),
                detail: None,
            },
        ],
    })
}

fn texts(messages: &[RuntimeMessage]) -> Vec<String> {
    messages.iter().map(RuntimeMessage::text_content).collect()
}

fn admission(error: AgentRunError, expected: AgentAdmissionError) {
    assert!(matches!(error, AgentRunError::Admission(actual) if actual == expected));
}

fn assert_facts_preserved(entries: &[SessionEntry], original: &[SessionEntry]) {
    assert_eq!(&entries[..original.len()], original);
}

#[tokio::test]
async fn interleaved_branches_continue_with_their_own_history_without_reexecuting_tools() {
    let model = Model::new(vec![
        tool_call(),
        answer("shared answer"),
        answer("answer A"),
        answer("answer B"),
        answer("answer A2"),
        answer("answer B2"),
        answer("resumed A"),
    ]);
    let echo = Arc::new(EchoTool::new(true));
    let mut config = AgentSessionConfig::new(model.clone(), "test");
    config
        .tools
        .register_dynamic(echo_definition(), echo.clone())
        .unwrap();
    let session = AgentSession::new(config);
    bounded(session.prompt("shared question")).await.unwrap();
    let shared = session.active_leaf().unwrap();
    let shared_facts = session.entries();
    bounded(session.prompt("branch A")).await.unwrap();
    let a = session.active_leaf().unwrap();
    let a_facts = session.entries();
    let a_question = a_facts[a_facts.len() - 2].id;
    session.fork(shared).unwrap();
    bounded(session.prompt("branch B")).await.unwrap();
    let b = session.active_leaf().unwrap();
    assert!(session.leaves().contains(&a));
    assert!(session.leaves().contains(&b));
    session.switch_branch(Some(a)).unwrap();
    bounded(session.prompt("A continued")).await.unwrap();
    let a_continued = session.active_leaf().unwrap();
    session.switch_branch(Some(b)).unwrap();
    bounded(session.prompt("B continued")).await.unwrap();
    session.fork(a_question).unwrap();
    bounded(session.continue_run()).await.unwrap();

    assert_eq!(echo.call_count(), 1);
    assert_facts_preserved(&session.entries(), &shared_facts);
    assert_facts_preserved(&session.entries(), &a_facts);
    assert!(session.leaves().contains(&a_continued));
    let calls = model.calls.lock().unwrap();
    let shared = ["shared question", "", "old result", "shared answer"];
    assert_eq!(
        texts(&calls[2].history),
        [shared.as_slice(), &["branch A"]].concat()
    );
    assert_eq!(
        texts(&calls[3].history),
        [shared.as_slice(), &["branch B"]].concat()
    );
    assert_eq!(
        texts(&calls[4].history),
        [shared.as_slice(), &["branch A", "answer A", "A continued"]].concat()
    );
    assert_eq!(
        texts(&calls[5].history),
        [shared.as_slice(), &["branch B", "answer B", "B continued"]].concat()
    );
    assert_eq!(
        texts(&calls[6].history),
        [shared.as_slice(), &["branch A"]].concat()
    );
}

#[tokio::test]
async fn rewind_and_regeneration_keep_original_answers_and_attachment_inputs() {
    let model = Model::new(vec![
        answer("original"),
        answer("user retry"),
        answer("assistant retry"),
        answer("edited retry"),
        answer("rewound retry"),
    ]);
    let session = AgentSession::new(AgentSessionConfig::new(model.clone(), "test"));
    let original_input = picture("question", "original.png");
    bounded(session.prompt(original_input.clone()))
        .await
        .unwrap();
    let original_facts = session.entries();
    let question = original_facts[0].id;
    let original_answer = session.active_leaf().unwrap();
    bounded(session.regenerate(question, None)).await.unwrap();
    let user_retry = session.active_leaf().unwrap();
    bounded(session.regenerate(original_answer, None))
        .await
        .unwrap();
    let assistant_retry = session.active_leaf().unwrap();
    let edited = picture("edited question", "edited.png");
    bounded(session.regenerate(question, Some(edited.clone())))
        .await
        .unwrap();
    let edited_retry = session.active_leaf().unwrap();
    assert_eq!(session.rewind(question).unwrap(), original_input);
    assert!(session.context().is_empty());
    bounded(session.prompt(original_input.clone()))
        .await
        .unwrap();

    for (leaf, expected) in [
        (original_answer, ["question", "original"]),
        (user_retry, ["question", "user retry"]),
        (assistant_retry, ["question", "assistant retry"]),
        (edited_retry, ["edited question", "edited retry"]),
    ] {
        assert!(session.leaves().contains(&leaf));
        session.switch_branch(Some(leaf)).unwrap();
        assert_eq!(texts(&session.context()), expected);
    }
    assert_facts_preserved(&session.entries(), &original_facts);
    let calls = model.calls.lock().unwrap();
    for call in [&calls[0], &calls[1], &calls[2], &calls[4]] {
        assert_eq!(call.history, [original_input.clone()]);
    }
    assert_eq!(calls[3].history, [edited]);
    let restored = SessionHistory::from_entries(session.entries(), session.active_leaf()).unwrap();
    let references: Vec<_> = restored
        .attachment_references()
        .iter()
        .map(|item| item.reference)
        .collect();
    assert_eq!(
        references
            .iter()
            .filter(|&&path| path == "original.png")
            .count(),
        2
    );
    assert!(references.contains(&"edited.png"));
}

#[tokio::test]
async fn branches_across_compaction_and_reset_project_the_right_model_context() {
    let mut history = SessionHistory::from(vec![
        picture("old question", "old.png"),
        RuntimeMessage::text(ModelRole::Assistant, "old answer"),
        RuntimeMessage::from("recent"),
    ]);
    let original = history.active_leaf().unwrap();
    let old_entry = history.entries()[0].id;
    let original_facts = history.entries().to_vec();
    history
        .compact(CompactionPlan {
            covered: 0..2,
            replacement: vec![picture("summary", "summary.png")],
        })
        .unwrap();
    let compacted = history.active_leaf().unwrap();
    history
        .reset_context(vec![picture("reset", "reset.png")])
        .unwrap();
    let reset = history.active_leaf().unwrap();
    history.fork(original).unwrap();
    let mut sibling_input = picture("sibling", "sibling.png");
    sibling_input
        .source
        .insert("attachment".into(), json!("ignored-source.png"));
    let parts = &mut sibling_input.model_message_mut().unwrap().content;
    parts.push(MessagePart::Video {
        mime_type: "video/mp4".into(),
        data: String::new(),
        path: Some("sibling.mp4".into()),
    });
    parts.push(MessagePart::ImageUrl {
        url: "https://example.test/image.png".into(),
        detail: None,
    });
    history.push(sibling_input);
    let sibling_input_id = history.active_leaf().unwrap();
    history.push(RuntimeMessage::from_legacy(json!({
        "role":"assistant", "content":"",
        "tool_calls":[{"id":"artifact", "type":"function", "function":{"name":"echo", "arguments":"{}"}}]
    })).unwrap());
    history.push(RuntimeMessage::from_legacy(json!({
        "role":"tool", "tool_call_id":"artifact", "content":"file result",
        "artifacts":[{"name":"file", "mimeType":"text/plain", "dataUrl":"", "sizeBytes":4, "path":"file.txt"}]
    })).unwrap());
    let artifact = history.active_leaf().unwrap();
    history.push(RuntimeMessage::text(ModelRole::Assistant, "sibling answer"));
    let sibling = history.active_leaf().unwrap();
    let refs: Vec<_> = history
        .attachment_references()
        .iter()
        .map(|r| (r.entry_id, r.reference))
        .collect();
    assert_eq!(
        refs,
        [
            (old_entry, "old.png"),
            (compacted, "summary.png"),
            (reset, "reset.png"),
            (sibling_input_id, "sibling.png"),
            (sibling_input_id, "sibling.mp4"),
            (sibling_input_id, "https://example.test/image.png"),
            (artifact, "file.txt")
        ]
    );
    assert_facts_preserved(history.entries(), &original_facts);

    let model = Model::new(vec![
        answer("original reply"),
        answer("summary reply"),
        answer("reset reply"),
        answer("sibling reply"),
    ]);
    let mut config = AgentSessionConfig::new(model.clone(), "test");
    config.history = history;
    let session = AgentSession::new(config);
    for leaf in [original, compacted, reset, sibling] {
        session.switch_branch(Some(leaf)).unwrap();
        bounded(session.prompt("next")).await.unwrap();
    }
    let calls = model.calls.lock().unwrap();
    assert_eq!(
        texts(&calls[0].history),
        ["old question", "old answer", "recent", "next"]
    );
    assert_eq!(texts(&calls[1].history), ["summary", "recent", "next"]);
    assert_eq!(texts(&calls[2].history), ["reset", "next"]);
    assert_eq!(
        texts(&calls[3].history),
        [
            "old question",
            "old answer",
            "recent",
            "sibling",
            "",
            "file result",
            "sibling answer",
            "next"
        ]
    );
    assert_facts_preserved(&session.entries(), &original_facts);
}

#[tokio::test]
async fn native_signature_replays_only_for_its_source_on_the_selected_branch() {
    let mut call = RuntimeMessage::from_legacy(json!({
        "role":"assistant", "content":"", "_ui_secret":"HOST_ONLY",
        "tool_calls":[{"id":"signed", "type":"function", "function":{"name":"echo", "arguments":"{}"}}]
    })).unwrap();
    let part = call
        .model_message_mut()
        .unwrap()
        .content
        .iter_mut()
        .find(|part| matches!(part, MessagePart::ToolCall { .. }))
        .unwrap();
    if let MessagePart::ToolCall { signature, .. } = part {
        *signature = Some(ProviderSignature::Scoped {
            value: "NATIVE_SIGNATURE".into(),
            model: "source-model".into(),
            provider_id: Some("source-provider".into()),
        });
    }
    let mut history = SessionHistory::from(vec![RuntimeMessage::from("shared")]);
    let root = history.active_leaf().unwrap();
    history.extend([
        call,
        RuntimeMessage::from_legacy(
            json!({"role":"tool", "tool_call_id":"signed", "content":"tool output"}),
        )
        .unwrap(),
        RuntimeMessage::text(ModelRole::Assistant, "signed answer"),
    ]);
    let signed_leaf = history.active_leaf().unwrap();
    history.fork(root).unwrap();
    history.extend([
        RuntimeMessage::from("sibling"),
        RuntimeMessage::text(ModelRole::Assistant, "sibling answer"),
    ]);
    let sibling = history.active_leaf().unwrap();
    let facts = history.entries().to_vec();

    for (model_name, provider_id, expected) in [
        ("source-model", "source-provider", true),
        ("other-model", "source-provider", false),
        ("source-model", "other-provider", false),
    ] {
        let model = Model::new(vec![answer("signed response"), answer("sibling response")]);
        let mut config = AgentSessionConfig::new(model.clone(), model_name);
        config.provider_id = provider_id.into();
        config.history = history.clone();
        let session = AgentSession::new(config);
        session.switch_branch(Some(signed_leaf)).unwrap();
        bounded(session.prompt("continue signed")).await.unwrap();
        session.switch_branch(Some(sibling)).unwrap();
        bounded(session.prompt("continue sibling")).await.unwrap();
        assert_facts_preserved(&session.entries(), &facts);

        let calls = model.calls.lock().unwrap();
        let encode = |call: &ModelCall| {
            let request = request_from_history(
                &call.history,
                &call.model,
                call.tools.clone(),
                call.options.clone(),
                call.metadata.clone(),
            );
            request_body(
                &request,
                &GeminiOptions {
                    provider_id: Some(provider_id.into()),
                    include_tool_ids: true,
                    ..Default::default()
                },
            )
        };
        let signed = encode(&calls[0]);
        let part = signed["contents"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["parts"].as_array().unwrap())
            .find(|part| part["functionCall"]["id"] == "signed")
            .unwrap();
        assert_eq!(part.get("thoughtSignature").is_some(), expected);
        if expected {
            assert_eq!(part["thoughtSignature"], "NATIVE_SIGNATURE");
        }
        assert!(!signed.to_string().contains("HOST_ONLY"));
        let sibling_body = encode(&calls[1]).to_string();
        assert!(!sibling_body.contains("NATIVE_SIGNATURE"));
        assert!(!sibling_body.contains("tool output"));
        assert!(sibling_body.contains("sibling answer"));
    }
}

#[tokio::test]
async fn invalid_navigation_and_regeneration_preserve_facts_and_active_path() {
    let call = RuntimeMessage::from_legacy(json!({
        "role":"assistant", "content":"",
        "tool_calls":[{"id":"call", "type":"function", "function":{"name":"echo", "arguments":"{}"}}]
    })).unwrap();
    let history = SessionHistory::from(vec![
        RuntimeMessage::from("question"),
        call,
        RuntimeMessage::from_legacy(
            json!({"role":"tool", "tool_call_id":"call", "content":"result"}),
        )
        .unwrap(),
        RuntimeMessage::text(ModelRole::Assistant, "answer"),
        RuntimeMessage::text(ModelRole::Assistant, "second answer"),
    ]);
    let call_id = history.entries()[1].id;
    let answer_id = history.entries()[4].id;
    let question_id = history.entries()[0].id;
    let model = Model::new(vec![]);
    let mut config = AgentSessionConfig::new(model.clone(), "test");
    config.history = history;
    let session = AgentSession::new(config);
    let entries = session.entries();
    let context = session.context();
    let leaf = session.active_leaf();
    for error in [
        session.switch_branch(Some(EntryId(u64::MAX))),
        session.fork(call_id),
        session.switch_branch(Some(call_id)),
    ] {
        assert!(matches!(error, Err(AgentAdmissionError::InvalidHistory(_))));
    }
    assert!(matches!(
        session.rewind(answer_id),
        Err(AgentAdmissionError::InvalidHistory(_))
    ));
    assert!(matches!(
        bounded(session.regenerate(EntryId(u64::MAX), None)).await,
        Err(AgentRunError::Admission(
            AgentAdmissionError::InvalidHistory(_)
        ))
    ));
    for invalid in [
        RuntimeMessage::from(" "),
        RuntimeMessage::text(ModelRole::Assistant, "wrong role"),
    ] {
        admission(
            bounded(session.regenerate(question_id, Some(invalid)))
                .await
                .unwrap_err(),
            AgentAdmissionError::EmptyInput,
        );
    }
    admission(
        bounded(session.regenerate(answer_id, None))
            .await
            .unwrap_err(),
        AgentAdmissionError::InvalidContinuation,
    );
    assert_eq!(session.entries(), entries);
    assert_eq!(session.context(), context);
    assert_eq!(session.active_leaf(), leaf);
    assert!(model.calls.lock().unwrap().is_empty());
}

#[test]
fn regeneration_without_a_runtime_preserves_the_selected_branch() {
    let history = SessionHistory::from(vec![
        RuntimeMessage::from("question"),
        RuntimeMessage::text(ModelRole::Assistant, "answer"),
    ]);
    let question = history.entries()[0].id;
    let mut config = AgentSessionConfig::new(Model::new(vec![]), "test");
    config.history = history;
    let session = AgentSession::new(config);
    let entries = session.entries();
    let context = session.context();
    let leaf = session.active_leaf();
    let run = session.regenerate(question, Some(RuntimeMessage::from("edit")));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    admission(
        runtime.block_on(run).unwrap_err(),
        AgentAdmissionError::NoRuntime,
    );
    assert_eq!(session.entries(), entries);
    assert_eq!(session.context(), context);
    assert_eq!(session.active_leaf(), leaf);
}

#[test]
fn imported_inactive_unfinished_tool_round_cannot_be_selected() {
    let mut history = SessionHistory::from(vec![
        RuntimeMessage::from("question"),
        RuntimeMessage::text(ModelRole::Assistant, "complete"),
    ]);
    let complete = history.active_leaf().unwrap();
    let root = history.entries()[0].id;
    history.fork(root).unwrap();
    history.push(RuntimeMessage::from_legacy(json!({
        "role":"assistant", "content":"",
        "tool_calls":[{"id":"pending", "type":"function", "function":{"name":"echo", "arguments":"{}"}}]
    })).unwrap());
    let pending = history.active_leaf().unwrap();
    let entries = history.entries().to_vec();
    assert!(SessionHistory::from_entries(entries.clone(), Some(pending)).is_err());
    let mut restored = SessionHistory::from_entries(entries.clone(), Some(complete)).unwrap();
    let context = restored.context().to_vec();
    assert!(restored.leaves().contains(&pending));
    assert!(restored.switch_branch(Some(pending)).is_err());
    assert_eq!(restored.entries(), entries);
    assert_eq!(restored.context(), context);
    assert_eq!(restored.active_leaf(), Some(complete));
}

struct ManagedTool {
    delegate: EchoTool,
    gate: Arc<Gate>,
}
impl ToolExecutor for ManagedTool {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            let mut prepared = self
                .delegate
                .before_tool_call(progress, ctx, tool, call, record)
                .await?;
            if let ToolPreparation::Ready(ready) = &mut prepared {
                ready.drain_on_cancel = true;
                ready.timeout_ms = None;
            }
            Ok(prepared)
        })
    }
    fn call<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            self.gate.hold().await;
            Ok(ToolOutput::text("drained"))
        })
    }
}

struct CommitHost(Arc<Gate>);
impl AgentHost for CommitHost {
    fn commit_run<'a>(
        &'a self,
        _: AgentRunIdentity<'a>,
        _: &'a AgentRunResult,
    ) -> AgentHostFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.0.hold().await;
            Ok(())
        })
    }
}

async fn assert_navigation_busy(session: &AgentSession, anchor: EntryId) {
    let entries = session.entries();
    let context = session.context();
    let leaf = session.active_leaf();
    assert_eq!(
        session.switch_branch(Some(anchor)),
        Err(AgentAdmissionError::Busy)
    );
    assert_eq!(session.fork(anchor), Err(AgentAdmissionError::Busy));
    assert_eq!(session.rewind(anchor), Err(AgentAdmissionError::Busy));
    admission(
        bounded(session.regenerate(anchor, None)).await.unwrap_err(),
        AgentAdmissionError::Busy,
    );
    assert_eq!(session.entries(), entries);
    assert_eq!(session.context(), context);
    assert_eq!(session.active_leaf(), leaf);
}

#[tokio::test]
async fn navigation_stays_busy_through_stop_tool_cleanup_and_blocked_commit() {
    let tool_gate = Arc::new(Gate::default());
    let commit_gate = Arc::new(Gate::default());
    let model = Model::new(vec![tool_call()]);
    let history = SessionHistory::from(vec![
        RuntimeMessage::from("original question"),
        RuntimeMessage::text(ModelRole::Assistant, "original answer"),
    ]);
    let anchor = history.entries()[0].id;
    let original_leaf = history.active_leaf().unwrap();
    let original = history.entries().to_vec();
    let mut config = AgentSessionConfig::new(model, "test");
    config.history = history;
    config.host = Arc::new(CommitHost(commit_gate.clone()));
    config
        .tools
        .register_dynamic(
            echo_definition(),
            Arc::new(ManagedTool {
                delegate: EchoTool::new(true),
                gate: tool_gate.clone(),
            }),
        )
        .unwrap();
    let session = AgentSession::new(config);
    let run = session.prompt("active question");
    bounded(tool_gate.entered.notified()).await;
    assert_eq!(session.status(), SessionStatus::Running);
    assert_navigation_busy(&session, anchor).await;
    assert_eq!(session.abort(), StopAdmission::Requested);
    assert_eq!(session.status(), SessionStatus::StopRequested);
    assert_navigation_busy(&session, anchor).await;
    tool_gate.release.notify_one();
    bounded(commit_gate.entered.notified()).await;
    assert_eq!(session.status(), SessionStatus::Finishing);
    assert_navigation_busy(&session, anchor).await;
    let mut inactive = Box::pin(session.wait_inactive());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut inactive)
            .await
            .is_err()
    );
    commit_gate.release.notify_one();
    assert_eq!(
        bounded(run).await.unwrap().status,
        AgentRunStatus::Cancelled
    );
    bounded(inactive).await;
    assert_eq!(session.status(), SessionStatus::Idle);
    assert_facts_preserved(&session.entries(), &original);
    assert!(texts(&session.context()).contains(&"active question".to_owned()));
    assert!(texts(&session.context()).contains(&"drained".to_owned()));
    session.switch_branch(Some(original_leaf)).unwrap();
    assert_eq!(
        texts(&session.context()),
        ["original question", "original answer"]
    );
}
