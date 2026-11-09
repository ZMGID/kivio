//! Owns background command transitions. Waiters hold this owner, never a map.
//! Process termination and log deletion run after the registry lock is released.
use super::{kill_process_group, BackgroundCommand, BackgroundCommandStatus as Status};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
    time::SystemTime,
};
#[derive(Clone, Debug)]
pub(crate) struct BackgroundCommandSnapshot {
    pub conversation_id: Option<String>,
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
            conversation_id: job.conversation_id.clone(),
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
pub(crate) struct BackgroundCommandRegistry {
    jobs: Mutex<HashMap<String, BackgroundCommand>>,
    retired: Mutex<HashSet<String>>,
    changed: tokio::sync::watch::Sender<u64>,
    completion: Mutex<Option<std::sync::Arc<dyn Fn(BackgroundCommandSnapshot) + Send + Sync>>>,
}
impl Default for BackgroundCommandRegistry {
    fn default() -> Self {
        Self {
            jobs: Mutex::default(),
            retired: Mutex::default(),
            changed: tokio::sync::watch::channel(0).0,
            completion: Mutex::default(),
        }
    }
}
fn visible(job: &BackgroundCommand, caller: Option<&str>, include_unowned: bool) -> bool {
    caller.is_none()
        || job.conversation_id.as_deref() == caller
        || (include_unowned && job.conversation_id.is_none())
}
fn process_pending(job: &BackgroundCommand) -> bool {
    matches!(job.status, Status::Running)
        || job.reaped.as_ref().is_some_and(|reaped| !*reaped.borrow())
}
impl BackgroundCommandRegistry {
    pub(crate) fn set_completion_hook(
        &self,
        hook: std::sync::Arc<dyn Fn(BackgroundCommandSnapshot) + Send + Sync>,
    ) {
        *self.completion.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    fn completed(&self, job: BackgroundCommandSnapshot) {
        let hook = self
            .completion
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook(job);
        }
    }
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }
    fn notify(&self) {
        self.changed
            .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
    }
    #[cfg(test)]
    pub(crate) fn register(&self, job: BackgroundCommand) {
        let owner = job.conversation_id.clone();
        self.start(owner.as_deref(), || Ok((job, ()))).unwrap();
    }

