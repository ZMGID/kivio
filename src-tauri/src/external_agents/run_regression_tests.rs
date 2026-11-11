use super::*;
use crate::chat::repository::repository;
use serde_json::{json, Value};
use std::path::Path;
use tauri::Manager;

const PEER: &str = r#"
import json, pathlib, sys
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('fixture 1.0.0')
    sys.exit(0)
if 'models' in sys.argv:
    sys.exit(0)
mode = (root / 'mode').read_text()
native = 'fixture-native'
def send(value):
    print(json.dumps(dict(jsonrpc='2.0', **value)), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    with (root / 'wire.jsonl').open('a', encoding='utf-8') as log:
        log.write(json.dumps(request) + '\n')
    method = request.get('method')
    rid = request.get('id')
    if method == 'initialize':
        send(dict(id=rid, result=dict(protocolVersion=1, agentCapabilities=dict(loadSession=True))))
    elif method in ('session/new', 'session/load'):
        result = dict(sessionId=native, configOptions=[dict(id='effort', category='reasoning')],
            models=dict(currentModelId='fixture-model', availableModels=[dict(modelId='fixture-model', name='Fixture')]))
        send(dict(id=rid, result=result))
    elif method == 'session/set_config_option':
        marker = root / 'config-failed'
        if mode == 'history' and not marker.exists():
            marker.write_text('failed before prompt')
            sys.exit(0)
        send(dict(id=rid, result={}))
    elif method == 'session/prompt':
        send(dict(method='session/update', params=dict(sessionId=native,
            update=dict(sessionUpdate='agent_message_chunk', content=dict(type='text', text='partial answer')))))
        if mode == 'error':
            send(dict(id=rid, error=dict(code=-32000, message='fixture terminal failure')))
        else:
            send(dict(id=rid, result=dict(stopReason='cancelled' if mode == 'cancel' else 'end_turn')))
    elif rid is not None:
        send(dict(id=rid, result={}))
"#;

pub(crate) fn isolated_peer_test() -> bool {
    if std::env::var_os("KIVIO_ACP_REGRESSION_CHILD").is_some() {
        return false;
    }
    let current = std::thread::current();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            current.name().unwrap(),
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("KIVIO_ACP_REGRESSION_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "isolated ACP regression failed\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    true
}

pub(crate) fn run_peer<F: std::future::Future>(run: F) -> F::Output {
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(30), run)
            .await
            .expect("controlled ACP caller did not settle within 30 seconds")
    })
}

pub(crate) fn with_acp_peer(mode: &str, test: impl FnOnce(&AppHandle, &Path)) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let workspace = directory.path().join("workspace");
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("GROK_HOME", &home);
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(directory.path().join("peer.py"), PEER).unwrap();
    std::fs::write(directory.path().join("mode"), mode).unwrap();
    let binary = {
        let path = directory.path().join("peer.cmd");
        std::fs::write(&path, "@echo off\r\npython -u \"%~dp0peer.py\" %*\r\n").unwrap();
        path
    };
    let mut previous = crate::settings::Settings::default();
    previous.chat.external_cli_agents = crate::external_agents::registry::AGENT_DEFS
        .iter()
        .filter_map(|def| {
            crate::external_agents::overrides::agent_config(def.id)
                .map(|config| (def.id.into(), config))
        })
        .collect();
    struct Restore(crate::settings::Settings);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::external_agents::overrides::sync_from_settings(&self.0);
        }
    }
    let _restore = Restore(previous);
    let state = AppState::new_headless(
        crate::settings::Settings::default(),
        directory.path().join("usage"),
    );
    state.update_settings_for_test(|settings| {
        settings.chat_memory.enabled = false;
        settings.chat_tools.request_debug_enabled = mode == "diagnostics";
        settings.chat_tools.native_tools.working_directory =
            workspace.to_string_lossy().into_owned();
        settings.chat.external_cli_agents.insert(
            "grok".into(),
            crate::settings::ExternalCliAgentConfig {
                path: binary.to_string_lossy().into_owned(),
                env: vec![crate::settings::CliEnvVar {
                    key: "GROK_HOME".into(),
                    value: home.to_string_lossy().into_owned(),
                }],
                ..Default::default()
            },
        );
    });
    crate::external_agents::overrides::sync_from_settings(&state.settings_read());
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("settings.json"),
        serde_json::to_vec(&json!({
            "settings": state.settings_read().clone()
        }))
        .unwrap(),
    )
    .unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = data.to_string_lossy().into_owned();
    context.config_mut().app.windows.clear();
    let application = tauri::Builder::default()
        .any_thread()
        .manage(state)
        .manage(crate::chat::repository::ConversationRepository::default())
        .manage(crate::app_mcp::KivioServer::default())
        .build(context)
        .unwrap();
    test(application.handle(), directory.path());
}

