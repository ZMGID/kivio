//! Tauri host for the shared computer-control installer.
#[path = "computer_control_runtime.rs"]
mod runtime;
pub use runtime::{ControlTool, ControlToolStatus};
pub(crate) use runtime::{CUA_MCP_SERVER_ID, LEGACY_CUA_MCP_SERVER_ID, is_cua_mcp_server_id};
use tauri::AppHandle;
use std::path::Path;
use crate::skills::SkillMeta;
fn import_control_skill(app: AppHandle, source: &Path) -> Result<SkillMeta, String> {
    let result = crate::skills::import_skill(app, source.to_string_lossy().into_owned());
    result.skill.filter(|_| result.success).ok_or_else(|| {
        result
            .error
            .unwrap_or_else(|| "Skill installation failed".into())
    })
}


#[tauri::command]
pub async fn computer_control_check(tool: ControlTool) -> Result<String, String> { runtime::computer_control_check(tool).await }
#[tauri::command]
pub async fn computer_control_status(tool: ControlTool, check_updates: Option<bool>) -> Result<ControlToolStatus, String> { runtime::computer_control_status(tool, check_updates).await }
#[tauri::command]
pub async fn computer_control_install(app: AppHandle, tool: ControlTool) -> Result<SkillMeta, String> {
    let source = runtime::computer_control_install(tool).await?;
    import_control_skill(app, &source)
}
#[tauri::command]
pub async fn computer_control_update(app: AppHandle, tool: ControlTool) -> Result<SkillMeta, String> {
    let source = runtime::computer_control_update(tool).await?;
    import_control_skill(app, &source)
}
