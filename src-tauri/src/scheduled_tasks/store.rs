//! JSON persistence: `tasks.json` holds every task; `runs/<task>.json` holds
//! that task's newest runs. Writes are atomic (temp file + rename).

use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::types::{ScheduledTask, TaskRun};

pub const MAX_RUNS_PER_TASK: usize = 50;

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn load_tasks(&self) -> Vec<ScheduledTask> {
        read_json(&self.dir.join("tasks.json"))
    }

    pub fn save_tasks(&self, tasks: &[ScheduledTask]) -> Result<(), String> {
        write_json(&self.dir.join("tasks.json"), tasks)
    }

    pub fn load_runs(&self, task_id: &str) -> Vec<TaskRun> {
        read_json(&self.runs_path(task_id))
    }

    /// Replaces the run with the same id or prepends a new one; keeps the newest runs.
    pub fn upsert_run(&self, run: &TaskRun) -> Result<(), String> {
        let mut runs = self.load_runs(&run.task_id);
        match runs.iter_mut().find(|existing| existing.id == run.id) {
            Some(existing) => *existing = run.clone(),
            None => runs.insert(0, run.clone()),
        }
        runs.truncate(MAX_RUNS_PER_TASK);
        write_json(&self.runs_path(&run.task_id), &runs)
    }

    pub fn save_runs(&self, task_id: &str, runs: &[TaskRun]) -> Result<(), String> {
        write_json(&self.runs_path(task_id), runs)
    }

    pub fn delete_runs(&self, task_id: &str) {
        let _ = fs::remove_file(self.runs_path(task_id));
    }

    fn runs_path(&self, task_id: &str) -> PathBuf {
        self.dir.join("runs").join(format!("{task_id}.json"))
    }
}

/// Missing file → empty. A corrupt file is kept aside so the next save cannot
/// silently destroy what the user had.
fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(text) = fs::read_to_string(path) else {
        return T::default();
    };
    match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(err) => {
            let backup =
                path.with_extension(format!("corrupt-{}.json", chrono::Local::now().timestamp()));
            eprintln!(
                "[scheduled-tasks] {} is unreadable ({err}); moved to {}",
                path.display(),
                backup.display()
            );
            let _ = fs::rename(path, backup);
            T::default()
        }
    }
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    crate::chat::storage::atomic_write(path, &text, "scheduled tasks")
}