pub(crate) fn conversation(id: &str) -> Conversation {
    serde_json::from_value(json!({
        "id":id,"title":"ACP regression","provider_id":"","model":"",
        "messages":[],"created_at":1,"updated_at":1,
        "agent_runtime":{"kind":"external","externalAgentId":"grok"}
    }))
    .unwrap()
}

fn message(id: &str, role: &str, text: &str) -> crate::chat::ChatMessage {
    serde_json::from_value(json!({"id":id,"role":role,"content":text,"timestamp":1})).unwrap()
}

#[test]
fn subagent_completion_waits_for_the_parent_message_to_be_saved() {
    if isolated_peer_test() {
        return;
    }
    with_acp_peer("subagent-storage", |app, _| {
        run_peer(async {
            use crate::external_agents::session::live::{
                LiveSession, SessionControl, SessionIdentity,
            };
            use std::sync::{atomic::AtomicBool, Arc};

            let state = app.state::<AppState>();
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            let control =
                SessionControl::spawn(tx, async move { while rx.recv().await.is_some() {} });
            let identity = SessionIdentity::from_control(&control);
            let actor_id = identity.instance_id().unwrap();
            let id = "conv_subagent_storage";
            state.external_live_sessions().register(
                id.into(),
                LiveSession {
                    control,
                    _chat_credential: None,
                    agent_id: "claude".into(),
                    cwd: String::new(),
                    launch_config: LaunchConfig::default(),
                    last_activity: Instant::now(),
                    child_pid: None,
                    turns_served: 1,
                    busy: Arc::new(AtomicBool::new(false)),
                },
            );
            let repo = repository(app);
            let original = repo.create(app, conversation(id)).await.unwrap();
            let key = SubagentCardKey::LaunchedBy("toolu-child".into());
            let update = SubagentCardUpdate {
                status: "completed".into(),
                preview: "Final answer".into(),
                steps: vec![],
                summary: Some("Final answer".into()),
            };
            assert!(
                !write_subagent_card(app, id, &identity, actor_id, &key, &update)
                    .await
                    .unwrap()
            );
            assert_eq!(repo.get(app, id).await.unwrap().revision, original.revision);
            assert!(write_subagent_card(
                app,
                "conv_missing_subagent",
                &identity,
                actor_id,
                &key,
                &update
            )
            .await
            .is_err());

            let runtime = state.chat_runtime();
            let generation = runtime.begin_reply_generation(id).unwrap();
            let saving = persist_subagent_card(
                app.clone(),
                id.into(),
                key,
                update.status.clone(),
                update.preview.clone(),
                vec![],
                update.summary.clone(),
                identity.clone(),
            );
            let (_, next) = tokio::join!(biased; saving, async {
                repo.mutate(app, id, |conversation| {
                let mut parent = message("parent", "assistant", "Dispatched");
                parent.tool_calls.push(serde_json::from_value(json!({
                    "id":"toolu-child", "name":"SendMessage", "source":"external_cli", "status":"running",
                    "structured_content":{"resumedSubagent":{"taskId":"child","name":"Worker"}}
                })).unwrap());
                conversation.messages.push(parent);
                Ok(())
                }).await.unwrap();
                runtime.end_generation(id, generation);
                runtime.begin_reply_generation(id).unwrap()
            });
            let stored = repo.get(app, id).await.unwrap();
            let record = &stored.messages[0].tool_calls[0];
            assert_eq!(record.status, ToolCallStatus::Success);
            assert_eq!(record.result_preview.as_deref(), Some("Final answer"));
            assert!(record.completed_at.is_some());
            assert_eq!(
                record.structured_content.as_ref().unwrap()["resumedSubagent"]["taskId"],
                "child"
            );
            assert!(runtime.is_generation_active(id, next));
            runtime.end_generation(id, next);

            state.external_live_sessions().remove(id);
            assert!(write_subagent_card(
                app,
                id,
                &identity,
                actor_id,
                &SubagentCardKey::LaunchedBy("toolu-child".into()),
                &update
            )
            .await
            .is_err());
            assert_eq!(repo.get(app, id).await.unwrap().revision, stored.revision);
        });
    });
}

