//! Library actions must commit through the same engine/store used by chat.
use kivio_model_runtime::{conversation_data::Conversation, conversation_files::ConversationStore};
use serde_json::json;
use zeron_ui::{EngineBootConfig, HarnessId, state::EngineHandle};

#[tokio::test]
async fn library_batch_preserves_partial_failures_and_group_membership() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("conversations");
    let store = ConversationStore::new(directory.clone());
    let mut first = Conversation::new("fixture".into(), "fixture".into());
    first.id = "conv_library_a".into();
    let first = store.create(first).unwrap();
    let mut second = Conversation::new("fixture".into(), "fixture".into());
    second.id = "conv_library_b".into();
    let second = store.create(second).unwrap();
    std::fs::write(
        directory.join("projects.json"),
        json!({"projects":[{"id":"proj_library","name":"Project","created_at":1,"updated_at":1}]})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        directory.join("sets.json"),
        json!({"sets":[{"id":"set_library","name":"Set","created_at":1,"updated_at":1}]})
            .to_string(),
    )
    .unwrap();
    let handle = EngineHandle::bootstrap(EngineBootConfig {
        data_dir: root.path().into(),
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: HarnessId::Codex,
    })
    .await
    .unwrap();
    let client = handle.client();
    let result = client.call("Mutate", json!({"op":"batchLibrary", "chatIds":[first.id,"conv_missing",second.id,first.id],"action":{"op":"moveChat","groupKind":"project","groupId":"proj_library"}})).await.unwrap();
    assert_eq!(result["succeeded"], json!([first.id, second.id]));
    assert_eq!(result["failed"].as_array().unwrap().len(), 1);
    assert_eq!(result["failed"][0]["id"], "conv_missing");
    let moved = store.load(&first.id).unwrap();
    assert_eq!(
        moved.revision,
        first.revision + 1,
        "duplicate selection must mutate once"
    );
    assert_eq!(moved.project_id.as_deref(), Some("proj_library"));
    assert_eq!(moved.folder.as_deref(), Some("Project"));
    assert_eq!(moved.set_id, None);
    client
        .call(
            "Mutate",
            json!({"op":"moveChat","chatId":first.id,"groupKind":"set","groupId":"set_library"}),
        )
        .await
        .unwrap();
    let moved = store.load(&first.id).unwrap();
    assert_eq!(moved.set_id.as_deref(), Some("set_library"));
    assert_eq!(moved.project_id, None);
    assert_eq!(moved.folder, None);
    let revision = moved.revision;
    assert!(
        client
            .call(
                "Mutate",
                json!({"op":"moveChat","chatId":first.id,"groupKind":"set","groupId":"set_missing"})
            )
            .await
            .is_err()
    );
    assert_eq!(store.load(&first.id).unwrap().revision, revision);
    client.call("Mutate", json!({"op":"batchLibrary","chatIds":[first.id,second.id],"action":{"op":"setChatPinned","pinned":true}})).await.unwrap();
    assert!(store.load(&first.id).unwrap().pinned && store.load(&second.id).unwrap().pinned);
    client.call("Mutate", json!({"op":"batchLibrary","chatIds":[first.id,second.id],"action":{"op":"setChatArchived","archived":true}})).await.unwrap();
    assert!(store.load(&first.id).unwrap().archived && store.load(&second.id).unwrap().archived);
    client
        .call(
            "Mutate",
            json!({"op":"moveChat","chatId":first.id,"groupKind":"none"}),
        )
        .await
        .unwrap();
    assert_eq!(store.load(&first.id).unwrap().set_id, None);
    let result = client.call("Mutate", json!({"op":"batchLibrary","chatIds":[first.id,"conv_missing"],"action":{"op":"deleteChat"}})).await.unwrap();
    assert_eq!(result["succeeded"], json!([first.id]));
    assert!(store.load(&first.id).is_err());
    assert!(store.load(&second.id).is_ok());
    handle.shutdown().await;
}

#[tokio::test]
async fn library_set_backup_restores_attachments_to_new_identities_and_rejects_damage() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("conversations");
    let store = ConversationStore::new(directory.clone());
    let mut conversation = Conversation::new("fixture".into(), "fixture".into());
    conversation.id = "conv_set_source".into();
    conversation.set_id = Some("set_source".into());
    conversation.archived = true;
    conversation.messages.push(serde_json::from_value(json!({
        "id":"message_source", "role":"user", "content":"keep content", "timestamp":1,
        "attachments":[{"id":"attachment_source","type":"file","name":"note.txt","path":"note.txt"}]
    })).unwrap());
    store.create(conversation).unwrap();
    std::fs::create_dir_all(directory.join("conv_set_source_attachments")).unwrap();
    std::fs::write(
        directory.join("conv_set_source_attachments/note.txt"),
        b"portable attachment",
    )
    .unwrap();
    std::fs::write(directory.join("sets.json"), json!({"sets":[{"id":"set_source","name":"Portable","system_prompt":"keep system","created_at":1,"updated_at":1}]}).to_string()).unwrap();
    std::fs::write(
        directory.join("conversation-pins.json"),
        json!({"set_source":[{"id":"conv_set_source","row":3}]}).to_string(),
    )
    .unwrap();
    let handle = EngineHandle::bootstrap(EngineBootConfig {
        data_dir: root.path().into(),
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: HarnessId::Codex,
    })
    .await
    .unwrap();
    let client = handle.client();
    let path = root.path().join("portable.json");
    client
        .call("KivioExportSet", json!({"setId":"set_source", "path":path}))
        .await
        .unwrap();
    let imported = client
        .call("KivioImportSet", json!({"path":path}))
        .await
        .unwrap();
    let imported_id = imported["id"].as_str().unwrap();
    assert_ne!(imported_id, "set_source");
    assert_eq!(imported["system_prompt"], "keep system");
    let all = store.load_all().unwrap();
    assert_eq!(all.len(), 2);
    let restored = all
        .iter()
        .find(|c| c.set_id.as_deref() == Some(imported_id))
        .unwrap();
    assert_ne!(restored.id, "conv_set_source");
    assert!(restored.archived);
    assert_eq!(restored.messages[0].content, "keep content");
    assert_eq!(
        std::fs::read(directory.join(format!("{}_attachments/note.txt", restored.id))).unwrap(),
        b"portable attachment"
    );
    let catalog = kivio_model_runtime::group_catalog::load_set_index(&directory).unwrap();
    assert_eq!(
        catalog.sets[0].id, imported_id,
        "new set precedes original manual order"
    );
    assert_eq!(catalog.sets[1].id, "set_source");
    let pins = kivio_model_runtime::group_catalog::load_conversation_pins(&directory).unwrap();
    assert_eq!(pins[imported_id][0].id, restored.id);
    assert_eq!(pins[imported_id][0].row, 3);
    let baseline = std::fs::read(directory.join("sets.json")).unwrap();
    let mut broken: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    broken["files"][0]["data"] = json!("invalid base64!");
    std::fs::write(&path, broken.to_string()).unwrap();
    assert!(
        client
            .call("KivioImportSet", json!({"path":path}))
            .await
            .is_err()
    );
    assert_eq!(store.load_all().unwrap().len(), 2);
    assert_eq!(
        std::fs::read(directory.join("sets.json")).unwrap(),
        baseline
    );
    handle.shutdown().await;
}
