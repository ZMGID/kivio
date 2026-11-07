use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use kivio_agent_sdk::{
    agent::{
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        session::CompactionPlan,
        AgentAdmissionError, AgentSession, AgentSessionConfig, EntryId, RuntimeMessage,
        SessionEntry, SessionEntryKind, SessionHistory, SessionSnapshot,
    },
    model::{
        GenerateOutput, MessagePart, ModelError, ModelMessage, ModelRole, PendingToolCall,
        ProviderSignature,
    },
};
use kivio_agent_sdk_consumer::{echo_definition, EchoTool};
use serde_json::json;

struct Model {
    outputs: Mutex<VecDeque<GenerateOutput>>,
    calls: Mutex<Vec<ModelCall>>,
}

impl Model {
    fn new(outputs: Vec<GenerateOutput>) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(outputs.into()),
            calls: Mutex::new(vec![]),
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
                .expect("unexpected model call"))
        })
    }
}

fn answer(text: &str) -> GenerateOutput {
    GenerateOutput::text(text.into(), None)
}

fn round_trip(history: &SessionHistory) -> SessionHistory {
    let bytes = serde_json::to_vec(&history.snapshot().unwrap()).unwrap();
    serde_json::from_slice::<SessionSnapshot>(&bytes)
        .unwrap()
        .into_history()
}

#[test]
fn snapshot_preserves_all_branches_compaction_sources_and_media_references() {
    let mut user = RuntimeMessage::model(ModelMessage {
        role: ModelRole::User,
        content: vec![MessagePart::Image {
            mime_type: "image/png".into(),
            data: String::new(),
            path: Some("host://missing-but-retained/image.png".into()),
            detail: None,
        }],
    });
    user.source
        .insert("origin".into(), json!({"id":"stable-ui","nested":[1,2]}));
    let mut history = SessionHistory::from(vec![RuntimeMessage::system("system"), user]);
    let shared = history.active_leaf().unwrap();
    history.push(RuntimeMessage::text(ModelRole::Assistant, "answer A"));
    let a = history.active_leaf().unwrap();
    history
        .compact(CompactionPlan {
            covered: 1..3,
            replacement: vec![RuntimeMessage::text(ModelRole::User, "summary A")],
        })
        .unwrap();
    let compacted_a = history.active_leaf().unwrap();
    history.fork(shared).unwrap();
    history.push(RuntimeMessage::text(ModelRole::Assistant, "answer B"));
    let b = history.active_leaf().unwrap();
    let mut restored = round_trip(&history);
    assert_eq!(restored.entries(), history.entries());
    assert_eq!(restored.active_leaf(), Some(b));
    assert_eq!(restored.context(), history.context());
    assert_eq!(
        restored.attachment_references(),
        history.attachment_references()
    );
    restored.switch_branch(Some(compacted_a)).unwrap();
    assert_eq!(
        restored.context().last().unwrap().text_content(),
        "summary A"
    );
    restored.switch_branch(Some(a)).unwrap();
    assert_eq!(
        restored.context().last().unwrap().text_content(),
        "answer A"
    );
    assert_eq!(restored.context()[1].source["origin"]["id"], "stable-ui");
}

#[test]
fn snapshot_ids_are_lossless_strings_and_invalid_imports_fail() {
    let history = SessionHistory::from_entries(
        vec![SessionEntry {
            id: EntryId(u64::MAX),
            parent_id: None,
            kind: SessionEntryKind::Message(RuntimeMessage::text(ModelRole::User, "question")),
        }],
        Some(EntryId(u64::MAX)),
    )
    .unwrap();
    let valid = serde_json::to_value(history.snapshot().unwrap()).unwrap();
    assert_eq!(valid["entries"][0]["id"], u64::MAX.to_string());
    assert_eq!(valid["active_leaf"], u64::MAX.to_string());
    let mut restored = round_trip(&history);
    restored.push(RuntimeMessage::text(ModelRole::Assistant, "answer"));
    assert_ne!(restored.active_leaf(), Some(EntryId(u64::MAX)));

    let mut invalids = Vec::new();
    let mut future = valid.clone();
    future["version"] = json!(999);
    invalids.push(future);
    for id in [
        json!(1),
        json!(-1),
        json!(1.5),
        json!("18446744073709551616"),
    ] {
        let mut invalid = valid.clone();
        invalid["entries"][0]["id"] = id;
        invalids.push(invalid);
    }
    let mut missing = valid.clone();
    missing["active_leaf"] = json!("55");
    invalids.push(missing);
    let mut cycle = valid.clone();
    cycle["entries"][0]["parent_id"] = json!(u64::MAX.to_string());
    invalids.push(cycle);
    let mut duplicate = valid.clone();
    duplicate["entries"]
        .as_array_mut()
        .unwrap()
        .push(valid["entries"][0].clone());
    invalids.push(duplicate);
    let mut unknown = valid.clone();
    unknown["entries"][0]["kind"] = json!({"unknown_required_fact":{}});
    invalids.push(unknown);
    for invalid in invalids {
        assert!(
            serde_json::from_value::<SessionSnapshot>(invalid.clone()).is_err(),
            "accepted {invalid}"
        );
    }
    assert_eq!(history.entries().len(), 1);
}

