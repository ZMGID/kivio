use std::fs;
use std::path::{Path, PathBuf};

use tauri::AppHandle;

use super::storage::{automations_dir, validate_id};
use super::types::{AutomationRun, AutomationRunSummary};

const MAX_RUNS_PER_AUTOMATION: usize = 50;

pub(crate) fn delete_all(app: &AppHandle, automation_id: &str) -> Result<(), String> {
    validate_id(automation_id)?;
    let dir = automations_dir(app)?.join("runs").join(automation_id);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|err| format!("delete run history failed: {err}"))?;
    }
    Ok(())
}

fn runs_dir(app: &AppHandle, automation_id: &str) -> Result<PathBuf, String> {
    validate_id(automation_id)?;
    let dir = automations_dir(app)?.join("runs").join(automation_id);
    fs::create_dir_all(&dir).map_err(|err| format!("create runs dir failed: {err}"))?;
    Ok(dir)
}

fn run_path(app: &AppHandle, automation_id: &str, run_id: &str) -> Result<PathBuf, String> {
    validate_id(run_id)?;
    Ok(runs_dir(app, automation_id)?.join(format!("{run_id}.json")))
}

pub(crate) fn write_run(app: &AppHandle, run: &AutomationRun) -> Result<(), String> {
    write_run_in(&runs_dir(app, &run.automation_id)?, run)
}

fn write_run_in(dir: &Path, run: &AutomationRun) -> Result<(), String> {
    validate_id(&run.id)?;
    let path = dir.join(format!("{}.json", run.id));
    let new_run = !path.exists();
    let json = serde_json::to_string_pretty(run).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|err| format!("write run failed: {err}"))?;
    fs::rename(&tmp, path).map_err(|err| format!("commit run failed: {err}"))?;
    if new_run {
        prune(dir)?;
    }
    Ok(())
}

fn prune(dir: &Path) -> Result<(), String> {
    let mut summaries = list_in(dir)?;
    if summaries.len() <= MAX_RUNS_PER_AUTOMATION {
        return Ok(());
    }
    summaries.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    let extra = summaries.len() - MAX_RUNS_PER_AUTOMATION;
    for summary in summaries.into_iter().take(extra) {
        validate_id(&summary.id)?;
        let path = dir.join(format!("{}.json", summary.id));
        let _ = fs::remove_file(path);
    }
    Ok(())
}

pub(crate) fn list(
    app: &AppHandle,
    automation_id: &str,
) -> Result<Vec<AutomationRunSummary>, String> {
    let dir = runs_dir(app, automation_id)?;
    list_in(&dir)
}

fn list_in(dir: &Path) -> Result<Vec<AutomationRunSummary>, String> {
    let mut items = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|err| format!("list runs failed: {err}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        match read_run_file(&path) {
            Ok(run) => items.push(AutomationRunSummary {
                id: run.id,
                origin: run.origin,
                status: run.status,
                started_at: run.started_at,
                finished_at: run.finished_at,
                error: run.error,
            }),
            Err(err) => eprintln!("skip automation run {}: {err}", path.display()),
        }
    }
    items.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(items)
}

pub(crate) fn get(
    app: &AppHandle,
    automation_id: &str,
    run_id: &str,
) -> Result<AutomationRun, String> {
    read_run_file(&run_path(app, automation_id, run_id)?)
}

fn read_run_file(path: &PathBuf) -> Result<AutomationRun, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("read run failed: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("parse run failed: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoints_do_not_prune_and_new_runs_keep_the_retention_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut records = Vec::new();
        for i in 0..=MAX_RUNS_PER_AUTOMATION {
            let run = AutomationRun {
                id: uuid::Uuid::new_v4().to_string(),
                automation_id: uuid::Uuid::new_v4().to_string(),
                origin: "manual".into(),
                status: "success".into(),
                started_at: format!("{i:04}"),
                finished_at: None,
                error: None,
                nodes: vec![],
            };
            fs::write(
                dir.path().join(format!("{}.json", run.id)),
                serde_json::to_vec(&run).unwrap(),
            )
            .unwrap();
            records.push(run);
        }
        let current = records.last().unwrap();
        write_run_in(dir.path(), current).unwrap();
        assert_eq!(
            list_in(dir.path()).unwrap().len(),
            MAX_RUNS_PER_AUTOMATION + 1,
            "a node checkpoint must not scan and prune unrelated run history"
        );
        let mut next = current.clone();
        next.id = uuid::Uuid::new_v4().to_string();
        next.started_at = "9999".into();
        write_run_in(dir.path(), &next).unwrap();
        let retained = list_in(dir.path()).unwrap();
        assert_eq!(retained.len(), MAX_RUNS_PER_AUTOMATION);
        assert_eq!(retained[0].id, next.id);
        assert!(!dir.path().join(format!("{}.json", records[0].id)).exists());
        assert_eq!(
            read_run_file(&dir.path().join(format!("{}.json", current.id)))
                .unwrap()
                .id,
            current.id
        );
    }
}
