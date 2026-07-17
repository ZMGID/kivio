use tauri::AppHandle;

use crate::chat::storage::{load_conversation, save_conversation};
use crate::chat::types::AgentRuntimeConfig;
use crate::external_agents::detection::{detect_all_agents, EXTERNAL_AGENT_MODELS_CACHE_TTL};
use crate::external_agents::slash::{cache_key, list_external_cli_slash_commands};
use crate::external_agents::workspace::resolve_detection_cwd;
use crate::state::AppState;

#[tauri::command]
pub async fn chat_detect_external_agents(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    force_refresh: Option<bool>,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let force = force_refresh.unwrap_or(false);
    let cwd = resolve_detection_cwd(&app, conversation_id.as_deref())?;
    let cwd_key = cwd.to_string_lossy().into_owned();
    if !force {
        if let Some(agents) =
            state.get_cached_detected_agents(&cwd_key, EXTERNAL_AGENT_MODELS_CACHE_TTL)
        {
            return Ok(serde_json::json!({
                "success": true,
                "agents": agents,
                "cached": true,
            }));
        }
    }

    let agents = detect_all_agents(&cwd).await;
    for agent in &agents {
        if !agent.models.is_empty() {
            state.set_cached_external_agent_models(
                cache_key(&agent.id, &cwd_key),
                agent.models.clone(),
            );
        }
    }
    state.set_cached_detected_agents(cwd_key, agents.clone());
    Ok(serde_json::json!({
        "success": true,
        "agents": agents,
        "cached": false,
    }))
}

#[tauri::command]
pub async fn chat_list_external_cli_slash_commands(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    agent_id: String,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let (supports, commands, message) =
        list_external_cli_slash_commands(&app, &state, &agent_id, conversation_id.as_deref())
            .await?;
    Ok(serde_json::json!({
        "success": true,
        "supportsSlashCommands": supports,
        "commands": commands,
        "message": message,
    }))
}

#[tauri::command]
pub fn chat_set_agent_runtime(
    app: AppHandle,
    conversation_id: String,
    agent_runtime: AgentRuntimeConfig,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    conversation.agent_runtime = agent_runtime;
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}
