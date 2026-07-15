use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::chat::storage::{conversations_dir, find_project_by_id};
use crate::external_agents::types::RuntimeAgentDef;

pub fn resolve_effective_cwd(
    app: &AppHandle,
    conversation_id: &str,
    project_id: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(project_id) = project_id.filter(|id| !id.trim().is_empty()) {
        if let Ok(project) = find_project_by_id(app, project_id) {
            if let Some(root) = project.root_path.filter(|p| !p.trim().is_empty()) {
                let path = PathBuf::from(root);
                if path.is_dir() {
                    return Ok(path);
                }
            }
        }
    }

    let base = conversations_dir(app)?
        .parent()
        .ok_or_else(|| "chat data root unavailable".to_string())?
        .join("chat-workspaces")
        .join(conversation_id);
    std::fs::create_dir_all(&base).map_err(|e| format!("create workspace: {e}"))?;
    Ok(base)
}

pub fn extra_allowed_dirs_for_agent(
    def: &RuntimeAgentDef,
    skill_scan_paths: &[String],
) -> Vec<String> {
    if def.id == "codex" {
        return Vec::new();
    }
    skill_scan_paths
        .iter()
        .filter(|p| !p.trim().is_empty() && Path::new(p).is_dir())
        .cloned()
        .collect()
}