    pub(super) fn start<T>(
        &self,
        owner: Option<&str>,
        spawn: impl FnOnce() -> Result<(BackgroundCommand, T), String>,
    ) -> Result<T, String> {
        let retired = self.retired.lock().unwrap_or_else(|e| e.into_inner());
        if owner.is_some_and(|owner| retired.contains(owner)) {
            return Err("Conversation was deleted; background commands cannot start".into());
        }
        let (job, started) = spawn()?;
        const MAX_TRACKED_BACKGROUND_COMMANDS: usize = 64;
        let mut evicted_logs = Vec::new();
        let mut map = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        // Evict oldest terminal jobs first if we are over the cap; never evict a
        // still-running job (it owns a live process group).
        while map.len() >= MAX_TRACKED_BACKGROUND_COMMANDS {
            let oldest_terminal = map
                .values()
                .filter(|job| !process_pending(job))
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
        drop(retired);
        self.notify();
        for path in evicted_logs {
            let _ = std::fs::remove_file(path);
        }
        Ok(started)
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
        let completed = jobs
            .get_mut(id)
            .filter(|job| matches!(job.status, Status::Running))
            .map(|job| {
                job.status = status;
                BackgroundCommandSnapshot::from(&*job)
            });
        drop(jobs);
        self.notify();
        if let Some(job) = completed {
            self.completed(job);
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
        let completed = BackgroundCommandSnapshot::from(&*job);
        drop(jobs);
        self.notify();
        self.completed(completed);
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
            .retain(|_, j| process_pending(j) || !visible(j, caller, false));
        self.notify();
    }
    pub(crate) fn kill_all(&self) -> usize {
        let jobs: Vec<_> = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, job)| job)
            .collect();
        self.notify();
        Self::dispose(jobs)
    }
    pub(crate) fn kill_for_conversation(&self, conversation_id: &str) -> usize {
        let mut requests = Vec::new();
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, job| {
                if job.conversation_id.as_deref() != Some(conversation_id) {
                    return true;
                }
                let (signal, pid) = Self::take_stop(job);
                let pending = job.reaped.as_ref().is_some_and(|reaped| !*reaped.borrow());
                requests.push((signal, pid, (!pending).then(|| job.log_path.clone())));
                pending
            });
        self.notify();
        let mut killed = 0;
        for (signal, pid, log) in requests {
            killed += usize::from(Self::send_stop(signal, pid));
            if let Some(log) = log {
                let _ = std::fs::remove_file(log);
            }
        }
        killed
    }

    pub(crate) async fn retire_conversations(&self, owners: &[String]) -> Result<usize, String> {
        let mut jobs: Vec<_> = {
            let mut retired = self.retired.lock().unwrap_or_else(|e| e.into_inner());
            retired.extend(owners.iter().cloned());
            self.jobs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values_mut()
                .filter(|job| {
                    job.conversation_id
                        .as_ref()
                        .is_some_and(|owner| owners.contains(owner))
                })
                .map(|job| {
                    let (signal, pid) = Self::take_stop(job);
                    (
                        job.job_id.clone(),
                        job.log_path.clone(),
                        job.reaped.clone(),
                        signal,
                        pid,
                    )
                })
                .collect()
        };
        self.notify();
        let mut killed = 0;
        for (_, _, _, signal, pid) in &mut jobs {
            killed += usize::from(Self::send_stop(signal.take(), pid.take()));
        }
        let mut error = None;
        for (id, path, mut reaped, _, _) in jobs {
            let result = if let Some(reaped) = reaped.as_mut() {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    reaped.wait_for(|exited| *exited),
                )
                .await
                {
                    Ok(Ok(_)) => Ok(()),
                    Ok(Err(_)) => Err(format!(
                        "Background command {id} waiter ended without confirming process exit"
                    )),
                    Err(_) => Err(format!(
                        "Timed out waiting for background command {id} to exit"
                    )),
                }
            } else {
                Ok(())
            };
            match result {
                Ok(()) => {
                    self.jobs
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                    let _ = std::fs::remove_file(path);
                }
                Err(failure) => {
                    error.get_or_insert(failure);
                }
            }
        }
        self.notify();
        error.map_or(Ok(killed), Err)
    }

    fn take_stop(
        job: &mut BackgroundCommand,
    ) -> (Option<tokio::sync::oneshot::Sender<()>>, Option<u32>) {
        if !matches!(job.status, Status::Running) {
            return (None, None);
        }
        job.status = Status::Killed;
        let signal = job.kill_tx.take();
        let pid = if signal.is_none() { job.pid } else { None };
        (signal, pid)
    }

    fn send_stop(signal: Option<tokio::sync::oneshot::Sender<()>>, pid: Option<u32>) -> bool {
        if let Some(signal) = signal {
            let _ = signal.send(());
            true
        } else if let Some(pid) = pid {
            kill_process_group(pid);
            true
        } else {
            false
        }
    }
    fn dispose(jobs: Vec<BackgroundCommand>) -> usize {
        let mut killed = 0;
        for mut job in jobs {
            let (signal, pid) = Self::take_stop(&mut job);
            killed += usize::from(Self::send_stop(signal, pid));
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
            reaped: None,
        }
    }

    #[tokio::test]
    async fn owner_kill_keeps_exit_evidence_for_later_root_retirement() {
        let owner = BackgroundCommandRegistry::default();
        let (kill, killed) = tokio::sync::oneshot::channel();
        let (exited, reaped) = tokio::sync::watch::channel(false);
        let mut running = job("pending", Some("conv"));
        running.kill_tx = Some(kill);
        running.reaped = Some(reaped);
        owner.register(running);
        assert_eq!(owner.kill_for_conversation("conv"), 1);
        killed.await.unwrap();
        owner.clear_finished(Some("conv"));
        assert!(owner.snapshot("pending", Some("conv")).is_some());
        owner
            .start(Some("conv"), || Ok((job("revived", Some("conv")), ())))
            .unwrap();
        let owners = ["conv".to_string()];
        let mut retirement = Box::pin(owner.retire_conversations(&owners));
        assert!(futures::poll!(retirement.as_mut()).is_pending());
        exited.send(true).unwrap();
        retirement.await.unwrap();
        assert!(owner.snapshots(Some("conv"), false).is_empty());
    }

    #[tokio::test]
    async fn cancelled_retirement_keeps_exit_evidence_for_retry_and_denies_late_spawn() {
        let owner = std::sync::Arc::new(BackgroundCommandRegistry::default());
        let (kill, killed) = tokio::sync::oneshot::channel();
        let (exited, reaped) = tokio::sync::watch::channel(false);
        let mut running = job("pending", Some("conv"));
        running.kill_tx = Some(kill);
        running.reaped = Some(reaped);
        owner.register(running);
        let retiring = owner.clone();
        let task =
            tokio::spawn(async move { retiring.retire_conversations(&["conv".into()]).await });
        killed.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        owner.clear_finished(Some("conv"));
        assert!(owner.snapshot("pending", Some("conv")).is_some());
        assert!(owner
            .start::<()>(Some("conv"), || panic!("retired owner reached OS spawn"))
            .is_err());
        exited.send(true).unwrap();
        owner.retire_conversations(&["conv".into()]).await.unwrap();
        assert!(owner.snapshot("pending", Some("conv")).is_none());
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retirement_waits_for_spawn_admission_and_disposes_the_inserted_process() {
        use std::os::windows::process::CommandExt;
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        let owner = std::sync::Arc::new(BackgroundCommandRegistry::default());
        let (spawned, ready) = std::sync::mpsc::channel();
        let (release, paused) = std::sync::mpsc::channel();
        let spawning = owner.clone();
        let starter = std::thread::spawn(move || {
            spawning
                .start(Some("conv"), || {
                    let child = std::process::Command::new("powershell.exe")
                        .args([
                            "-NoProfile",
                            "-NonInteractive",
                            "-Command",
                            "Start-Sleep -Seconds 60",
                        ])
                        .creation_flags(0x08000000 | 0x00000200)
                        .spawn()
                        .unwrap();
                    let mut running = job("paused", Some("conv"));
                    running.pid = Some(child.id());
                    let (kill, killed) = tokio::sync::oneshot::channel();
                    let (exited, reaped) = tokio::sync::watch::channel(false);
                    running.kill_tx = Some(kill);
                    running.reaped = Some(reaped);
                    spawned.send(()).unwrap();
                    paused.recv().unwrap();
                    Ok((running, (child, killed, exited)))
                })
                .unwrap()
        });
        ready.recv().unwrap();
        let retiring = owner.clone();
        let mut retirement =
            tokio::spawn(async move { retiring.retire_conversations(&["conv".into()]).await });
        let early =
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut retirement).await;
        let returned_before_insert = early.is_ok();
        release.send(()).unwrap();
        let (mut child, killed, exited) = starter.join().unwrap();
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, child.id()) }.unwrap();
        let waiter = std::thread::spawn(move || {
            let _ = killed.blocking_recv();
            kill_process_group(child.id());
            child.wait().unwrap();
            exited.send(true).unwrap();
        });
        match early {
            Ok(result) => result,
            Err(_) => retirement.await,
        }
        .unwrap()
        .unwrap();
        let exited = unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 };
        waiter.join().unwrap();
        unsafe {
            let _ = CloseHandle(handle);
        }
        assert!(
            !returned_before_insert,
            "retirement passed an OS spawn whose registry insert was pending"
        );
        assert!(
            exited,
            "process admitted before retirement survived cleanup"
        );
        assert!(owner.snapshots(Some("conv"), false).is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn conversation_disposal_waits_for_real_process_exit() {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        let owner = std::sync::Arc::new(BackgroundCommandRegistry::default());
        let mut command = tokio::process::Command::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 60",
            ])
            .creation_flags(0x08000000 | 0x00000200)
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (reaped_tx, reaped) = tokio::sync::watch::channel(false);
        let mut running = job("process", Some("conv"));
        running.pid = Some(pid);
        running.kill_tx = Some(tx);
        running.reaped = Some(reaped);
        owner.register(running);
        let waiter_owner = owner.clone();
        let waiter = tokio::spawn(async move {
            let _ = rx.await;
            kill_process_group(pid);
            child.wait().await.unwrap();
            waiter_owner.complete("process", Status::Killed);
            reaped_tx.send(true).unwrap();
        });

        owner.retire_conversations(&["conv".into()]).await.unwrap();
        let stopped_at_return = unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 };
        waiter.await.unwrap();
        unsafe {
            let _ = CloseHandle(handle);
        }
        assert!(
            stopped_at_return,
            "disposal returned before its waiter killed/reaped the process"
        );
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

    #[tokio::test]
    async fn completion_and_removal_wake_waiters_without_exposing_sibling_jobs() {
        let owner = BackgroundCommandRegistry::default();
        let mut changed = owner.subscribe();
        owner.register(job("A-service", Some("subagent-root-A")));
        owner.register(job("B-service", Some("subagent-root-B")));
        changed.borrow_and_update();
        owner.complete("A-service", Status::Exited { code: Some(0) });
        tokio::time::timeout(std::time::Duration::from_secs(1), changed.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner.snapshots(Some("subagent-root-A"), false).len(), 1);
        assert!(owner
            .snapshot("B-service", Some("subagent-root-A"))
            .is_none());
        owner.kill_for_conversation("subagent-root-A");
        changed.changed().await.unwrap();
        assert!(owner
            .snapshot("B-service", Some("subagent-root-B"))
            .is_some());
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
