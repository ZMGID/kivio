mod support;

use std::{
    collections::VecDeque,
    fs,
    future::Future,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kivio_agent_sdk::{
    agent::{
        control::{AgentRunHandle, InputAdmission, StopAdmission},
        host::{InputPersistence, SessionInput},
        provider_runtime::{
            ModelCall, ModelDelivery, ModelOutputSink, ModelRuntime, ProviderFuture,
        },
        session::{
            ContextPreparation, ContextReason, ContextSnapshot, PolicyFuture, PolicyProgress,
            SessionPolicy,
        },
        types::AgentRunStatus,
        AgentHost, AgentHostFuture, AgentRunError, AgentSession, AgentSessionConfig,
        RuntimeMessage, SessionEntry, SessionEntryKind, SessionHistory, SteeringMessage,
    },
    context::{BudgetContextPolicy, SummaryOptions},
    model::{
        gemini::{GeminiConfig, GeminiProvider},
        FinishReason, GenerateOutput, MessagePart, ModelError, ModelRole, PendingToolCall,
        ProviderSignature,
    },
    skills::{SkillAccess, SkillRegistry, SkillRunContext, SkillSource, SkillTool},
    tools::{ToolAuthorization, ToolDefinition, ToolError, ToolFuture},
};
use kivio_agent_sdk_consumer::{echo_definition, EchoTool};
use serde_json::{json, Value};
use tokio::sync::Notify;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("session did not settle")
}

enum Summary {
    Output(GenerateOutput),
    Error,
    Wait,
}

struct Model {
    summaries: Mutex<VecDeque<Summary>>,
    answers: Mutex<VecDeque<GenerateOutput>>,
    calls: Mutex<Vec<ModelCall>>,
    summary_started: Notify,
}

impl Model {
    fn new(summaries: Vec<Summary>, answers: Vec<GenerateOutput>) -> Arc<Self> {
        Arc::new(Self {
            summaries: Mutex::new(summaries.into()),
            answers: Mutex::new(answers.into()),
            calls: Mutex::new(Vec::new()),
            summary_started: Notify::new(),
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
            let summary = call.delivery == ModelDelivery::Collected;
            self.calls.lock().unwrap().push(call);
            if summary {
                let step = self
                    .summaries
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected summary request");
                match step {
                    Summary::Output(output) => Ok(output),
                    Summary::Error => Err(ModelError::new("summary provider failed")),
                    Summary::Wait => {
                        self.summary_started.notify_one();
                        std::future::pending().await
                    }
                }
            } else {
                Ok(self
                    .answers
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected model request"))
            }
        })
    }
}

fn text(value: &str) -> GenerateOutput {
    GenerateOutput::text(value.into(), None)
}

fn policy(model: Arc<dyn ModelRuntime>) -> BudgetContextPolicy {
    let mut options = SummaryOptions::new("fixture", "summary-model", 8_000);
    options.options.max_tokens = 1_024;
    let mut policy = BudgetContextPolicy::new(model, options);
    policy.threshold = Some(3_000);
    policy.max_output_tokens = 1_024;
    policy
}

struct SkillFile {
    root: PathBuf,
    path: PathBuf,
}

impl SkillFile {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "kivio-context-skill-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let directory = root.join("demo");
        fs::create_dir_all(&directory).unwrap();
        Self {
            path: directory.join("SKILL.md"),
            root,
        }
    }

    fn write(&self, body: &str) {
        fs::write(
            &self.path,
            format!("---\nname: demo\ndescription: demo instructions\n---\n{body}\n"),
        )
        .unwrap();
    }
}

impl Drop for SkillFile {
    fn drop(&mut self) {
        fs::remove_file(&self.path).unwrap();
        fs::remove_dir(self.path.parent().unwrap()).unwrap();
        fs::remove_dir(&self.root).unwrap();
    }
}

struct AllowSkill;

