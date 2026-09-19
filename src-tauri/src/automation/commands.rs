use tauri::{AppHandle, Manager};

use super::history;
use super::mutations;
use super::runner;
use super::storage;
use super::types::{
    Automation, AutomationMeta, AutomationRun, AutomationRunStarted, AutomationRunSummary,
    RunOrigin,
};

#[tauri::command]
pub fn automation_list(app: AppHandle) -> Result<Vec<AutomationMeta>, String> {
    storage::list(&app)
}

#[tauri::command]
pub fn automation_get(app: AppHandle, id: String) -> Result<Automation, String> {
    storage::get(&app, &id)
}

#[tauri::command]
pub fn automation_save(app: AppHandle, automation: Automation) -> Result<Automation, String> {
    mutations::save(&app, automation)
}

#[tauri::command]
pub fn automation_delete(app: AppHandle, id: String) -> Result<(), String> {
    mutations::delete(&app, &id)
}

#[tauri::command]
pub fn automation_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<Automation, String> {
    mutations::set_enabled(&app, &id, enabled)
}

#[tauri::command]
pub fn automation_run(
    app: AppHandle,
    id: String,
    until_node_id: Option<String>,
) -> Result<AutomationRunStarted, String> {
    runner::enqueue(app, id, RunOrigin::Manual, until_node_id, None)
}

#[tauri::command]
pub fn automation_test_node(
    app: AppHandle,
    id: String,
    node_id: String,
    input: super::types::NodeOutput,
) -> Result<AutomationRunStarted, String> {
    runner::test_node(app, id, node_id, input)
}

#[tauri::command]
pub fn automation_validate(automation: Automation) -> Vec<super::types::ValidationIssue> {
    super::validate::validate(&automation)
}

#[tauri::command]
pub fn automation_cancel(app: AppHandle, id: String) -> Result<(), String> {
    runner::cancel(&app, &id)
}

#[tauri::command]
pub fn automation_active_run(app: AppHandle, id: String) -> Result<Option<AutomationRun>, String> {
    let run_id = app
        .state::<crate::state::AppState>()
        .automation_runs
        .active_run(&id);
    run_id
        .map(|run_id| history::get(&app, &id, &run_id))
        .transpose()
}

#[tauri::command]
pub fn automation_export(app: AppHandle, id: String, path: String) -> Result<(), String> {
    storage::export_to_file(&app, &id, &path)
}

#[tauri::command]
pub fn automation_import(app: AppHandle, path: String) -> Result<Automation, String> {
    // 导入产物必然 enabled=false，无需刷新热键。
    storage::import_from_file(&app, &path)
}

#[tauri::command]
pub fn automation_runs_list(
    app: AppHandle,
    id: String,
) -> Result<Vec<AutomationRunSummary>, String> {
    history::list(&app, &id)
}

#[tauri::command]
pub fn automation_run_get(
    app: AppHandle,
    id: String,
    run_id: String,
) -> Result<AutomationRun, String> {
    history::get(&app, &id, &run_id)
}
