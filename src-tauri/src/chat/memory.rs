pub use super::memory_files::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;

use crate::mcp::types::McpToolCallResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMemoryState {
    pub success: bool,
    pub l1: MemoryLayerContent,
    pub l2: MemoryLayerContent,
    pub dir: String,
}

/// Resolve (and create) the `chat-memory` dir under an explicit app-data base.
/// This is the AppHandle-free core so non-Tauri callers (e.g. the kivio-code
/// terminal agent, which has no `AppHandle`) can read/write the SAME memory the
/// GUI writes, by passing their own resolved app-data dir.

pub fn memory_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?;
    memory_dir_at(&base)
}

pub fn read_layer(app: &AppHandle, layer: MemoryLayer) -> Result<MemoryLayerContent, String> {
    read_layer_at(&memory_dir(app)?, layer)
}

pub fn read_all(app: &AppHandle) -> Result<ChatMemoryState, String> {
    let dir = memory_dir(app)?;
    Ok(ChatMemoryState {
        success: true,
        l1: read_layer_at(&dir, MemoryLayer::L1)?,
        l2: read_layer_at(&dir, MemoryLayer::L2)?,
        dir: dir.display().to_string(),
    })
}

pub fn save_layer(
    app: &AppHandle,
    layer: MemoryLayer,
    content: &str,
) -> Result<MemoryLayerContent, String> {
    save_layer_at(&memory_dir(app)?, layer, content)
}

pub fn l1_prompt_block(app: &AppHandle) -> Result<Option<String>, String> {
    l1_prompt_block_at(&memory_dir(app)?)
}

#[tauri::command]
pub(crate) fn chat_memory_get(app: AppHandle) -> Result<ChatMemoryState, String> {
    read_all(&app)
}

#[tauri::command]
pub(crate) fn chat_memory_save(
    app: AppHandle,
    layer: String,
    content: String,
) -> Result<MemoryLayerContent, String> {
    save_layer(&app, MemoryLayer::from_str(&layer)?, &content)
}

#[tauri::command]
#[allow(deprecated)]
pub(crate) fn chat_memory_open_folder(app: AppHandle) -> Result<serde_json::Value, String> {
    let dir = memory_dir(&app)?;
    let dir_str = dir.display().to_string();
    app.shell()
        .open(dir_str.clone(), None)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "success": true,
        "path": dir_str,
    }))
}

pub fn tool_read(app: &AppHandle, arguments: &Value) -> Result<McpToolCallResult, String> {
    tool_read_at(&memory_dir(app)?, arguments)
}


pub fn tool_search(app: &AppHandle, arguments: &Value) -> Result<McpToolCallResult, String> {
    tool_search_at(&memory_dir(app)?, arguments)
}

pub fn tool_modify(app: &AppHandle, arguments: &Value) -> Result<McpToolCallResult, String> {
    tool_modify_at(&memory_dir(app)?, arguments)
}

pub use super::memory_tools::{MemoryModifyArgs, tool_read_at, tool_search_at, tool_modify_at};
