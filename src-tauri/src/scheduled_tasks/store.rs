//! JSON persistence: `tasks.json` holds every task; `runs/<task>.json` holds
//! that task's newest runs. Writes are atomic (temp file + rename).

use std::fs;
use std::io::Write;
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

    pub fn load_tasks(&self) -> Result<Vec<ScheduledTask>, String> {
        read_list(&self.dir.join("tasks.json"))
    }

    pub fn save_tasks(&self, tasks: &[ScheduledTask]) -> Result<(), String> {
        write_json(&self.dir.join("tasks.json"), tasks)
    }

    pub fn load_runs(&self, task_id: &str) -> Result<Vec<TaskRun>, String> {
        read_list(&self.runs_path(task_id))
    }

    /// Replaces the run with the same id or prepends a new one; keeps the newest runs.
    /// A failure to read existing history is returned and does not write.
    pub fn upsert_run(&self, run: &TaskRun) -> Result<(), String> {
        let mut runs = self.load_runs(&run.task_id)?;
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

/// NotFound → empty. Any other read error is returned so callers do not replace the file.
/// Invalid UTF-8 or JSON is copied to a unique sibling and removed only after that copy
/// is durable. If the backup cannot be finished, the original stays and this returns Err.
fn read_list<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(format!("无法读取定时任务数据 {}：{err}", path.display()));
        }
    };
    let reason = match std::str::from_utf8(&bytes) {
        Ok(text) => match serde_json::from_str::<Vec<T>>(text) {
            Ok(value) => return Ok(value),
            Err(err) => err.to_string(),
        },
        Err(err) => err.to_string(),
    };
    let backup = write_unique_backup(path, &bytes)?;
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "损坏的定时任务数据已备份到 {}，但无法移走 {}：{err}",
                backup.display(),
                path.display()
            ));
        }
    }
    eprintln!(
        "[scheduled-tasks] {} is unreadable ({reason}); kept at {}",
        path.display(),
        backup.display()
    );
    Ok(Vec::new())
}

