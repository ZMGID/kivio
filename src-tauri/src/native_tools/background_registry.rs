//! Owns background command transitions. Waiters hold this owner, never a map.
//! Process termination and log deletion run after the registry lock is released.
use super::{kill_process_group, BackgroundCommand, BackgroundCommandStatus as Status};
use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::SystemTime};
#[derive(Clone, Debug)]
pub(crate) struct BackgroundCommandSnapshot {
    pub job_id: String,
    pub pid: Option<u32>,
    pub command: String,
    pub cwd: String,
    pub log_path: PathBuf,
    pub status: Status,
    pub started_at: SystemTime,
}
impl From<&BackgroundCommand> for BackgroundCommandSnapshot {
    fn from(job: &BackgroundCommand) -> Self {
        Self {
            job_id: job.job_id.clone(),
            pid: job.pid,
            command: job.command.clone(),
            cwd: job.cwd.clone(),
            log_path: job.log_path.clone(),
            status: job.status.clone(),
            started_at: job.started_at,
        }
    }
}
#[derive(Default)]
pub(crate) struct BackgroundCommandRegistry {
    jobs: Mutex<HashMap<String, BackgroundCommand>>,
}
fn visible(job: &BackgroundCommand, caller: Option<&str>, include_unowned: bool) -> bool {
    caller.is_none()
        || job.conversation_id.as_deref() == caller
        || (include_unowned && job.conversation_id.is_none())
}
impl BackgroundCommandRegistry {
    pub(crate) fn register(&self, job: crate::native_tools::BackgroundCommand) {
        const MAX_TRACKED_BACKGROUND_COMMANDS: usize = 64;
        let mut evicted_logs = Vec::new();
        let mut map = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        // Evict oldest terminal jobs first if we are over the cap; never evict a
        // still-running job (it owns a live process group).
        while map.len() >= MAX_TRACKED_BACKGROUND_COMMANDS {
            let oldest_terminal = map
                .values()
                .filter(|j| {
                    !matches!(
                        j.status,
                        crate::native_tools::BackgroundCommandStatus::Running
                    )
                })
                .min_by_key(|j| j.started_at)
                .map(|j| j.job_id.clone());
            match oldest_terminal {
                Some(id) => {
                    // Remove the evicted job's per-job log too, otherwise long
                    // sessions that churn >64 short-lived background commands
                    // leak one small (now-unreachable) log file per eviction.
                    if let Some(job) = map.remove(&id) {
                        evicted_logs.push(job.log_path);
                    }
                }
                // All remaining jobs are still running; stop evicting.
                None => break,
            }
        }
        map.insert(job.job_id.clone(), job);
        drop(map);
        for path in evicted_logs {
            let _ = std::fs::remove_file(path);
        }
    }
    pub(crate) fn snapshot(
        &self,
        id: &str,
        caller: Option<&str>,
    ) -> Option<BackgroundCommandSnapshot> {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .filter(|j| visible(j, caller, true))
            .map(BackgroundCommandSnapshot::from)
    }
    pub(crate) fn snapshots(
        &self,
        caller: Option<&str>,
        include_unowned: bool,
    ) -> Vec<BackgroundCommandSnapshot> {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|j| visible(j, caller, include_unowned))
            .map(BackgroundCommandSnapshot::from)
            .collect()
    }
    pub(crate) fn complete(&self, id: &str, status: Status) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = jobs.get_mut(id) {
            if !matches!(job.status, Status::Killed) {
                job.status = status;
            }
        }
    }
    pub(crate) fn kill(&self, id: &str, caller: Option<&str>) -> Result<bool, String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let job = jobs
            .get_mut(id)
            .filter(|j| visible(j, caller, true))
            .ok_or_else(|| format!("No background job with job_id {id}"))?;
        if !matches!(job.status, Status::Running) {
            return Ok(false);
        }
        job.status = Status::Killed;
        let signal = job.kill_tx.take();
        let pid = job.pid;
        drop(jobs);
        if let Some(signal) = signal {
            let _ = signal.send(());
        } else if let Some(pid) = pid {
            kill_process_group(pid);
        }
        Ok(true)
    }
    pub(crate) fn clear_finished(&self, caller: Option<&str>) {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, j| matches!(j.status, Status::Running) || !visible(j, caller, false));
    }
    pub(crate) fn kill_all(&self) -> usize {
        let jobs: Vec<_> = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, job)| job)
            .collect();
        Self::dispose(jobs)
    }
    pub(crate) fn kill_for_conversation(&self, conversation_id: &str) -> usize {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let owned: Vec<_> = jobs
            .values()
            .filter(|j| j.conversation_id.as_deref() == Some(conversation_id))
            .map(|j| j.job_id.clone())
            .collect();
        let removed = owned
            .into_iter()
            .filter_map(|id| jobs.remove(&id))
            .collect();
        drop(jobs);
        Self::dispose(removed)
    }
    fn dispose(jobs: Vec<BackgroundCommand>) -> usize {
        let mut killed = 0;
        for mut job in jobs {
            if matches!(job.status, Status::Running) {
                if let Some(signal) = job.kill_tx.take() {
                    let _ = signal.send(());
                    killed += 1;
                } else if let Some(pid) = job.pid {
                    kill_process_group(pid);
                    killed += 1;
                }
            }
            let _ = std::fs::remove_file(&job.log_path);
        }
        killed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, conversation: Option<&str>) -> BackgroundCommand {
        BackgroundCommand {
            job_id: id.into(),
            conversation_id: conversation.map(str::to_owned),
            pid: None,
            command: "test".into(),
            cwd: String::new(),
            log_path: std::env::temp_dir().join(format!("kivio-bg-owner-{}", uuid::Uuid::new_v4())),
            status: Status::Running,
            started_at: SystemTime::now(),
            kill_tx: None,
        }
    }

    #[test]
    fn kill_signal_is_once_only_and_late_completion_cannot_undo_it() {
        let owner = BackgroundCommandRegistry::default();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let mut running = job("one", Some("a"));
        running.kill_tx = Some(tx);
        owner.register(running);
        assert!(owner.kill("one", Some("b")).is_err());
        assert!(owner.kill("one", Some("a")).unwrap());
        assert!(rx.try_recv().is_ok());
        owner.complete("one", Status::Exited { code: Some(0) });
        assert_eq!(
            owner.snapshot("one", Some("a")).unwrap().status,
            Status::Killed
        );
        assert!(!owner.kill("one", Some("a")).unwrap());
    }

    #[test]
    fn panel_and_tool_visibility_preserve_unowned_policy_and_clear_only_finished() {
        let owner = BackgroundCommandRegistry::default();
        owner.register(job("owned", Some("a")));
        owner.register(job("other", Some("b")));
        owner.register(job("unowned", None));
        assert_eq!(owner.snapshots(Some("a"), false).len(), 1);
        assert_eq!(owner.snapshots(Some("a"), true).len(), 2);
        owner.complete("owned", Status::Exited { code: Some(0) });
        owner.clear_finished(Some("b"));
        assert!(owner.snapshot("owned", Some("a")).is_some());
        owner.clear_finished(Some("a"));
        assert!(owner.snapshot("owned", Some("a")).is_none());
        assert_eq!(owner.snapshots(None, false).len(), 2);
        owner.kill_for_conversation("b");
        assert_eq!(owner.snapshots(None, false).len(), 1);
        assert!(owner.snapshot("unowned", None).is_some());
    }
}