impl ToolAuthorization for AllowSkill {
    fn authorize<'a>(
        &'a self,
        _: &'a kivio_agent_sdk::agent::execute::ToolProgress<'a>,
        _: &'a kivio_agent_sdk::agent::ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>> {
        Box::pin(async { Ok(()) })
    }
}

fn skill_call(id: &str) -> GenerateOutput {
    let mut output = text("");
    output.tool_calls.push(PendingToolCall {
        id: id.into(),
        function_name: SkillTool::definition().model.name,
        arguments: json!({"name":"demo"}),
        arguments_raw: r#"{"name":"demo"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    output
}

#[tokio::test]
async fn compacted_skill_instructions_are_reloaded_through_the_skill_tool() {
    let file = SkillFile::new();
    file.write("FIRST_SKILL_BODY");
    let registry = Arc::new(SkillRegistry::discover(&[SkillSource {
        path: file.root.clone(),
        source: "fixture".into(),
    }]));
    let context = SkillRunContext::new(registry, SkillAccess::default());
    let model = Model::new(
        vec![Summary::Output(text("skill was used earlier"))],
        vec![
            skill_call("skill-first"),
            text(&"x".repeat(9_000)),
            text(&"y".repeat(9_000)),
            skill_call("skill-second"),
            text("done"),
        ],
    );
    let mut config = AgentSessionConfig::new(model.clone(), "test-model");
    config
        .tools
        .register_static(
            SkillTool::definition(),
            SkillTool::new(context),
            Arc::new(AllowSkill),
        )
        .unwrap();
    let session = AgentSession::new(config);
    bounded(session.prompt("load the skill")).await.unwrap();
    bounded(session.prompt("continue work")).await.unwrap();
    let before = session.entries();
    assert!(before.iter().any(|entry| matches!(&entry.kind, SessionEntryKind::Message(message) if message.text_content().contains("FIRST_SKILL_BODY"))));
    file.write("SECOND_SKILL_BODY");
    session.set_policy(policy(model.clone())).unwrap();
    assert_eq!(
        bounded(session.prompt("use the skill again"))
            .await
            .unwrap()
            .content,
        "done"
    );
    assert_original_entries(&session.entries(), &before);
    assert_eq!(count_compactions(&session.entries()), 1);

    let calls = model.calls.lock().unwrap();
    let summary = calls
        .iter()
        .position(|call| call.delivery == ModelDelivery::Collected)
        .unwrap();
    let after_summary = texts(&calls[summary + 1].history).join("\n");
    assert!(after_summary.contains("demo"));
    assert!(after_summary.contains("load it again"));
    assert!(!after_summary.contains("FIRST_SKILL_BODY"));
    let after_tool = texts(&calls[summary + 2].history).join("\n");
    assert!(after_tool.contains("SECOND_SKILL_BODY"));
    assert!(!after_tool.contains("FIRST_SKILL_BODY"));
}

#[tokio::test]
async fn provider_receives_projected_history_without_host_metadata() {
    let server = support::Server::start(vec![
        support::answer("wire summary"),
        support::answer("wire answer"),
    ])
    .await;
    let provider = Arc::new(server.provider());
    let mut config =
        AgentSessionConfig::new(provider.clone(), "test-model").with_policy(policy(provider));
    config.history = history().into();
    let session = AgentSession::new(config);
    let original = session.entries();
    assert_eq!(
        bounded(session.prompt("continue")).await.unwrap().content,
        "wire answer"
    );
    assert_original_entries(&session.entries(), &original);
    assert_eq!(count_compactions(&session.entries()), 1);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let sent = requests[1].body.to_string();
    assert!(sent.contains("wire summary"));
    assert!(sent.contains("result one"));
    assert!(sent.contains("result two"));
    assert!(!sent.contains("ARCHIVED_A"));
    assert!(!sent.contains("HOST_METADATA_SECRET"));
    assert!(!sent.contains("native-cipher"));
    let messages = requests[1].body["messages"].as_array().unwrap();
    let call = messages
        .iter()
        .find(|message| message["tool_calls"].is_array())
        .unwrap();
    assert_eq!(call["tool_calls"][0]["id"], "first");
    assert_eq!(call["tool_calls"][1]["id"], "second");
    assert_eq!(
        messages
            .iter()
            .filter_map(|message| message["tool_call_id"].as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert!(original.iter().any(|entry| matches!(&entry.kind, SessionEntryKind::Message(message) if serde_json::to_string(&message.to_legacy()).unwrap().contains("native-cipher"))));
}

#[tokio::test]
async fn compacted_tool_signature_only_replays_to_its_source_model_and_provider() {
    for (model_name, provider_id, expected) in [
        ("source-model", "source-provider", true),
        ("other-model", "source-provider", false),
        ("source-model", "other-provider", false),
    ] {
        let answer = |text: &str| {
            support::Response::Sse(format!(
                "{}{}",
                support::event(
                    json!({"candidates":[{"content":{"role":"model","parts":[{"text":text}]}}]})
                ),
                support::event(json!({"candidates":[{"finishReason":"STOP"}]})),
            ))
        };
        let server = support::Server::start_paths(
            "/v1",
            vec![
                "/v1/models/summary-model:streamGenerateContent?alt=sse".into(),
                format!("/v1/models/{model_name}:streamGenerateContent?alt=sse"),
            ],
            vec![answer("signed summary"), answer("signed answer")],
        )
        .await;
        let mut provider_config = GeminiConfig::new(server.endpoint.clone());
        provider_config.options.provider_id = Some(provider_id.into());
        provider_config.options.include_tool_ids = true;
        let provider = Arc::new(GeminiProvider::new(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(support::LIMIT)
                .build()
                .unwrap(),
            provider_config,
        ));
        let mut messages = history();
        let call = messages
            .iter_mut()
            .filter_map(RuntimeMessage::model_message_mut)
            .flat_map(|message| &mut message.content)
            .find(|part| matches!(part, MessagePart::ToolCall { id, .. } if id == "first"))
            .unwrap();
        if let MessagePart::ToolCall { signature, .. } = call {
            *signature = Some(ProviderSignature::Scoped {
                value: "NATIVE_SIGNATURE".into(),
                model: "source-model".into(),
                provider_id: Some("source-provider".into()),
            });
        }
        let mut config =
            AgentSessionConfig::new(provider.clone(), model_name).with_policy(policy(provider));
        config.provider_id = provider_id.into();
        config.history = messages.into();
        let session = AgentSession::new(config);
        let original = session.entries();
        assert_eq!(
            bounded(session.prompt("continue")).await.unwrap().content,
            "signed answer"
        );
        assert_original_entries(&session.entries(), &original);
        assert_eq!(count_compactions(&session.entries()), 1);
        let requests = server.requests.lock().unwrap();
        let body = &requests[1].body;
        let parts = body["contents"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["parts"].as_array().unwrap());
        let call = parts
            .into_iter()
            .find(|part| part["functionCall"]["id"] == "first")
            .unwrap();
        assert_eq!(call.get("thoughtSignature").is_some(), expected);
        if expected {
            assert_eq!(call["thoughtSignature"], "NATIVE_SIGNATURE");
        }
        assert!(!body.to_string().contains("HOST_METADATA_SECRET"));
        assert!(original.iter().any(|entry| matches!(&entry.kind, SessionEntryKind::Message(message) if serde_json::to_string(&message.to_legacy()).unwrap().contains("NATIVE_SIGNATURE"))));
    }
}

#[tokio::test]
async fn changing_a_tool_schema_recalculates_the_next_request_budget() {
    let model = Model::new(
        vec![Summary::Output(text("schema change summary"))],
        vec![text("first answer"), text("second answer")],
    );
    let mut budget = policy(model.clone());
    budget.summary.window = 6_000;
    budget.threshold = Some(3_200);
    let mut config = AgentSessionConfig::new(model.clone(), "test-model").with_policy(budget);
    config.history = vec![
        RuntimeMessage::from(format!("OLD_SCHEMA_HISTORY {}", "a".repeat(11_000))),
        RuntimeMessage::text(ModelRole::Assistant, "previous answer"),
    ]
    .into();
    let executor = Arc::new(EchoTool::new(true));
    config
        .tools
        .register_dynamic(echo_definition(), executor.clone())
        .unwrap();
    let session = AgentSession::new(config);
    bounded(session.prompt("first question")).await.unwrap();
    assert_eq!(count_compactions(&session.entries()), 0);

    let mut updated = echo_definition();
    updated.model.description = "expanded schema description ".repeat(180);
    let mut tools = kivio_agent_sdk::tools::ToolRegistry::new();
    tools.register_dynamic(updated, executor).unwrap();
    session.set_tools(tools).unwrap();
    assert_eq!(
        bounded(session.prompt("second question"))
            .await
            .unwrap()
            .content,
        "second answer"
    );
    assert_eq!(count_compactions(&session.entries()), 1);
    let calls = model.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.delivery == ModelDelivery::Collected)
            .count(),
        1
    );
    let latest = calls
        .iter()
        .rev()
        .find(|call| call.delivery == ModelDelivery::Streamed)
        .unwrap();
    assert_eq!(latest.tools[0].name, "echo");
    assert!(latest.tools[0]
        .description
        .contains("expanded schema description"));
    assert!(!texts(&latest.history)
        .join("\n")
        .contains("OLD_SCHEMA_HISTORY"));
}

#[tokio::test]
async fn prior_usage_can_trigger_compaction_before_the_local_estimate_reaches_the_budget() {
    for anchored in [false, true] {
        let model = Model::new(
            if anchored {
                vec![Summary::Output(text("anchored summary"))]
            } else {
                vec![]
            },
            vec![text("answer")],
        );
        let mut budget = policy(model.clone());
        budget.summary.window = 4_000;
        budget.threshold = Some(4_000);
        budget.state.initial_anchor_total_tokens = anchored.then_some(5_000);
        let mut config = AgentSessionConfig::new(model.clone(), "test-model").with_policy(budget);
        config.history = vec![
            RuntimeMessage::from(format!("ANCHOR_HISTORY {}", "a".repeat(9_000))),
            RuntimeMessage::text(ModelRole::Assistant, "previous answer"),
        ]
        .into();
        let session = AgentSession::new(config);
        let original = session.entries();
        assert_eq!(
            bounded(session.prompt("continue")).await.unwrap().content,
            "answer"
        );
        assert_original_entries(&session.entries(), &original);
        assert_eq!(count_compactions(&session.entries()), usize::from(anchored));
        let calls = model.calls.lock().unwrap();
        let request = calls
            .iter()
            .find(|call| call.delivery == ModelDelivery::Streamed)
            .unwrap();
        let context = texts(&request.history).join("\n");
        assert_eq!(context.contains("ANCHOR_HISTORY"), !anchored);
        assert_eq!(context.contains("anchored summary"), anchored);
    }
}

#[tokio::test]
async fn imported_usage_anchor_is_not_reused_after_compaction_in_a_later_run() {
    let model = Model::new(
        vec![Summary::Output(text("anchored summary"))],
        vec![text(&"b".repeat(9_000)), text("next answer")],
    );
    let mut budget = policy(model.clone());
    budget.summary.window = 4_000;
    budget.threshold = Some(4_000);
    budget.state.initial_anchor_total_tokens = Some(5_000);
    let mut config = AgentSessionConfig::new(model.clone(), "test-model").with_policy(budget);
    config.history = vec![
        RuntimeMessage::from(format!("ANCHOR_HISTORY {}", "a".repeat(9_000))),
        RuntimeMessage::text(ModelRole::Assistant, "previous answer"),
    ]
    .into();
    let session = AgentSession::new(config);
    bounded(session.prompt("continue")).await.unwrap();
    assert_eq!(count_compactions(&session.entries()), 1);
    assert_eq!(
        bounded(session.prompt("next")).await.unwrap().content,
        "next answer"
    );
    assert_eq!(count_compactions(&session.entries()), 1);
}

fn legacy(value: Value) -> RuntimeMessage {
    RuntimeMessage::from_legacy(value).unwrap()
}

fn history() -> Vec<RuntimeMessage> {
    vec![
        RuntimeMessage::system("Keep the user's constraints."),
        RuntimeMessage::from(format!("ARCHIVED_A {}", "a".repeat(9_000))),
        RuntimeMessage::text(ModelRole::Assistant, "earlier answer"),
        RuntimeMessage::from(format!("ARCHIVED_B {}", "b".repeat(9_000))),
        legacy(json!({
            "role":"assistant", "content":"",
            "tool_calls":[
                {"id":"first", "type":"function", "function":{"name":"echo", "arguments":"{\"text\":\"one\"}"}},
                {"id":"second", "type":"function", "function":{"name":"echo", "arguments":"{\"text\":\"two\"}"}}
            ],
            "reasoning_items":[{"model":"source-model", "item":{"type":"reasoning", "id":"native-id", "encrypted_content":"native-cipher"}}],
            "host_only":"HOST_METADATA_SECRET"
        })),
        legacy(json!({"role":"tool", "tool_call_id":"first", "content":"result one"})),
        legacy(json!({"role":"tool", "tool_call_id":"second", "content":"result two"})),
    ]
}

fn assert_original_entries(entries: &[SessionEntry], original: &[SessionEntry]) {
    assert_eq!(&entries[..original.len()], original);
}

fn count_compactions(entries: &[SessionEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| matches!(entry.kind, SessionEntryKind::Compaction { .. }))
        .count()
}

fn texts(messages: &[RuntimeMessage]) -> Vec<String> {
    messages.iter().map(RuntimeMessage::text_content).collect()
}

fn assert_pair(messages: &[RuntimeMessage]) {
    let parts = messages
        .iter()
        .filter_map(RuntimeMessage::model_message)
        .flat_map(|message| &message.content);
    let mut calls = Vec::new();
    let mut results = Vec::new();
    for part in parts {
        match part {
            MessagePart::ToolCall { id, .. } => calls.push(id.as_str()),
            MessagePart::ToolResult { tool_call_id, .. } => results.push(tool_call_id.as_str()),
            _ => {}
        }
    }
    assert_eq!(calls, ["first", "second"]);
    assert_eq!(results, ["first", "second"]);
}

struct PolicyWithInput {
    inner: BudgetContextPolicy,
    supplied: bool,
}

impl SessionPolicy for PolicyWithInput {
    fn prepare_context<'a>(
        &'a mut self,
        reason: ContextReason,
        run: ContextSnapshot<'a>,
        control: AgentRunHandle<'a>,
        progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, Result<ContextPreparation, String>> {
        self.inner.prepare_context(reason, run, control, progress)
    }

    fn context_applied<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        control: AgentRunHandle<'a>,
        progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, (Vec<RuntimeMessage>, bool)> {
        Box::pin(async move {
            let (mut inputs, blocked) =
                self.inner.context_applied(history, control, progress).await;
            if !self.supplied {
                inputs.push(RuntimeMessage::from("input after first compaction"));
                self.supplied = true;
            }
            (inputs, blocked)
        })
    }

    fn context_rejected(&mut self, phase: &str, progress: &dyn PolicyProgress) {
        self.inner.context_rejected(phase, progress);
    }
}

#[tokio::test]
async fn successive_compactions_keep_original_entries_and_complete_tool_rounds() {
    let model = Model::new(
        vec![
            Summary::Output(text("summary one")),
            Summary::Output(text("summary two")),
        ],
        vec![
            text(&format!("first answer {}", "c".repeat(9_000))),
            text("final answer"),
        ],
    );
    let mut config =
        AgentSessionConfig::new(model.clone(), "test-model").with_policy(PolicyWithInput {
            inner: policy(model.clone()),
            supplied: false,
        });
    config.history = history().into();
    let executor = Arc::new(EchoTool::new(true));
    config
        .tools
        .register_dynamic(echo_definition(), executor.clone())
        .unwrap();
    let session = AgentSession::new(config);
    let original = session.entries();

    bounded(session.continue_run()).await.unwrap();
    assert_eq!(count_compactions(&session.entries()), 1);
    bounded(session.prompt("second question")).await.unwrap();

    let entries = session.entries();
    assert_original_entries(&entries, &original);
    assert_eq!(count_compactions(&entries), 2);
    assert_eq!(executor.call_count(), 0);
    assert_eq!(entries.iter().filter(|entry| matches!(&entry.kind, SessionEntryKind::Message(message) if message.text_content() == "input after first compaction")).count(), 1);

    let calls = model.calls.lock().unwrap();
    let requests: Vec<_> = calls
        .iter()
        .filter(|call| call.delivery == ModelDelivery::Streamed)
        .collect();
    assert_eq!(requests.len(), 2);
    let latest = &requests[1].history;
    let visible = texts(latest).join("\n");
    assert!(visible.contains("summary two"));
    assert!(visible.contains("second question"));
    assert_eq!(
        texts(latest)
            .iter()
            .filter(|value| *value == "input after first compaction")
            .count(),
        1
    );
    assert!(!visible.contains("ARCHIVED_A"));
    assert!(!visible.contains("ARCHIVED_B"));
    assert_pair(latest);
    assert!(original.iter().any(|entry| matches!(&entry.kind, SessionEntryKind::Message(message) if message.source.get("host_only") == Some(&json!("HOST_METADATA_SECRET")))));
    assert!(original.iter().any(|entry| matches!(&entry.kind, SessionEntryKind::Message(message) if serde_json::to_string(&message.to_legacy()).unwrap().contains("native-cipher"))));
}

fn illegal_summary_tool_call() -> GenerateOutput {
    let mut output = text("must not be installed");
    output.tool_calls.push(PendingToolCall {
        id: "summary-call".into(),
        function_name: "echo".into(),
        arguments: json!({"text":"should never run"}),
        arguments_raw: r#"{"text":"should never run"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    output
}

#[tokio::test]
async fn failed_truncated_and_tool_call_summaries_leave_old_context_usable() {
    let mut truncated = text("incomplete summary");
    truncated.finish_reason = Some(FinishReason::from_openai("length"));
    for summary in [
        Summary::Error,
        Summary::Output(truncated),
        Summary::Output(illegal_summary_tool_call()),
    ] {
        let model = Model::new(vec![summary], vec![text("normal answer")]);
        let mut config =
            AgentSessionConfig::new(model.clone(), "test-model").with_policy(policy(model.clone()));
        let old_context = history();
        config.history = old_context.clone().into();
        let executor = Arc::new(EchoTool::new(true));
        config
            .tools
            .register_dynamic(echo_definition(), executor.clone())
            .unwrap();
        let session = AgentSession::new(config);
        let original = session.entries();
        assert_eq!(
            bounded(session.prompt("continue safely"))
                .await
                .unwrap()
                .content,
            "normal answer"
        );
        let entries = session.entries();
        assert_original_entries(&entries, &original);
        assert_eq!(count_compactions(&entries), 0);
        assert_eq!(&session.context()[..old_context.len()], old_context);
        assert_eq!(executor.call_count(), 0);
        let calls = model.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].delivery == ModelDelivery::Collected);
        assert!(calls[1].delivery == ModelDelivery::Streamed);
        assert!(texts(&calls[1].history).join("\n").contains("ARCHIVED_A"));
        assert_pair(&calls[1].history);
    }
}

#[tokio::test]
async fn cancellation_during_summary_keeps_facts_and_returns_unconsumed_input_once() {
    let model = Model::new(vec![Summary::Wait], vec![]);
    let mut config =
        AgentSessionConfig::new(model.clone(), "test-model").with_policy(policy(model.clone()));
    let old_context = history();
    config.history = old_context.clone().into();
    let session = AgentSession::new(config);
    let original = session.entries();
    let run = session.prompt("continue");
    bounded(model.summary_started.notified()).await;
    let steering = SteeringMessage::new("during-summary".into(), "queued once").unwrap();
    assert_eq!(session.steer(steering.clone()), InputAdmission::Accepted);
    assert_eq!(session.steer(steering), InputAdmission::Duplicate);
    assert_eq!(session.abort(), StopAdmission::Requested);
    let result = bounded(run).await.unwrap();
    assert!(matches!(result.status, AgentRunStatus::Cancelled));
    assert_original_entries(&session.entries(), &original);
    assert_eq!(count_compactions(&session.entries()), 0);
    assert_eq!(session.unconsumed_inputs().len(), 1);
    assert_eq!(session.unconsumed_inputs()[0].message.id, "during-summary");
    assert_eq!(&session.context()[..old_context.len()], old_context);
    assert!(texts(&session.context()).join("\n").contains("ARCHIVED_A"));
    assert_pair(&session.context());
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

enum CheckpointAction {
    Succeed,
    Fail,
    Wait,
    Oversized,
}

struct CheckpointHost {
    model: Arc<Model>,
    action: CheckpointAction,
    seen: AtomicUsize,
    entered: Notify,
    release: Notify,
    checkpoint: Mutex<SessionHistory>,
}

impl AgentHost for CheckpointHost {
    fn checkpoint_runtime<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        history: &'a kivio_agent_sdk::agent::SessionHistory,
        finishing: bool,
    ) -> AgentHostFuture<'a, Result<Vec<SessionInput>, String>> {
        Box::pin(async move {
            let summary_started = self
                .model
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| call.delivery == ModelDelivery::Collected);
            if finishing || !summary_started || self.seen.fetch_add(1, Ordering::SeqCst) != 0 {
                return Ok(vec![]);
            }
            *self.checkpoint.lock().unwrap() = history.clone();
            match self.action {
                CheckpointAction::Fail => return Err("checkpoint failed".into()),
                CheckpointAction::Wait => {
                    self.entered.notify_one();
                    self.release.notified().await;
                }
                CheckpointAction::Succeed => {}
                CheckpointAction::Oversized => {
                    return Ok(vec![SessionInput::append(RuntimeMessage::from(
                        "incoming ".repeat(40_000),
                    ))]);
                }
            }
            Ok(vec![
                SessionInput::append(RuntimeMessage::from("new checkpoint input")),
                SessionInput {
                    message: RuntimeMessage::from("already persisted input"),
                    persistence: InputPersistence::AlreadyPersisted,
                },
            ])
        })
    }
}

