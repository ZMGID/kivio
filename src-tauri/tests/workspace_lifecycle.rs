//! Real directory lifecycle through the production Tauri/storage boundaries.
//! A separate test application owns disposable data; no user settings are loaded.
use kivio::{
    chat::{repository::ConversationRepository, storage},
    external_agents::{session, workspace},
};
use std::fs;
use tauri::Manager;

fn main() {
    let data = directories::BaseDirs::new()
        .unwrap()
        .data_dir()
        .to_path_buf();
    let isolated = tempfile::Builder::new()
        .prefix("kivio-workspace-test-")
        .tempdir_in(data)
        .unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = isolated
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .into();
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(ConversationRepository::default())
        .build(context)
        .expect("isolated application");
    let handle = app.handle();
    assert_eq!(handle.path().app_data_dir().unwrap(), isolated.path());
    let old = isolated.path().join("old");
    let new = isolated.path().join("new");
    fs::create_dir(&old).unwrap();
    fs::create_dir(&new).unwrap();
    // Pre-create settings before load_settings so legacy application migration cannot run.
    let mut settings = kivio::settings::Settings::default();
    settings.chat_tools.native_tools.working_directory = new.display().to_string();
    fs::write(
        isolated.path().join("settings.json"),
        serde_json::to_vec(&serde_json::json!({"settings": settings})).unwrap(),
    )
    .unwrap();
    let conversations = storage::conversations_dir(handle).unwrap();
    let bound = format!("conv_{}", uuid::Uuid::new_v4());
    let builtin = format!("conv_{}", uuid::Uuid::new_v4());
    for (id, external) in [(&bound, true), (&builtin, false)] {
        fs::create_dir(old.join(id)).unwrap();
        fs::write(old.join(id).join("report.txt"), id).unwrap();
        let mut conversation = serde_json::json!({
            "id": id, "title": "Directory lifecycle", "provider_id": "test", "model": "test",
            "created_at": 1, "updated_at": 1, "messages": []
        });
        if external {
            conversation["agent_runtime"] =
                serde_json::json!({"kind":"external", "externalAgentId":"codex"});
        }
        fs::write(
            conversations.join(format!("{id}.json")),
            serde_json::to_vec(&conversation).unwrap(),
        )
        .unwrap();
    }
    session::save_live_handle(
        handle,
        &bound,
        &session::LiveSessionHandle {
            agent_id: "codex".into(),
            protocol: "codex_app_server".into(),
            native_id: "native-test".into(),
            native_path: None,
            cwd: old.join(&bound).display().to_string(),
        },
    )
    .unwrap();

    let mut failures = Vec::new();
    let actual = workspace::resolve_effective_cwd(handle, &bound, None).unwrap();
    let detection = workspace::resolve_detection_cwd(handle, Some(&bound)).unwrap();
    if actual != detection {
        failures.push(format!(
            "probe directory {detection:?} differs from execution {actual:?}"
        ));
    }
    assert_eq!(workspace::resolve_detection_cwd(handle, None).unwrap(), new);

    tauri::async_runtime::block_on(storage::migrate_ordinary_conversation_workspaces(
        handle,
        old.to_str().unwrap(),
        new.to_str().unwrap(),
    ))
    .unwrap();
    if fs::read_to_string(actual.join("report.txt"))
        .ok()
        .as_deref()
        != Some(bound.as_str())
    {
        failures.push("migration moved files away from the native session directory".into());
    }
    assert_eq!(
        fs::read_to_string(new.join(&builtin).join("report.txt")).unwrap(),
        builtin
    );
    assert_eq!(
        session::load_live_handle(handle, &bound).unwrap().cwd,
        actual.display().to_string()
    );
    if new.join(&bound).exists() {
        failures.push("migration created a second directory for the bound CLI session".into());
    }
    assert_eq!(
        workspace::resolve_detection_cwd(handle, Some("conv_not_saved")).unwrap(),
        new
    );
    assert!(failures.is_empty(), "{}", failures.join("; "));
    println!("PASS: native binding and files retained; detection matches execution; builtin migration and draft default preserved");
}
