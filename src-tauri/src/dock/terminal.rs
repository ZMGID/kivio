//! Tauri transport adapter for the shared PTY session owner.
use std::sync::Arc;
use tauri::Emitter;
#[path = "terminal_runtime.rs"]
mod runtime;
use runtime::TerminalEvent;
pub use runtime::{TerminalCreateResult, TerminalService};
pub const TERMINAL_OUTPUT_EVENT: &str = "dock:terminal-output";
pub const TERMINAL_EXIT_EVENT: &str = "dock:terminal-exit";
pub fn new_service(app: tauri::AppHandle) -> TerminalService {
    TerminalService::new(move |event| {
        match event {
        TerminalEvent::Output { session_id, data } => app.emit(TERMINAL_OUTPUT_EVENT,
            serde_json::json!({"sessionId": session_id, "data": String::from_utf8_lossy(&data)})).map_err(|e| e.to_string()),
        TerminalEvent::Exit { session_id, code } => app.emit(TERMINAL_EXIT_EVENT,
            serde_json::json!({"sessionId": session_id, "code": code})).map_err(|e| e.to_string()),
    }
    })
}
#[tauri::command]
pub async fn dock_terminal_create(
    state: tauri::State<'_, Arc<TerminalService>>,
    workdir: String,
    cols: u16,
    rows: u16,
) -> Result<TerminalCreateResult, String> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        runtime::create_session(service, &workdir, cols, rows)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn dock_terminal_write(
    state: tauri::State<'_, Arc<TerminalService>>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.write(&session_id, data.as_bytes()))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn dock_terminal_resize(
    state: tauri::State<'_, Arc<TerminalService>>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.resize(&session_id, cols, rows))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn dock_terminal_close(
    state: tauri::State<'_, Arc<TerminalService>>,
    session_id: String,
) -> Result<(), String> {
    let service = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.close(&session_id))
        .await
        .map_err(|e| e.to_string())
}
