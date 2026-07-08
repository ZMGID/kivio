use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::chat::storage::{conversations_dir, find_project_by_id};
use crate::external_agents::types::RuntimeAgentDef;

#[derive(Debug, Clone)]
pub struct ResolvedWorkspace {
    pub cwd: PathBuf,
}

pub fn resolve_effective_cwd(
    app: &AppHandle,
    conversation_id: &str,
    project_id: Option<&str>,
) -> Result<ResolvedWorkspace, String> {
    if let Some(project_id) = project_id.filter(|id| !id.trim().is_empty()) {
        if let Ok(project) = find_project_by_id(app, project_id) {
            if let Some(root) = project.root_path.filter(|p| !p.trim().is_empty()) {
                let path = PathBuf::from(root);
                if path.is_dir() {
                    return Ok(ResolvedWorkspace { cwd: path });
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
    Ok(ResolvedWorkspace { cwd: base })
}

pub fn detection_cache_ttl() -> Duration {
    Duration::from_secs(24 * 60 * 60)
}

pub fn detection_cache_fresh(stored_at: Instant) -> bool {
    stored_at.elapsed() < detection_cache_ttl()
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
