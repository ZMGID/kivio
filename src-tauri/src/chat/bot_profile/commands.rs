//! Tauri commands. Parent registers them in `lib.rs`; this file does not.

use tauri::{AppHandle, State};

use crate::state::AppState;

use super::lifecycle;
use super::types::{
    BotCreateRequest, BotEditRequest, BotListItem, BotMemoryView, BotProfile, BotProfileDetail,
};

#[tauri::command]
pub async fn chat_create_bot(
    app: AppHandle,
    state: State<'_, AppState>,
    request: BotCreateRequest,
) -> Result<BotProfileDetail, String> {
    lifecycle::create_bot(&app, state.inner(), request).await
}

#[tauri::command]
pub async fn chat_update_bot(
    app: AppHandle,
    state: State<'_, AppState>,
    request: BotEditRequest,
) -> Result<BotProfileDetail, String> {
    lifecycle::update_bot(&app, state.inner(), request).await
}

#[tauri::command]
pub fn chat_get_bot_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    assistant_id: String,
) -> Result<BotProfileDetail, String> {
    let settings = state.settings_read().clone();
    lifecycle::profile_detail(&app, &settings, &assistant_id)
}

#[tauri::command]
pub async fn chat_list_bots(app: AppHandle) -> Result<Vec<BotListItem>, String> {
    lifecycle::list_bots(&app).await
}

#[tauri::command]
pub async fn chat_set_primary_bot(
    app: AppHandle,
    assistant_id: String,
    primary: bool,
) -> Result<BotProfile, String> {
    lifecycle::set_primary_bot(&app, &assistant_id, primary).await
}

#[tauri::command]
pub fn chat_get_bot_memory(app: AppHandle, assistant_id: String) -> Result<BotMemoryView, String> {
    lifecycle::memory_view(&app, &assistant_id)
}

#[tauri::command]
pub fn chat_save_bot_memory(
    app: AppHandle,
    assistant_id: String,
    layer: String,
    content: String,
) -> Result<BotMemoryView, String> {
    lifecycle::save_bot_memory(&app, &assistant_id, &layer, &content)
}

#[tauri::command]
pub fn chat_clear_bot_memory(
    app: AppHandle,
    assistant_id: String,
    layer: String,
) -> Result<BotMemoryView, String> {
    lifecycle::clear_bot_memory(&app, &assistant_id, &layer)
}

#[tauri::command]
pub async fn chat_add_bot_shared_memory(
    app: AppHandle,
    assistant_id: String,
    content: String,
) -> Result<BotProfile, String> {
    lifecycle::add_shared_memory(&app, &assistant_id, &content).await
}

#[tauri::command]
pub async fn chat_delete_bot_shared_memory(
    app: AppHandle,
    assistant_id: String,
    note_id: String,
) -> Result<BotProfile, String> {
    lifecycle::delete_shared_memory(&app, &assistant_id, &note_id).await
}
