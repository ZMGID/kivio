use tauri::AppHandle;

use super::types::{
    RunTrigger, ScheduleRule, ScheduledTask, ScheduledTaskInput, TaskRun, TaskSource,
};
use super::{emit_changed, now_secs, service, start_run};

#[tauri::command]
pub fn scheduled_tasks_list(app: AppHandle) -> Vec<ScheduledTask> {
    service(&app).list()
}

#[tauri::command]
pub async fn scheduled_task_save(
    app: AppHandle,
    task: ScheduledTaskInput,
) -> Result<ScheduledTask, String> {
    // Editing keeps who created it; only brand-new tasks from the UI are `user`.
    let source = match task.id.as_deref() {
        Some(id) => service(&app).get(id)?.source,
        None => TaskSource::User,
    };
    let saved = super::save_task(&app, task, source).await?;
    emit_changed(&app, &saved.id, None);
    Ok(saved)
}

#[tauri::command]
pub fn scheduled_task_delete(app: AppHandle, id: String) -> Result<(), String> {
    service(&app).delete(&id)?;
    emit_changed(&app, &id, None);
    Ok(())
}

#[tauri::command]
pub fn scheduled_task_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<ScheduledTask, String> {
    let task = service(&app).set_enabled(&id, enabled, now_secs())?;
    emit_changed(&app, &id, None);
    Ok(task)
}

/// Runs the task now without touching its schedule.
#[tauri::command]
pub fn scheduled_task_run_now(app: AppHandle, id: String) -> Result<TaskRun, String> {
    let task = service(&app).get(&id)?;
    Ok(start_run(&app, task, RunTrigger::Manual, None))
}

#[tauri::command]
pub fn scheduled_task_runs(app: AppHandle, id: String) -> Vec<TaskRun> {
    service(&app).runs(&id)
}

#[tauri::command]
pub fn scheduled_task_run_delete(
    app: AppHandle,
    task_id: String,
    run_id: String,
) -> Result<(), String> {
    service(&app).delete_run(&task_id, &run_id)?;
    emit_changed(&app, &task_id, None);
    Ok(())
}

#[tauri::command]
pub fn scheduled_task_preview(schedule: ScheduleRule) -> Result<Vec<i64>, String> {
    super::preview(&schedule)
}
