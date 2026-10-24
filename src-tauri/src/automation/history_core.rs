use std::{fs,path::{Path,PathBuf}};
use super::document::validate_id;
use super::types::{AutomationRun,AutomationRunSummary};
const MAX_RUNS_PER_AUTOMATION:usize=50;
pub fn write_run_in(dir: &Path, run: &AutomationRun) -> Result<(), String> {
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

pub fn list_in(dir: &Path) -> Result<Vec<AutomationRunSummary>, String> {
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

pub fn read_run_file(path: &PathBuf) -> Result<AutomationRun, String> {
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
