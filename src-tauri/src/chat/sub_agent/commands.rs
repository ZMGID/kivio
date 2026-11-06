use tauri::{AppHandle, Manager};
use crate::state::AppState;
use super::{lifecycle, registry::{AgentRecord, AgentStatus}, view::{self, SubagentRoster, SubagentSendReceipt, SubagentTranscriptCursor, SubagentTranscriptPage, SubagentView}};

async fn project(app: &AppHandle, record: AgentRecord) -> Result<SubagentView, String> {
    let mut view = view::project(app, record.clone());
    let (retained_progress, retained_model) = if view.progress.is_none() || view.model.is_none() {
        lifecycle::inspect(app, &record).await.unwrap_or_default()
    } else { Default::default() };
    let revive_error = if record.status == AgentStatus::Running { None } else { lifecycle::revival_error(app, &record).await };
    if view.progress.is_none() { view.progress = retained_progress; }
    if view.model.is_none() { view.model = retained_model.or_else(|| view.progress.as_ref().and_then(|value| value["model"].as_str()).map(str::to_string)); }
    view.revivable = revive_error.is_none();
    view.revive_error = revive_error;
    Ok(view)
}

async fn roster(app: &AppHandle, root: &str) -> Result<SubagentRoster, String> {
    crate::chat::storage::validate_conversation_id(root)?;
    crate::chat::storage::load_conversation(app, root)?;
    let records = lifecycle::discover(app, root).await?;
    let mut agents = Vec::new();
    let state = app.state::<AppState>();
    for record in records.into_iter().filter(|record| state.sub_agents.registry.is_published(root, &record.id)) { agents.push(project(app, record).await?); }
    let running_count = agents.iter().filter(|agent| agent.status == AgentStatus::Running).count();
    Ok(SubagentRoster { agents, running_count })
}

#[tauri::command]
pub async fn chat_subagent_snapshot(app: AppHandle, conversation_id: String) -> Result<SubagentRoster, String> { roster(&app, &conversation_id).await }

#[tauri::command]
pub async fn chat_subagent_open(app: AppHandle, conversation_id: String, agent_id: String, expected_generation: u64) -> Result<SubagentView, String> {
    let record = lifecycle::ensure_live(&app, &conversation_id, &agent_id, expected_generation).await?;
    project(&app, record).await
}

#[tauri::command]
pub async fn chat_subagent_send(app: AppHandle, conversation_id: String, agent_id: String, expected_generation: u64, content: String) -> Result<SubagentSendReceipt, String> {
    let state = app.state::<AppState>();
    let record = state.sub_agents.registry.resolve(&conversation_id, &agent_id).ok_or("Agent not found")?;
    if record.generation != expected_generation { return Err("Agent changed. Refresh its snapshot.".into()); }
    let receipt = super::irc::send(&app, &conversation_id, "@user", &format!("agent://{}", record.id), &content).await?;
    Ok(SubagentSendReceipt { id: receipt.id, recipient: receipt.recipient, status: receipt.status })
}

#[tauri::command]
pub async fn chat_subagent_kill(app: AppHandle, conversation_id: String, agent_id: String, expected_generation: u64) -> Result<SubagentView, String> {
    project(&app, lifecycle::kill(&app, &conversation_id, &agent_id, expected_generation).await?).await
}

#[tauri::command]
pub async fn chat_subagent_park(app: AppHandle, conversation_id: String, agent_id: String, expected_generation: u64) -> Result<SubagentView, String> {
    project(&app, lifecycle::park(&app, &conversation_id, &agent_id, expected_generation).await?).await
}

#[tauri::command]
pub async fn chat_subagent_restore(app: AppHandle, conversation_id: String) -> Result<SubagentRoster, String> {
    lifecycle::restore_interrupted(&app, &conversation_id).await?;
    roster(&app, &conversation_id).await
}

#[tauri::command]
pub async fn chat_subagent_transcript(app: AppHandle, conversation_id: String, agent_id: String, expected_generation: u64, cursor: Option<SubagentTranscriptCursor>) -> Result<SubagentTranscriptPage, String> {
    view::transcript(&app, &conversation_id, &agent_id, expected_generation, cursor).await
}
