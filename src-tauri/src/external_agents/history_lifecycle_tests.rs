use super::*;
use crate::chat::repository::repository;
use crate::chat::types::{AgentRuntimeConfig, ExternalHistoryMethod, ExternalHistoryTransition};
use crate::chat::{ChatMessage, Conversation};
use crate::external_agents::session::{begin_history_delivery, settle_history_delivery};
use tauri::Manager;

async fn import_prepared(
    app: &tauri::AppHandle,
    project_id: &str,
    def: &RuntimeAgentDef,
    session_id: &str,
    root: String,
    history: (
        Vec<crate::external_agents::import_history::ImportedMessage>,
        Option<PathBuf>,
        Option<i64>,
    ),
) -> Result<String, String> {
    let lock = import_lock(def.id, session_id);
    let _guard = lock.lock().await;
    if let Some(id) = existing_import(app, def.id, session_id).await? {
        return Ok(id);
    }
    publish_import(app, project_id, def, session_id, root, history).await
}

fn message(id: &str, role: &str, text: &str) -> ChatMessage {
    serde_json::from_value(serde_json::json!({"id":id,"role":role,"content":text,"timestamp":1}))
        .unwrap()
}

fn history() -> Vec<crate::external_agents::import_history::ImportedMessage> {
    vec![
        message("user_a", "user", "prefix A"),
        message("answer_a", "assistant", "answer A"),
    ]
    .into_iter()
    .map(
        |message| crate::external_agents::import_history::ImportedMessage {
            message,
            images: vec![],
        },
    )
    .collect()
}

async fn save_answer(
    app: &tauri::AppHandle,
    conversation: &mut Conversation,
    id: &str,
    outcome: &str,
    attempt: &crate::external_agents::session::HistoryAttempt,
) -> Result<(), String> {
    crate::chat::commands::push_assistant_message(
        app,
        &app.state(),
        &crate::settings::Settings::default(),
        conversation,
        id.into(),
        "collected answer".into(),
        None,
        vec![],
        vec![],
        vec![],
        vec![],
        None,
        None,
        Some("send"),
        Some(outcome),
        None,
        None,
        None,
        None,
        Some(attempt),
    )
    .await
}

async fn dispatch(
    app: &tauri::AppHandle,
    conversation: &Conversation,
    generation: u64,
    native: &str,
    attempt: &mut Option<crate::external_agents::session::HistoryAttempt>,
    request: &str,
    expected_prefix: &[&str],
) {
    use crate::external_agents::session::live::{SessionCommand, SessionControl, TurnDelivery};
    let prompt = crate::external_agents::prompt::compose_history_handoff(
        conversation,
        crate::external_agents::prompt::compose_external_prompt_passthrough(request),
        request,
    )
    .unwrap();
    for text in expected_prefix {
        assert_eq!(prompt.full_prompt.matches(text).count(), 1);
    }
    if conversation.external_history.is_none() {
        assert!(!prompt.full_prompt.contains("prefix A"));
    }
    let app_owned = app.clone();
    let id = conversation.id.clone();
    let expected_prompt = prompt.full_prompt.clone();
    let native_owned = native.to_owned();
    let pending = conversation.external_history.is_some();
    let (sender, mut commands) = tokio::sync::mpsc::channel(1);
    let control = SessionControl::spawn(sender, async move {
        let Some(SessionCommand::RunTurn {
            prompt,
            delivery,
            done,
            ..
        }) = commands.recv().await
        else {
            panic!("turn missing");
        };
        assert_eq!(prompt, expected_prompt);
        if pending {
            let saved = repository(&app_owned).get(&app_owned, &id).await.unwrap();
            assert!(matches!(saved.external_history.unwrap().delivery,
                crate::chat::types::ExternalHistoryDelivery::MayHaveSent { native_id, .. } if native_id == native_owned));
        }
        delivery.mark_possibly_sent();
        done.send(Ok(())).unwrap();
    });
    let history = conversation.external_history.as_ref().map(|history| {
        crate::external_agents::run::HistoryDispatch {
            app,
            transition_id: &history.id,
            native_id: native,
            attempt,
        }
    });
    crate::external_agents::run::drive_persistent_turn(
        &control,
        &TurnDelivery::default(),
        &|| true,
        None,
        &app.state::<crate::state::AppState>(),
        &conversation.id,
        generation,
        prompt.full_prompt,
        None,
        None,
        &[],
        &[],
        &mut |_, _| {},
        &|| false,
        None,
        history,
    )
    .await
    .unwrap();
}

