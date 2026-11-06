use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use crate::{native_tools::BackgroundCommandStatus, state::AppState};
use super::{SubAgentManager, MAIN_AGENT_ID};

pub(crate) fn owner_for_run(manager: &SubAgentManager, root: &str, run: &str) -> String {
    manager.registry.find_by_run(run).filter(|(found_root, _)| found_root == root)
        .and_then(|(_, id)| manager.registry.get(root, &id))
        .map_or_else(|| root.to_string(), |record| record.child_conversation_id)
}

fn scope(manager: &SubAgentManager, root: &str, owner: &str) -> Result<String, String> {
    if owner == MAIN_AGENT_ID { return Ok(root.into()); }
    manager.registry.resolve(root, owner).map(|record| record.child_conversation_id).ok_or("Agent not found".into())
}

pub(crate) fn subscribe(app: &AppHandle) -> tokio::sync::watch::Receiver<u64> {
    app.state::<AppState>().background_commands_handle().subscribe()
}

pub(crate) fn running(app: &AppHandle, root: &str, owner: &str) -> Result<Vec<String>, String> {
    let state = app.state::<AppState>();
    let owner = scope(&state.sub_agents, root, owner)?;
    Ok(state.background_commands_handle().snapshots(Some(&owner), false).into_iter()
        .filter(|job| matches!(job.status, BackgroundCommandStatus::Running)).map(|job| job.job_id).collect())
}

pub(crate) fn completed(app: &AppHandle, root: &str, owner: &str, ids: &[String]) -> Result<Vec<Value>, String> {
    let state = app.state::<AppState>();
    let owner = scope(&state.sub_agents, root, owner)?;
    let commands = state.background_commands_handle();
    Ok(ids.iter().filter_map(|id| {
        let job = commands.snapshot(id, Some(&owner));
        if job.as_ref().is_some_and(|job| matches!(job.status, BackgroundCommandStatus::Running)) { return None; }
        Some(match job {
            Some(job) => json!({"id":job.job_id,"kind":"service","status":job.status,"command":job.command,"url":format!("proc://{id}")}),
            None => json!({"id":id,"kind":"service","status":"removed"}),
        })
    }).collect())
}

pub(crate) async fn read(app: &AppHandle, root: &str, run: &str, url: &str) -> Result<String, String> {
    let id = url.strip_prefix("proc://").ok_or("Invalid process URL")?;
    if id.contains(['/', '\\', '?', '#']) { return Err("Invalid process URL".into()); }
    let state = app.state::<AppState>();
    let owner = owner_for_run(&state.sub_agents, root, run);
    if id.is_empty() {
        return crate::native_tools::list_background(&state, &json!({}), Some(&owner));
    }
    crate::native_tools::bash_output(&state, &json!({"job_id":id,"wait_ms":0}), Some(&owner)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unregistered_run_never_resolves_another_root_owner() {
        let manager = SubAgentManager::default();
        assert_eq!(owner_for_run(&manager, "root-a", "unknown"), "root-a");
        assert!(scope(&manager, "root-a", "Other").is_err());
    }
}
