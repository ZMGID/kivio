//! Native backup boundary: staging must not mutate a live installation.
use kivio_model_runtime::{
    backup,
    settings::{Settings, sanitize_settings},
};
use serde_json::json;
#[test]
fn native_backup_stages_verifies_and_swaps_before_stores_open() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let work = temp.path().join("workspace");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let mut settings = Settings::default();
    settings.chat_tools.native_tools.working_directory = work.to_string_lossy().into();
    settings
        .chat_tools
        .hooks
        .push(kivio_model_runtime::tool_settings::HookDef {
            id: "foreign".into(),
            name: "foreign".into(),
            enabled: true,
            event: "agent_start".into(),
            script: "echo never run during restore".into(),
            ..Default::default()
        });
    std::fs::write(
        source.join("settings.json"),
        serde_json::to_vec(&json!({"settings":settings})).unwrap(),
    )
    .unwrap();
    std::fs::write(source.join("data.txt"), "saved data").unwrap();
    std::fs::write(work.join("attachment.txt"), "attachment bytes").unwrap();
    let archive = temp.path().join("backup.kivio");
    backup::export_to(&source, &work, &archive, &Default::default()).unwrap();
    let target = temp.path().join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("data.txt"), "current data").unwrap();
    let bad = temp.path().join("broken.kivio");
    std::fs::write(&bad, b"not a backup").unwrap();
    assert!(backup::stage_restore(&target, &bad, false, &settings, sanitize_settings).is_err());
    assert!(!backup::restore_dir(&target).exists());
    backup::stage_restore(&target, &archive, false, &settings, sanitize_settings).unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("data.txt")).unwrap(),
        "current data"
    );
    assert!(backup::stage_restore(&target, &archive, false, &settings, sanitize_settings).is_err());
    backup::apply_pending(&target).unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("data.txt")).unwrap(),
        "saved data"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("target.before-restore/data.txt")).unwrap(),
        "current data"
    );
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(target.join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        document["settings"]["chatTools"]["hooks"][0]["enabled"],
        false
    );
    let restored: Settings = serde_json::from_value(document["settings"].clone()).unwrap();
    assert_eq!(
        std::fs::read_to_string(
            std::path::Path::new(&restored.chat_tools.native_tools.working_directory)
                .join("attachment.txt")
        )
        .unwrap(),
        "attachment bytes"
    );
    backup::apply_pending(&target).unwrap();
    assert!(!backup::restore_dir(&target).exists());
}

#[test]
fn configuration_restore_keeps_live_files_until_startup_and_preserves_other_sets() {
    use kivio_model_runtime::native_backup;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("profile");
    std::fs::create_dir_all(root.join("conversations")).unwrap();
    std::fs::write(
        root.join("settings.json"),
        br#"{"settings":{"chat":{"userDisplayName":"before"}},"outerMetadata":42}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("conversations/sets.json"),
        br#"{"sets":[{"id":"set_local","name":"Keep me","created_at":1,"updated_at":1}]}"#,
    )
    .unwrap();
    let archive = temp.path().join("configuration.json");
    native_backup::export_configuration(&root, &archive).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&archive).unwrap()).unwrap();
    value["settings"]["chat"]["userDisplayName"] = json!("restored");
    value["sets"]["sets"] =
        json!([{"id":"set_imported","name":"Imported","created_at":1,"updated_at":1}]);
    std::fs::write(&archive, serde_json::to_vec(&value).unwrap()).unwrap();
    native_backup::stage_configuration(&root, &archive, false).unwrap();
    assert!(native_backup::pending(&root));
    assert_eq!(
        kivio_model_runtime::native_preferences::load(&root)
            .unwrap()
            .chat
            .user_display_name,
        "before"
    );
    native_backup::apply_pending(&root).unwrap();
    assert_eq!(
        kivio_model_runtime::native_preferences::load(&root)
            .unwrap()
            .chat
            .user_display_name,
        "restored"
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(doc["outerMetadata"], 42);
    let sets =
        kivio_model_runtime::group_catalog::load_set_index(&root.join("conversations")).unwrap();
    assert_eq!(sets.sets.len(), 2);
    assert!(sets.sets.iter().any(|set| set.id == "set_local"));
    assert!(!native_backup::pending(&root));
    value["settings"]["chat"]["userDisplayName"] = json!("second restore");
    std::fs::write(&archive, serde_json::to_vec(&value).unwrap()).unwrap();
    native_backup::stage_configuration(&root, &archive, false).unwrap();
    native_backup::apply_pending(&root).unwrap();
    let recoveries: Vec<serde_json::Value> = std::fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("configuration-before-restore-")
                .then(|| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        })
        .collect();
    assert_eq!(recoveries.len(), 2);
    assert!(
        recoveries
            .iter()
            .any(|doc| doc["settings.json"]["settings"]["chat"]["userDisplayName"] == "restored")
    );
    value["sets"]["sets"][0]["id"] = json!("../../unsafe");
    std::fs::write(&archive, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(native_backup::stage_configuration(&root, &archive, false).is_err());
    assert!(!native_backup::pending(&root));
}