fn write_unique_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scheduled");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut failure = None;
    for seq in 0..1_000u32 {
        let dest = parent.join(format!("{stem}.corrupt-{stamp}-{seq}.json"));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                failure = Some(err);
                break;
            }
        };
        if let Err(err) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&dest);
            return Err(format!(
                "无法备份损坏的定时任务数据 {}：{err}",
                path.display()
            ));
        }
        return Ok(dest);
    }
    Err(format!(
        "无法备份损坏的定时任务数据 {}：{}",
        path.display(),
        failure
            .map(|err| err.to_string())
            .unwrap_or_else(|| "备份文件名已耗尽".into())
    ))
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    crate::chat::storage::atomic_write(path, &text, "scheduled tasks")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::super::types::{RunStatus, RunTrigger, TaskRun};
    use super::Store;

    fn sample(id: &str, task_id: &str) -> TaskRun {
        TaskRun {
            id: id.into(),
            task_id: task_id.into(),
            trigger: RunTrigger::Schedule,
            scheduled_at: Some(10),
            status: RunStatus::Succeeded,
            conversation_id: None,
            error: None,
            created_at: 10,
            started_at: None,
            finished_at: Some(12),
        }
    }

    fn corrupt_backups(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file()
                && entry.file_name().to_string_lossy().contains(".corrupt-")
            {
                found.push(entry.path());
            }
        }
        found.sort();
        found
    }

    fn payloads(dir: &Path) -> Vec<Vec<u8>> {
        let mut bytes = corrupt_backups(dir)
            .into_iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        bytes.sort();
        bytes
    }

    #[test]
    fn missing_file_is_empty_and_valid_empty_array_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        assert!(store.load_tasks().unwrap().is_empty());
        assert!(store.load_runs("absent").unwrap().is_empty());

        let path = dir.path().join("tasks.json");
        fs::write(&path, b"[]").unwrap();
        assert!(store.load_tasks().unwrap().is_empty());
        assert!(path.is_file());
        assert!(corrupt_backups(dir.path()).is_empty());
    }

    #[test]
    fn upsert_keeps_previously_loaded_runs() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.upsert_run(&sample("a", "task-1")).unwrap();
        store.upsert_run(&sample("b", "task-1")).unwrap();
        let runs = store.load_runs("task-1").unwrap();
        assert_eq!(
            runs.iter().map(|run| run.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }

    #[test]
    fn corrupt_utf8_and_json_are_kept_under_unique_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let tasks = dir.path().join("tasks.json");
        let first = b"\xff\xfetasks-v1";
        let second = b"\xfftasks-v2";
        fs::write(&tasks, first).unwrap();
        assert!(store.load_tasks().unwrap().is_empty());
        fs::write(&tasks, second).unwrap();
        assert!(store.load_tasks().unwrap().is_empty());
        assert!(!tasks.exists());
        assert_eq!(payloads(dir.path()), vec![second.to_vec(), first.to_vec()]);
        let names: Vec<_> = corrupt_backups(dir.path())
            .iter()
            .map(|path| path.file_name().unwrap().to_os_string())
            .collect();
        assert_eq!(names.len(), 2);
        assert_ne!(names[0], names[1]);

        let runs_dir = dir.path().join("runs");
        fs::create_dir_all(&runs_dir).unwrap();
        let runs = runs_dir.join("task-1.json");
        let broken = b"{\"id\":";
        fs::write(&runs, broken).unwrap();
        assert!(store.load_runs("task-1").unwrap().is_empty());
        assert!(!runs.exists());
        store.upsert_run(&sample("fresh", "task-1")).unwrap();
        assert_eq!(payloads(&runs_dir), vec![broken.to_vec()]);
        assert_eq!(store.load_runs("task-1").unwrap()[0].id, "fresh");
    }

    #[cfg(unix)]
    struct ResetMode<'a> {
        path: &'a Path,
        mode: u32,
    }

    #[cfg(unix)]
    impl Drop for ResetMode<'_> {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let Ok(meta) = fs::metadata(self.path) else {
                return;
            };
            let mut perms = meta.permissions();
            perms.set_mode(self.mode);
            let _ = fs::set_permissions(self.path, perms);
        }
    }

    #[cfg(unix)]
    fn tighten(path: &Path, mode: u32) -> ResetMode<'_> {
        use std::os::unix::fs::PermissionsExt;
        let original = fs::metadata(path).unwrap().permissions().mode();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(path, perms).unwrap();
        ResetMode {
            path,
            mode: original,
        }
    }

    #[cfg(unix)]
    #[test]
    fn refused_read_does_not_replace_tasks_or_run_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let tasks = dir.path().join("tasks.json");
        let runs_dir = dir.path().join("runs");
        fs::create_dir_all(&runs_dir).unwrap();
        let runs = runs_dir.join("task-1.json");
        let tasks_bytes = b"\xff tasks-v1";
        let runs_bytes = b"\xff runs-v1";
        fs::write(&tasks, tasks_bytes).unwrap();
        fs::write(&runs, runs_bytes).unwrap();
        let tasks_guard = tighten(&tasks, 0o000);
        let runs_guard = tighten(&runs, 0o000);
        // Privileged runners can bypass mode bits; do not quarantine the fixture there.
        if fs::read(&tasks).is_ok() {
            return;
        }
        let tasks_result = store.load_tasks();
        let runs_result = store.load_runs("task-1");
        let upsert_result = store.upsert_run(&sample("new", "task-1"));
        drop(tasks_guard);
        drop(runs_guard);
        assert!(tasks_result.is_err());
        assert!(runs_result.is_err());
        assert!(upsert_result.is_err());
        assert_eq!(fs::read(&tasks).unwrap(), tasks_bytes);
        assert_eq!(fs::read(&runs).unwrap(), runs_bytes);
        assert!(corrupt_backups(dir.path()).is_empty());
        assert!(corrupt_backups(&runs_dir).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn corrupt_backup_failure_keeps_original_and_blocks_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let runs_dir = dir.path().join("runs");
        fs::create_dir_all(&runs_dir).unwrap();
        let runs = runs_dir.join("task-1.json");
        let original = b"\xff\xfe{\"broken";
        fs::write(&runs, original).unwrap();
        let guard = tighten(&runs_dir, 0o555);
        // Privileged runners can create files even in a mode-555 directory.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let loaded = store.load_runs("task-1");
        let upserted = store.upsert_run(&sample("new", "task-1"));
        drop(guard);
        assert!(loaded.is_err());
        assert!(upserted.is_err());
        assert_eq!(fs::read(&runs).unwrap(), original);
        assert!(corrupt_backups(&runs_dir).is_empty());
    }
}
