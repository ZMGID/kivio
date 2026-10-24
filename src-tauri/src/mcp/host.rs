//! Tauri event adapter for the shared MCP connection pool.
use tauri::Emitter;
pub type EventSink = tauri::AppHandle;
pub fn emit(sink: &EventSink, name: &str, value: serde_json::Value) {
    let _ = sink.emit(name, value);
}