#[test]
fn external_history_publication_and_recovery_on_real_files() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = data.to_string_lossy().into_owned();
    context.config_mut().app.windows.clear();
    let application = tauri::Builder::default()
        .any_thread()
        .manage(crate::state::test_app_state())
        .manage(crate::chat::repository::ConversationRepository::default())
        .build(context)
        .unwrap();
    let app = application.handle();
    assert_eq!(app.path().app_data_dir().unwrap(), data);
    let root = workspace.to_string_lossy().into_owned();
    crate::chat::storage::create_project(
        app,
        crate::chat::ChatProject {
            id: "proj_history".into(),
            name: "History test".into(),
            description: None,
            color: None,
            root_path: Some(root.clone()),
            created_at: 1,
            updated_at: 1,
        },
    )
    .unwrap();
    struct ResetFailures;
    impl Drop for ResetFailures {
        fn drop(&mut self) {
            crate::chat::storage::fail_write_under_for_test(None);
        }
    }
    let _reset = ResetFailures;
    tauri::async_runtime::block_on(async {
        let def = get_agent_def("claude").unwrap();
        for label in ["import record", "external agent session", "conversation"] {
            crate::chat::storage::fail_write_under_for_test(Some((&data, label)));
            assert!(import_prepared(
                app,
                "proj_history",
                def,
                "stage-failure",
                root.clone(),
                (history(), None, None)
            )
            .await
            .is_err());
            crate::chat::storage::fail_write_under_for_test(None);
            assert!(bound_sessions(app).is_empty());
            assert!(
                std::fs::read_dir(data.join("external-agent-sessions"))
                    .unwrap()
                    .next()
                    .is_none(),
                "{label} left staging"
            );
        }
        let mut images = history();
        images[0].images = vec![
            crate::external_agents::import_history::ImportedImage {
                media_type: "image/png".into(),
                data_base64: "eA==".into(),
            },
            crate::external_agents::import_history::ImportedImage {
                media_type: "image/png".into(),
                data_base64: "invalid base64!".into(),
            },
        ];
        assert!(import_prepared(
            app,
            "proj_history",
            def,
            "image-failure",
            root.clone(),
            (images, None, None)
        )
        .await
        .is_err());
        assert!(bound_sessions(app).is_empty());
        let (first, duplicate) = tokio::join!(
            import_prepared(
                app,
                "proj_history",
                def,
                "native-source",
                root.clone(),
                (history(), None, None)
            ),
            import_prepared(
                app,
                "proj_history",
                def,
                "native-source",
                root.clone(),
                (history(), None, None)
            )
        );
        let source_id = first.unwrap();
        assert_eq!(duplicate.unwrap(), source_id);
        assert_eq!(
            import_one_session(app, "proj_history", "claude", "native-source")
                .await
                .unwrap(),
            source_id
        );
        assert_eq!(bound_sessions(app).len(), 1);
        assert_eq!(
            load_import_record(app, &source_id).unwrap().original_cwd,
            Some(root.clone())
        );

        crate::chat::storage::fail_write_under_for_test(Some((&data, "index")));
        let error = import_prepared(
            app,
            "proj_history",
            def,
            "index-failure",
            root.clone(),
            (history(), None, None),
        )
        .await
        .unwrap_err();
        assert!(error.contains("committed"), "{error}");
        let committed_id = bound_sessions(app)[&("claude".into(), "index-failure".into())]
            .conversation_id
            .clone();
        assert!(crate::external_agents::session::load_session(app, &committed_id).is_some());
        assert_eq!(
            repository(app)
                .get(app, &committed_id)
                .await
                .unwrap()
                .messages
                .len(),
            2
        );
        crate::chat::storage::fail_write_under_for_test(None);
        assert_eq!(
            import_prepared(
                app,
                "proj_history",
                def,
                "index-failure",
                root.clone(),
                (history(), None, None)
            )
            .await
            .unwrap(),
            committed_id
        );
        assert!(crate::chat::storage::load_index(app)
            .unwrap()
            .conversations
            .iter()
            .any(|item| item.id == committed_id));

        let orphan = "conv_orphan_import";
        save_import_record(
            app,
            &ImportRecord {
                conversation_id: orphan.into(),
                agent_id: "claude".into(),
                session_id: "crash-stage".into(),
                original_cwd: Some(root.clone()),
                source_path: None,
                source_mtime_ms: None,
                message_count: 0,
                imported_at: 1,
            },
        )
        .unwrap();
        let orphan_dir = crate::chat::storage::conversation_attachments_dir(app, orphan).unwrap();
        std::fs::write(orphan_dir.join("partial.png"), "partial").unwrap();
        import_prepared(
            app,
            "proj_history",
            def,
            "crash-stage",
            root.clone(),
            (history(), None, None),
        )
        .await
        .unwrap();
        assert!(!orphan_dir.exists());
        assert!(load_import_record(app, orphan).is_none());

        let empty_id = import_prepared(
            app,
            "proj_history",
            def,
            "empty-native",
            root.clone(),
            (vec![], None, None),
        )
        .await
        .unwrap();
        assert!(
            repository(app)
                .get(app, &empty_id)
                .await
                .unwrap()
                .runtime_locked
        );
        assert!(crate::external_agents::commands::set_agent_runtime(
            app,
            &empty_id,
            AgentRuntimeConfig::default()
        )
        .await
        .is_err());
        assert!(
            crate::external_agents::workspace::validate_working_directory_override(
                app,
                &empty_id,
                directory.path()
            )
            .is_err()
        );
        assert!(
            crate::external_agents::workspace::validate_working_directory_override(
                app, &empty_id, &workspace
            )
            .is_ok()
        );
        for bad_root in [
            directory.path().to_string_lossy().into_owned(),
            workspace.join("missing").to_string_lossy().into_owned(),
        ] {
            assert!(import_prepared(
                app,
                "proj_history",
                def,
                "wrong-root",
                bad_root,
                (history(), None, None)
            )
            .await
            .is_err());
            assert!(!bound_sessions(app).contains_key(&("claude".into(), "wrong-root".into())));
        }
        let source_attachments =
            crate::chat::storage::conversation_attachments_dir(app, &source_id).unwrap();
        std::fs::write(source_attachments.join("a.png"), "image fixture").unwrap();
        repository(app)
            .mutate(app, &source_id, |latest| {
                latest.messages[0]
                    .attachments
                    .push(crate::chat::Attachment {
                        id: "attachment".into(),
                        attachment_type: "image".into(),
                        name: "a.png".into(),
                        path: "a.png".into(),
                        content: None,
                    });
                Ok(())
            })
            .await
            .unwrap();
        let attachment_dirs = || {
            std::fs::read_dir(data.join("conversations"))
                .unwrap()
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .count()
        };
        let before_dirs = attachment_dirs();
        crate::chat::storage::fail_write_under_for_test(Some((&data, "conversation")));
        assert!(crate::chat::commands::mutations::chat_fork_conversation(
            app.clone(),
            app.state(),
            source_id.clone(),
            "answer_a".into(),
            None
        )
        .await
        .is_err());
        crate::chat::storage::fail_write_under_for_test(None);
        assert_eq!(attachment_dirs(), before_dirs);
        let before = repository(app).get(app, &source_id).await.unwrap();
        let source_bytes =
            std::fs::read(crate::chat::storage::conversation_file_path(app, &source_id).unwrap())
                .unwrap();
        let binding_bytes = std::fs::read(
            data.join("external-agent-sessions")
                .join(format!("{source_id}.json")),
        )
        .unwrap();
        let branch = crate::chat::commands::mutations::chat_fork_conversation(
            app.clone(),
            app.state(),
            source_id.clone(),
            "answer_a".into(),
            None,
        )
        .await
        .unwrap();
        let branch_id = branch["conversation"]["id"].as_str().unwrap();
        let mut conversation = repository(app).get(app, branch_id).await.unwrap();
        assert!(crate::external_agents::session::load_session(app, branch_id).is_none());
        assert_eq!(
            std::fs::read(
                crate::chat::storage::conversation_attachments_dir(app, branch_id)
                    .unwrap()
                    .join("a.png")
            )
            .unwrap(),
            b"image fixture"
        );
        assert_eq!(conversation.external_cwd, Some(root.clone()));
        let transition = conversation.external_history.clone().unwrap();
        assert_eq!(transition.method, ExternalHistoryMethod::Replay);
        assert_eq!(transition.prefix(&conversation).unwrap().len(), 2);
        assert_eq!(
            std::fs::read(crate::chat::storage::conversation_file_path(app, &source_id).unwrap())
                .unwrap(),
            source_bytes
        );
        assert_eq!(
            std::fs::read(
                data.join("external-agent-sessions")
                    .join(format!("{source_id}.json"))
            )
            .unwrap(),
            binding_bytes
        );
        let regenerate_branch = crate::chat::commands::mutations::chat_fork_conversation(
            app.clone(),
            app.state(),
            source_id.clone(),
            "answer_a".into(),
            None,
        )
        .await
        .unwrap();
        let regenerate_id = regenerate_branch["conversation"]["id"].as_str().unwrap();
        let regenerated = repository(app)
            .mutate(app, regenerate_id, |latest| {
                crate::chat::commands::mutations::apply_regenerate_truncation(latest, 1, None)
            })
            .await
            .unwrap();
        let regenerated = repository(app).get(app, &regenerated.id).await.unwrap();
        let regenerated_history = regenerated.external_history.as_ref().unwrap();
        assert_eq!(regenerated_history.committed_revision, regenerated.revision);
        assert!(regenerated_history.prefix(&regenerated).unwrap().is_empty());
        let prompt = crate::external_agents::prompt::compose_history_handoff(
            &regenerated,
            crate::external_agents::prompt::compose_external_prompt_passthrough("prefix A"),
            "prefix A",
        )
        .unwrap();
        assert_eq!(prompt.full_prompt.matches("prefix A").count(), 1);
        assert!(!prompt.full_prompt.contains("answer A"));

        let state = app.state::<crate::state::AppState>();
        let generation = state.chat_runtime().begin_generation(branch_id);
        repository(app)
            .append_message(app, branch_id, message("user_b", "user", "request B"))
            .await
            .unwrap();
        conversation = repository(app).get(app, branch_id).await.unwrap();
        let mut dispatched = None;
        dispatch(
            app,
            &conversation,
            generation,
            "independent-native",
            &mut dispatched,
            "request B",
            &["prefix A", "answer A", "request B"],
        )
        .await;
        let attempt = dispatched.unwrap();
        assert!(begin_history_delivery(
            app,
            branch_id,
            &transition.id,
            generation,
            "independent-native"
        )
        .await
        .is_err());
        crate::chat::storage::fail_write_under_for_test(Some((&data, "conversation")));
        assert!(save_answer(
            app,
            &mut conversation,
            "answer_failed",
            "completed",
            &attempt
        )
        .await
        .is_err());
        crate::chat::storage::fail_write_under_for_test(None);
        assert!(repository(app)
            .get(app, branch_id)
            .await
            .unwrap()
            .external_history
            .is_some());
        state.chat_runtime().end_generation(branch_id, generation);
        save_answer(
            app,
            &mut conversation,
            "answer_cancelled",
            "cancelled",
            &attempt,
        )
        .await
        .unwrap();
        conversation = repository(app).get(app, branch_id).await.unwrap();
        assert!(conversation
            .messages
            .iter()
            .any(|message| message.id == "answer_cancelled"));
        assert!(conversation.external_history.is_some());
        repository(app)
            .append_message(app, branch_id, message("user_c", "user", "continue C"))
            .await
            .unwrap();
        conversation = repository(app).get(app, branch_id).await.unwrap();
        let recovery_generation = state.chat_runtime().begin_generation(branch_id);
        let before_slash = conversation.external_history.clone();
        assert!(crate::external_agents::run::prepare_history_transition(
            app,
            &mut conversation,
            recovery_generation,
            true
        )
        .await
        .unwrap()
        .is_none());
        assert_eq!(
            repository(app)
                .get(app, branch_id)
                .await
                .unwrap()
                .external_history,
            before_slash
        );
        crate::external_agents::run::prepare_history_transition(
            app,
            &mut conversation,
            recovery_generation,
            false,
        )
        .await
        .unwrap();
        let mut recovered_attempt = None;
        dispatch(
            app,
            &conversation,
            recovery_generation,
            "recovery-native",
            &mut recovered_attempt,
            "continue C",
            &["prefix A", "request B", "collected answer", "continue C"],
        )
        .await;
        state
            .chat_runtime()
            .end_generation(branch_id, recovery_generation);
        let newer = repository(app)
            .mutate(app, branch_id, |latest| {
                latest.external_history = Some(ExternalHistoryTransition::new(
                    latest,
                    ExternalHistoryMethod::Replay,
                ));
                Ok(())
            })
            .await
            .unwrap();
        repository(app)
            .mutate(app, branch_id, |latest| {
                settle_history_delivery(latest, &attempt);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            repository(app)
                .get(app, branch_id)
                .await
                .unwrap()
                .external_history,
            newer.external_history
        );
        let generation = state.chat_runtime().begin_generation(branch_id);
        let new_attempt = begin_history_delivery(
            app,
            branch_id,
            &newer.external_history.unwrap().id,
            generation,
            "fresh-native",
        )
        .await
        .unwrap();
        save_answer(
            app,
            &mut conversation,
            "answer_success",
            "completed",
            &new_attempt,
        )
        .await
        .unwrap();
        conversation = repository(app).get(app, branch_id).await.unwrap();
        assert!(conversation.external_history.is_none());
        dispatch(
            app,
            &conversation,
            generation,
            "fresh-native",
            &mut None,
            "second successful request",
            &["second successful request"],
        )
        .await;
        assert_eq!(
            repository(app).get(app, &source_id).await.unwrap().revision,
            before.revision
        );
        state.chat_runtime().end_generation(branch_id, generation);

        let current = repository(app).get(app, &source_id).await.unwrap();
        assert!(repository(app)
            .mutate_expected(app, &source_id, Some(current.revision - 1), |latest| {
                latest.messages.clear();
                latest.external_history = Some(ExternalHistoryTransition::new(
                    latest,
                    ExternalHistoryMethod::NativeRewind,
                ));
                Ok(())
            })
            .await
            .is_err());
        assert!(repository(app)
            .get(app, &source_id)
            .await
            .unwrap()
            .external_history
            .is_none());
        crate::chat::commands::mutations::chat_rewind_to_message(
            app.clone(),
            app.state(),
            source_id.clone(),
            "user_a".into(),
        )
        .await
        .unwrap();
        let rewound = repository(app).get(app, &source_id).await.unwrap();
        assert!(rewound.messages.is_empty());
        assert_eq!(
            rewound
                .external_history
                .as_ref()
                .unwrap()
                .committed_revision,
            rewound.revision
        );
        assert!(rewound.runtime_locked);
        assert!(crate::external_agents::commands::set_agent_runtime(
            app,
            &source_id,
            AgentRuntimeConfig::default()
        )
        .await
        .is_err());
        crate::external_agents::session::forget_native_session(app, &source_id).unwrap();
        assert_eq!(
            bound_sessions(app)[&("claude".into(), "native-source".into())].conversation_id,
            source_id
        );
        assert!(
            crate::external_agents::workspace::validate_working_directory_override(
                app,
                &source_id,
                directory.path()
            )
            .is_err()
        );

        let marker = data
            .join("external-agent-sessions")
            .join(format!("rewound-{empty_id}.marker"));
        std::fs::write(&marker, "").unwrap();
        repository(app)
            .append_message(
                app,
                &empty_id,
                message("legacy_user", "user", "legacy new request"),
            )
            .await
            .unwrap();
        let mut legacy = repository(app).get(app, &empty_id).await.unwrap();
        let generation = state.chat_runtime().begin_generation(&empty_id);
        crate::chat::storage::fail_write_under_for_test(Some((&data, "index")));
        assert!(crate::external_agents::run::prepare_history_transition(
            app,
            &mut legacy,
            generation,
            false
        )
        .await
        .is_err());
        crate::chat::storage::fail_write_under_for_test(None);
        legacy = repository(app).get(app, &empty_id).await.unwrap();
        assert!(legacy.external_history.is_some());
        crate::external_agents::run::prepare_history_transition(
            app,
            &mut legacy,
            generation,
            false,
        )
        .await
        .unwrap();
        assert!(!marker.exists());
        let mut legacy_attempt = None;
        dispatch(
            app,
            &legacy,
            generation,
            "legacy-fresh",
            &mut legacy_attempt,
            "legacy new request",
            &["legacy new request"],
        )
        .await;
        save_answer(
            app,
            &mut legacy,
            "legacy_answer",
            "completed",
            &legacy_attempt.unwrap(),
        )
        .await
        .unwrap();
        assert!(crate::external_agents::run::prepare_history_transition(
            app,
            &mut legacy,
            generation,
            false
        )
        .await
        .unwrap()
        .is_none());
        legacy = repository(app).mutate(app, &empty_id, |latest| {
            let mut history = ExternalHistoryTransition::new(latest, ExternalHistoryMethod::Replay);
            history.through_message_id = Some("legacy_answer".into());
            latest.messages.retain(|message| message.id != "legacy_answer");
            latest.messages.push(message("repair_user", "user", "repair request"));
            latest.external_history = Some(history);
            Ok(())
        }).await.unwrap();
        let stale_transition = legacy.external_history.as_ref().unwrap().id.clone();
        let repaired = crate::external_agents::run::prepare_history_transition(
            app, &mut legacy, generation, false,
        ).await.unwrap().unwrap();
        assert_ne!(repaired.id, stale_transition);
        assert_eq!(repaired.method, ExternalHistoryMethod::Replay);
        assert!(repaired.prefix(&legacy).unwrap().iter().all(|message| message.id != "repair_user"));
        let handoff = crate::external_agents::prompt::compose_history_handoff(
            &legacy,
            crate::external_agents::prompt::compose_external_prompt_passthrough("repair request"),
            "repair request",
        ).unwrap();
        assert_eq!(handoff.full_prompt.matches("repair request").count(), 1);
        state.chat_runtime().end_generation(&empty_id, generation);
    });
}