fn wire(directory: &Path) -> Vec<Value> {
    std::fs::read_to_string(directory.join("wire.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn first_history_handoff_keeps_prefix_when_unsent_config_failure_resumes_empty_native() {
    if isolated_peer_test() {
        return;
    }
    with_acp_peer("history", |app, directory| {
        run_peer(async {
            let mut source = conversation("conv_history_source");
            source.messages = vec![
                message("prefix-user", "user", "retained prefix question"),
                message("prefix-answer", "assistant", "retained prefix answer"),
            ];
            source.agent_runtime.external_reasoning = Some("high".into());
            repository(app).create(app, source).await.unwrap();
            crate::external_agents::workspace::ensure_effective_cwd(
                app,
                "conv_history_source",
                None,
            )
            .unwrap();
            let branch = crate::chat::commands::mutations::chat_fork_conversation(
                app.clone(),
                app.state(),
                "conv_history_source".into(),
                "prefix-answer".into(),
                None,
            )
            .await
            .unwrap();
            let id = branch["conversation"]["id"].as_str().unwrap();
            repository(app)
                .append_message(app, id, message("request", "user", "next question"))
                .await
                .unwrap();
            let mut branch = repository(app).get(app, id).await.unwrap();
            assert_eq!(
                branch.external_history.as_ref().unwrap().method,
                crate::chat::types::ExternalHistoryMethod::Replay
            );
            let result = run_external_cli_reply(
                app,
                &app.state(),
                &mut branch,
                None,
                "next question",
                &[],
                &[],
                None,
                AgentRunEntry::Send,
            )
            .await;
            app.state::<AppState>()
                .external_live_sessions()
                .close_all()
                .await;
            assert_eq!(result.unwrap(), ExternalCliTurnOutcome::Completed);
            let requests = wire(directory);
            let methods: Vec<_> = requests
                .iter()
                .filter_map(|r| r["method"].as_str())
                .collect();
            assert_eq!(methods.iter().filter(|m| **m == "session/new").count(), 1);
            assert_eq!(methods.iter().filter(|m| **m == "session/load").count(), 1);
            let prompts: Vec<_> = requests
                .iter()
                .filter(|r| r["method"] == "session/prompt")
                .collect();
            assert_eq!(
                prompts.len(),
                1,
                "the failed config exchange must precede prompt delivery"
            );
            let prompt = prompts[0]["params"]["prompt"].to_string();
            assert!(
                prompt.contains("retained prefix question"),
                "first native prompt lost retained user history: {prompt}"
            );
            assert!(
                prompt.contains("retained prefix answer"),
                "first native prompt lost retained assistant history: {prompt}"
            );
            assert!(repository(app)
                .get(app, id)
                .await
                .unwrap()
                .external_history
                .is_none());
        })
    });
}

#[test]
fn failed_or_cancelled_external_turn_preserves_partial_answer() {
    if isolated_peer_test() {
        return;
    }
    for mode in ["error", "cancel", "complete"] {
        with_acp_peer(mode, |app, _| {
            run_peer(async {
                let mut conversation = conversation(&format!("conv_terminal_{mode}"));
                conversation
                    .messages
                    .push(message("request", "user", "write an answer"));
                let mut conversation = repository(app).create(app, conversation).await.unwrap();
                #[cfg(debug_assertions)]
                let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                #[cfg(debug_assertions)]
                {
                    let events = events.clone();
                    let id = conversation.id.clone();
                    crate::chat::protocol::set_protocol_debug_sink(Some(std::sync::Arc::new(
                        move |event| {
                            if let crate::chat::protocol::ChatProtocolEvent::Run(event) = event {
                                if event.conversation_id == id {
                                    events.lock().unwrap().push(event.event.clone());
                                }
                            }
                        },
                    )));
                }
                let result = run_external_cli_reply(
                    app,
                    &app.state(),
                    &mut conversation,
                    None,
                    "write an answer",
                    &[],
                    &[],
                    None,
                    AgentRunEntry::Send,
                )
                .await;
                app.state::<AppState>()
                    .external_live_sessions()
                    .close_all()
                    .await;
                #[cfg(debug_assertions)]
                crate::chat::protocol::set_protocol_debug_sink(None);
                let outcome = result.unwrap();
                match mode {
                    "error" => assert!(matches!(outcome, ExternalCliTurnOutcome::Failed(_))),
                    "cancel" => assert_eq!(outcome, ExternalCliTurnOutcome::Cancelled),
                    _ => assert_eq!(outcome, ExternalCliTurnOutcome::Completed),
                }
                let saved = repository(app).get(app, &conversation.id).await.unwrap();
                assert_eq!(
                    saved.messages.len(),
                    2,
                    "one assistant message per terminal"
                );
                let answer = saved.messages.last().unwrap();
                assert!(answer.content.contains("partial answer"));
                if let ExternalCliTurnOutcome::Failed(message) = &outcome {
                    assert_eq!(answer.content.matches(message.as_str()).count(), 1);
                }
                assert_eq!(
                    answer.stream_outcome.as_deref(),
                    Some(match mode {
                        "cancel" => "cancelled",
                        "error" => "error",
                        _ => "completed",
                    })
                );
                #[cfg(debug_assertions)]
                {
                    use crate::chat::protocol::ChatRunEvent;
                    let events = events.lock().unwrap();
                    let terminals: Vec<_> = events
                        .iter()
                        .filter(|event| {
                            matches!(
                                event,
                                ChatRunEvent::RunCompleted { .. }
                                    | ChatRunEvent::RunFailed { .. }
                                    | ChatRunEvent::RunCancelled { .. }
                            )
                        })
                        .collect();
                    assert_eq!(terminals.len(), 1, "one published terminal per run");
                    assert!(matches!(
                        (mode, terminals[0]),
                        ("error", ChatRunEvent::RunFailed { .. })
                            | ("cancel", ChatRunEvent::RunCancelled { .. })
                            | ("complete", ChatRunEvent::RunCompleted { .. })
                    ));
                }
            })
        });
    }
}

#[test]
fn rejected_external_compact_preserves_compression_timestamp() {
    if isolated_peer_test() {
        return;
    }
    for mode in ["error", "cancel", "complete"] {
        with_acp_peer(mode, |app, _| {
            run_peer(async {
                let mut conversation = conversation("conv_compact_failure");
                conversation.context_state.last_compressed_at = Some(123);
                conversation.context_state.warning = Some("keep warning".into());
                conversation.context_state.compression_count = 2;
                let before = conversation.context_state.clone();
                let conversation = repository(app).create(app, conversation).await.unwrap();
                let cwd = crate::external_agents::workspace::resolve_detection_cwd(
                    app,
                    Some(&conversation.id),
                )
                .unwrap();
                app.state::<AppState>()
                    .external_discovery()
                    .set_cached_external_slash_commands(
                        crate::external_agents::slash::cache_key("grok", &cwd.to_string_lossy()),
                        vec![crate::external_agents::types::ExternalCliSlashCommand {
                            name: "compact".into(),
                            slash: "/compact".into(),
                            description: None,
                            argument_hint: None,
                        }],
                    );
                let result = crate::chat::commands::context::chat_compress_context(
                    app.clone(),
                    app.state(),
                    conversation.id.clone(),
                )
                .await;
                app.state::<AppState>()
                    .external_live_sessions()
                    .close_all()
                    .await;
                let saved = repository(app).get(app, &conversation.id).await.unwrap();
                if mode == "complete" {
                    assert!(result.is_ok(), "completed compact failed: {result:?}");
                    assert_ne!(
                        saved.context_state.last_compressed_at,
                        before.last_compressed_at
                    );
                    assert!(saved.context_state.warning.is_none());
                } else {
                    assert!(result.is_err(), "rejected compact command returned success");
                    assert_eq!(
                        saved.context_state.last_compressed_at, before.last_compressed_at,
                        "rejected compact changed its success timestamp"
                    );
                    assert_eq!(saved.context_state.warning, before.warning);
                }
                assert_eq!(
                    saved.context_state.compression_count,
                    before.compression_count
                );
                assert_eq!(
                    serde_json::to_value(&saved.context_state.compaction_boundaries).unwrap(),
                    serde_json::to_value(&before.compaction_boundaries).unwrap()
                );
                assert!(saved
                    .messages
                    .last()
                    .unwrap()
                    .content
                    .contains("partial answer"));
            })
        });
    }
}

#[test]
fn native_diagnostics_records_real_conversation_id_from_common_entry() {
    if isolated_peer_test() {
        return;
    }
    with_acp_peer("diagnostics", |app, _| {
        run_peer(async {
            let uuid = Uuid::new_v4();
            let mut conversation = conversation(&format!("conv_{uuid}"));
            conversation
                .messages
                .push(message("request", "user", "produce a trace"));
            let mut conversation = repository(app).create(app, conversation).await.unwrap();
            run_external_cli_reply(
                app,
                &app.state(),
                &mut conversation,
                None,
                "produce a trace",
                &[],
                &[],
                None,
                AgentRunEntry::Send,
            )
            .await
            .unwrap()
            .require_completed()
            .unwrap();
            let state = app.state::<AppState>();
            state.external_live_sessions().close_all().await;
            let trace = state.usage_dir.join("native-protocol/current.jsonl");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let records = loop {
                let records: Vec<Value> = std::fs::read_to_string(&trace)
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();
                if !records.is_empty() {
                    break records;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "common entry produced no native diagnostic records"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            };
            assert!(
                records
                    .iter()
                    .all(|record| record["conversation"] == conversation.id),
                "native trace lost common-entry conversation attribution: {records:?}"
            );
        })
    });
}