#[test]
fn grafting_concurrent_arms_remaps_compaction_without_changing_selection() {
    let base = SessionHistory::from(vec![RuntimeMessage::text(ModelRole::User, "shared")]);
    let anchor = base.active_leaf();
    let mut left = base.clone();
    left.push(RuntimeMessage::text(ModelRole::Assistant, "left"));
    let selected = left.active_leaf();
    let original = left.entries().to_vec();
    let mut right = base.clone();
    right.push(RuntimeMessage::text(ModelRole::Assistant, "right"));
    let right_answer = right.active_leaf().unwrap();
    right
        .compact(CompactionPlan {
            covered: 0..2,
            replacement: vec![RuntimeMessage::text(ModelRole::User, "right summary")],
        })
        .unwrap();
    right.push(RuntimeMessage::text(ModelRole::User, "continue right"));
    let graft = left.graft_branch(&right, anchor).unwrap();
    assert_eq!(left.active_leaf(), selected);
    assert_eq!(&left.entries()[..original.len()], original);
    assert_ne!(graft.ids[&right_answer], right_answer);
    left.switch_branch(graft.leaf).unwrap();
    assert_eq!(left.context(), right.context());
    assert_eq!(round_trip(&left).context(), right.context());
    left.switch_branch(selected).unwrap();
    assert_eq!(left.context().last().unwrap().text_content(), "left");
    let before = left.clone();
    let unrelated = SessionHistory::from(vec![RuntimeMessage::text(ModelRole::User, "different")]);
    assert!(left.graft_branch(&unrelated, anchor).is_err());
    assert_eq!(left, before);
}

#[tokio::test]
async fn restored_session_rebinds_authority_and_does_not_reexecute_old_tools() {
    let mut call = answer("");
    call.tool_calls.push(PendingToolCall {
        id: "completed-call".into(),
        function_name: "echo".into(),
        arguments: json!({"text":"old result"}),
        arguments_raw: r#"{"text":"old result"}"#.into(),
        arguments_parse_error: None,
        signature: Some(ProviderSignature::Scoped {
            value: "opaque-replay".into(),
            model: "saved-model".into(),
            provider_id: Some("saved-provider".into()),
        }),
    });
    let first_model = Model::new(vec![call, answer("first answer")]);
    let echo = Arc::new(EchoTool::new(true));
    let mut config = AgentSessionConfig::new(first_model, "saved-model");
    config.provider_id = "saved-provider".into();
    config.conversation_id = "saved-conversation".into();
    config.options.temperature = Some(0.25);
    config.options.provider_options = json!({"authorization":"must-not-persist"});
    config
        .tools
        .register_dynamic(echo_definition(), echo.clone())
        .unwrap();
    let original = AgentSession::new(config);
    let pending = original.prompt("first question");
    assert!(matches!(
        original.snapshot(),
        Err(AgentAdmissionError::Busy)
    ));
    pending.await.unwrap();
    assert_eq!(echo.call_count(), 1);
    let old_entries = original.entries();
    let bytes = serde_json::to_vec(&original.snapshot().unwrap()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("must-not-persist"));
    let saved: SessionSnapshot = serde_json::from_slice(&bytes).unwrap();
    let second_model = Model::new(vec![answer("next answer")]);
    let mut config = AgentSessionConfig::from_snapshot(second_model.clone(), saved).unwrap();
    assert_eq!(config.model, "saved-model");
    assert_eq!(config.conversation_id, "saved-conversation");
    assert_eq!(config.options.temperature, Some(0.25));
    assert!(config.tools.definitions().is_empty());
    assert!(config
        .options
        .provider_options
        .as_object()
        .unwrap()
        .is_empty());
    config
        .tools
        .register_dynamic(echo_definition(), echo.clone())
        .unwrap();
    let restored = AgentSession::new(config);
    assert_eq!(restored.entries(), old_entries);
    assert!(second_model.calls.lock().unwrap().is_empty());
    restored.prompt("next question").await.unwrap();
    assert_eq!(echo.call_count(), 1);
    assert_eq!(second_model.calls.lock().unwrap().len(), 1);
    assert_eq!(&restored.entries()[..old_entries.len()], old_entries);
}

#[test]
fn history_only_restore_requires_host_configuration() {
    let snapshot = SessionHistory::default().snapshot().unwrap();
    assert!(snapshot.config().is_none());
    assert!(AgentSessionConfig::from_snapshot(Model::new(vec![]), snapshot).is_err());
}

#[test]
fn unknown_nested_payloads_cannot_be_silently_erased_on_import() {
    let mut history = SessionHistory::from(vec![RuntimeMessage::text(ModelRole::User, "known")]);
    history.push(RuntimeMessage::model(ModelMessage {
        role: ModelRole::Assistant,
        content: vec![MessagePart::ToolCall {
            id: "c".into(),
            name: "read".into(),
            arguments: json!({"arbitrary":true}),
            arguments_raw: r#"{"arbitrary":true}"#.into(),
            signature: Some(ProviderSignature::Scoped {
                value: "signature".into(),
                model: "model".into(),
                provider_id: None,
            }),
        }],
    }));
    history.push(RuntimeMessage::model(ModelMessage {
        role: ModelRole::Tool,
        content: vec![MessagePart::ToolResult {
            tool_call_id: "c".into(),
            content: "result".into(),
            is_error: false,
            artifacts: vec![kivio_agent_sdk::model::ModelArtifact {
                id: None,
                name: "output".into(),
                mime_type: "text/plain".into(),
                data_url: String::new(),
                size_bytes: None,
                path: Some("host://file".into()),
            }],
        }],
    }));
    let encoded = serde_json::to_value(history.snapshot().unwrap()).unwrap();
    for path in [
        "/entries/0/kind/value/body/value",
        "/entries/0/kind/value/body/value/content/0",
        "/entries/1/kind/value/body/value/content/0/signature",
        "/entries/2/kind/value/body/value/content/0/artifacts/0",
    ] {
        let mut invalid = encoded.clone();
        invalid
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("required_future_payload".into(), json!({"must":"survive"}));
        assert!(
            serde_json::from_value::<SessionSnapshot>(invalid).is_err(),
            "accepted unknown field at {path}"
        );
    }
    assert_eq!(round_trip(&history).entries(), history.entries());
}