#[tokio::test]
async fn oversized_checkpoint_input_is_preserved_without_sending_an_overflowing_request() {
    let model = Model::new(vec![Summary::Output(text("valid summary"))], vec![]);
    let host = Arc::new(CheckpointHost {
        model: model.clone(),
        action: CheckpointAction::Oversized,
        seen: AtomicUsize::new(0),
        entered: Notify::new(),
        release: Notify::new(),
        checkpoint: Mutex::new(SessionHistory::default()),
    });
    let mut config =
        AgentSessionConfig::new(model.clone(), "test-model").with_policy(policy(model.clone()));
    config.history = history().into();
    config.host = host;
    let session = AgentSession::new(config);
    let original = session.entries();
    let result = bounded(session.prompt("continue")).await.unwrap();
    assert!(matches!(result.status, AgentRunStatus::CompactionBlocked));
    assert_original_entries(&session.entries(), &original);
    assert_eq!(count_compactions(&session.entries()), 1);
    assert!(session
        .context()
        .iter()
        .any(|message| message.text_content() == "incoming ".repeat(40_000)));
    assert!(model
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call.delivery == ModelDelivery::Collected));
}

#[tokio::test]
async fn checkpoints_preserve_original_facts_and_deliver_each_input_once() {
    for action in [
        CheckpointAction::Succeed,
        CheckpointAction::Fail,
        CheckpointAction::Wait,
    ] {
        let cancelled = matches!(action, CheckpointAction::Wait);
        let failed = matches!(action, CheckpointAction::Fail);
        let model = Model::new(
            vec![Summary::Output(text("valid summary"))],
            vec![text("finished")],
        );
        let host = Arc::new(CheckpointHost {
            model: model.clone(),
            action,
            seen: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
            checkpoint: Mutex::new(SessionHistory::default()),
        });
        let mut config =
            AgentSessionConfig::new(model.clone(), "test-model").with_policy(policy(model.clone()));
        let old_context = history();
        config.history = old_context.clone().into();
        config.host = host.clone();
        let session = AgentSession::new(config);
        let original = session.entries();
        let run = session.prompt("continue");
        if cancelled {
            bounded(host.entered.notified()).await;
            assert_eq!(session.abort(), StopAdmission::Requested);
            host.release.notify_one();
        }
        let result = bounded(run).await;
        if failed {
            assert!(matches!(result, Err(AgentRunError::Failed { .. })));
        } else {
            let result = result.unwrap();
            if cancelled {
                assert!(matches!(result.status, AgentRunStatus::Cancelled));
            }
            let generated = texts(&result.generated_history);
            assert_eq!(
                generated
                    .iter()
                    .filter(|text| *text == "new checkpoint input")
                    .count(),
                1
            );
            assert!(!generated
                .iter()
                .any(|text| text == "already persisted input"));
        }
        assert_original_entries(host.checkpoint.lock().unwrap().entries(), &original);
        assert_eq!(
            count_compactions(host.checkpoint.lock().unwrap().entries()),
            0
        );
        assert!(texts(host.checkpoint.lock().unwrap().context())
            .join("\n")
            .contains("ARCHIVED_A"));
        assert_original_entries(&session.entries(), &original);
        assert_eq!(
            count_compactions(&session.entries()),
            usize::from(!failed && !cancelled)
        );
        let context = texts(&session.context());
        if failed || cancelled {
            assert!(context.iter().any(|text| text.contains("ARCHIVED_A")));
            assert_eq!(&session.context()[..old_context.len()], old_context);
        }
        assert_pair(&session.context());
        if !failed {
            assert_eq!(
                context
                    .iter()
                    .filter(|text| *text == "new checkpoint input")
                    .count(),
                1
            );
            assert_eq!(
                context
                    .iter()
                    .filter(|text| *text == "already persisted input")
                    .count(),
                1
            );
        }
        let calls = model.calls.lock().unwrap();
        if !failed && !cancelled {
            let wire = texts(
                &calls
                    .iter()
                    .find(|call| call.delivery == ModelDelivery::Streamed)
                    .unwrap()
                    .history,
            );
            assert_eq!(
                wire.iter()
                    .filter(|text| *text == "new checkpoint input")
                    .count(),
                1
            );
            assert_eq!(
                wire.iter()
                    .filter(|text| *text == "already persisted input")
                    .count(),
                1
            );
        } else {
            assert!(calls
                .iter()
                .all(|call| call.delivery == ModelDelivery::Collected));
        }
    }
}
