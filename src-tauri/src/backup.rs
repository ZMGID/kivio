//! Tauri host for the shared backup transaction.
pub mod webdav;
pub mod core;
pub mod configuration;
use core::*;
use crate::{settings::Settings, state::AppState};
use std::{collections::BTreeMap, path::{Path, PathBuf}};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
pub(crate) use core::apply_pending;

#[tauri::command]
pub(crate) async fn export_full_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    interface: BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let root = app.path().app_data_dir().map_err(error)?;
    crate::provider_oauth::export_credentials(&state.settings_read())?;
    let workspace = PathBuf::from(
        &state
            .settings_read()
            .chat_tools
            .native_tools
            .working_directory,
    );
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = OPERATION.try_lock().map_err(|_| "备份或恢复正在进行")?;
        export_to(&root, &workspace, Path::new(&path), &interface)
    })
    .await
    .map_err(error)?
}
#[tauri::command]
pub(crate) async fn import_full_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    trusted: bool,
) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(error)?;
    let local = state.settings_read().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = OPERATION.try_lock().map_err(|_| "备份或恢复正在进行")?;
        stage_restore(&root, Path::new(&path), trusted, &local, crate::settings::sanitize_settings)
    })
    .await
    .map_err(error)?
}
#[tauri::command]
pub(crate) fn backup_interface(app: AppHandle) -> Result<Option<Value>, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(error)?
        .join(".backup-interface.json");
    if path.exists() {
        read_json(&path).map(Some)
    } else {
        Ok(None)
    }
}
#[tauri::command]
pub(crate) fn backup_restore_pending(app: AppHandle) -> Result<bool, String> {
    Ok(restore_dir(&app.path().app_data_dir().map_err(error)?)
        .join("ready.json")
        .exists())
}
#[tauri::command]
pub(crate) fn restart_after_restore(app: AppHandle) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(error)?;
    if !restore_dir(&root).join("ready.json").exists() {
        return Err("没有待恢复备份".into());
    }
    app.restart();
}
