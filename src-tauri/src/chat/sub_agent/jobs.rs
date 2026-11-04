//! Owner-scoped background jobs (OMP `async/job-manager.ts`, `job-control.ts`).
//! A job stands for one subagent run; its result is delivered to `owner_id` once
//! (auto-delivery or `wait`), never both. Job status is independent of agent status.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;

use super::registry::now_ms;

/// Unconsumed settled results are dropped after this (OMP default 5 min).
const UNCONSUMED_RETENTION_MS: i64 = 5 * 60_000;
/// Consumed results are dropped this long after consumption (OMP 30 s).
const CONSUMED_RETENTION_MS: i64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: String,
    /// Unique per execution, unlike the reusable display/job id.
    pub execution_id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub root: String,
    pub status: JobStatus,
    pub label: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// Settled result text (`<task-result>` + follow-up); `None` once consumed.
    pub result: Option<String>,
    /// Latest `AgentProgress` JSON while running.
    pub progress: Option<Value>,
    pub consumed: bool,
}

impl JobSnapshot {
    pub fn duration_ms(&self) -> i64 {
        self.finished_at.unwrap_or_else(now_ms) - self.started_at
    }
}

struct Job {
    snapshot: JobSnapshot,
    cancel: watch::Sender<bool>,
    /// Active `wait` calls watching this job; watched results skip auto-delivery.
    watchers: usize,
    inline: bool,
    delivery_in_flight: bool,
    expires_at: Option<tokio::time::Instant>,
}

/// Called with `(root, owner_id)` whenever an owner may have a deliverable result.
pub type SettledHook = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub struct JobManager {
    inner: Arc<Mutex<HashMap<String, Job>>>,
    // shortcut: ids remain reserved for this process; carry execution ids into executor callbacks to reclaim them safely.
    used_ids: Mutex<HashSet<String>>,
    changed: watch::Sender<u64>,
    hook: Mutex<Option<SettledHook>>,
}

impl Default for JobManager {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            used_ids: Mutex::new(HashSet::new()),
            changed: watch::channel(0).0,
            hook: Mutex::new(None),
        }
    }
}

impl JobManager {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Job>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bump(&self) {
        self.changed.send_modify(|seq| *seq += 1);
    }

    fn notify(&self, root: &str, owner: &str) {
        let hook = self.hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(hook) = hook {
            hook(root, owner);
        }
    }

    /// Delivery installs this once at startup.
    pub fn set_settled_hook(&self, hook: SettledHook) {
        *self.hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    /// Wakes on every register/progress/settle/consume; `wait` races on it.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn schedule_eviction(&self, id: &str, execution_id: &str, deadline: tokio::time::Instant) {
        let jobs = Arc::downgrade(&self.inner);
        let id = id.to_string();
        let execution_id = execution_id.to_string();
        let task = async move {
            tokio::time::sleep_until(deadline).await;
            if let Some(jobs) = jobs.upgrade() {
                let mut jobs = jobs.lock().unwrap_or_else(|e| e.into_inner());
                if jobs.get(&id).is_some_and(|job| {
                    job.snapshot.execution_id == execution_id
                        && !job.delivery_in_flight
                        && job.expires_at == Some(deadline)
                }) {
                    jobs.remove(&id);
                }
            }
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(task);
        } else {
            tauri::async_runtime::spawn(task);
        }
    }

    fn schedule_snapshots(&self, snapshots: &[JobSnapshot]) {
        let deadlines: Vec<_> = {
            let jobs = self.lock();
            snapshots.iter().filter_map(|snapshot| {
                jobs.get(&snapshot.id)
                    .filter(|job| job.snapshot.execution_id == snapshot.execution_id)
                    .and_then(|job| job.expires_at.map(|deadline| (snapshot.id.clone(), snapshot.execution_id.clone(), deadline)))
            }).collect()
        };
        for (id, execution_id, deadline) in deadlines {
            self.schedule_eviction(&id, &execution_id, deadline);
        }
    }

    /// Register a running job. The id prefers `agent_id`, suffixing `-2`, `-3`…
    /// on collision, so a job id can differ from its agent id.
    pub fn register(
        &self,
        root: &str,
        owner_id: &str,
        agent_id: &str,
        label: &str,
    ) -> (String, watch::Receiver<bool>) {
        self.register_with_mode(root, owner_id, agent_id, label, false)
    }

    pub fn register_inline(
        &self, root: &str, owner_id: &str, agent_id: &str, label: &str,
    ) -> (String, watch::Receiver<bool>) {
        self.register_with_mode(root, owner_id, agent_id, label, true)
    }

    fn register_with_mode(
        &self, root: &str, owner_id: &str, agent_id: &str, label: &str, inline: bool,
    ) -> (String, watch::Receiver<bool>) {
        let mut jobs = self.lock();
        let mut used_ids = self.used_ids.lock().unwrap_or_else(|e| e.into_inner());
        prune(&mut jobs);
        let mut id = agent_id.to_string();
        let mut n = 2;
        while used_ids.contains(&id) {
            id = format!("{agent_id}-{n}");
            n += 1;
        }
        used_ids.insert(id.clone());
        let (cancel, cancelled) = watch::channel(false);
        jobs.insert(
            id.clone(),
            Job {
                snapshot: JobSnapshot {
                    id: id.clone(),
                    execution_id: uuid::Uuid::new_v4().to_string(),
                    agent_id: agent_id.into(),
                    owner_id: owner_id.into(),
                    root: root.into(),
                    status: JobStatus::Running,
                    label: label.into(),
                    started_at: now_ms(),
                    finished_at: None,
                    result: None,
                    progress: None,
                    consumed: false,
                },
                cancel,
                watchers: 0,
                inline,
                delivery_in_flight: false,
                expires_at: None,
            },
        );
        drop(jobs);
        self.bump();
        (id, cancelled)
    }

    pub fn report_progress(&self, job_id: &str, mut progress: Value) {
        let updated = if let Some(job) = self.lock().get_mut(job_id) {
            let status = progress.get("status").and_then(Value::as_str);
            let matches = match job.snapshot.status {
                JobStatus::Running => true,
                JobStatus::Completed => status == Some("completed"),
                JobStatus::Failed => matches!(status, Some("failed" | "aborted")),
                JobStatus::Cancelled => matches!(status, Some("cancelled" | "aborted")),
            };
            if matches {
                if job.snapshot.status == JobStatus::Running {
                    let wait = job.snapshot.progress.as_mut().and_then(Value::as_object_mut)
                        .and_then(|previous| previous.remove("wait"));
                    if let (Some(wait), Some(object)) = (wait, progress.as_object_mut()) {
                        object.entry("wait").or_insert(wait);
                    }
                }
                job.snapshot.progress = Some(progress);
            }
            matches
        } else {
            false
        };
        if updated {
            self.bump();
        }
    }

    /// Preserve a nested child's live wait snapshot alongside its normal progress.
    pub(crate) fn report_wait_progress(&self, root: &str, agent_id: &str, wait: Option<Value>) {
        let updated = {
            let mut jobs = self.lock();
            if let Some(job) = jobs.values_mut().find(|job| {
                job.snapshot.root == root && job.snapshot.agent_id == agent_id
                    && job.snapshot.status == JobStatus::Running
            }) {
                let progress = job.snapshot.progress.get_or_insert_with(|| serde_json::json!({}));
                if let Some(object) = progress.as_object_mut() {
                    if let Some(wait) = wait { object.insert("wait".into(), wait); }
                    else { object.remove("wait"); }
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if updated {
            self.bump();
        }
    }

    /// Settle a running job. A cancelled job keeps `cancelled` and is not delivered.
    pub fn settle(&self, job_id: &str, outcome: Result<String, String>) {
        let (notify, eviction) = {
            let mut jobs = self.lock();
            let Some(job) = jobs.get_mut(job_id) else { return };
            if job.snapshot.status != JobStatus::Running {
                return;
            }
            let (status, text) = match outcome {
                Ok(text) => (JobStatus::Completed, text),
                Err(text) => (JobStatus::Failed, text),
            };
            job.snapshot.status = status;
            job.snapshot.result = Some(text);
            job.snapshot.finished_at = Some(now_ms());
            let eviction = if job.inline {
                None
            } else {
                let deadline = tokio::time::Instant::now() + Duration::from_millis(UNCONSUMED_RETENTION_MS as u64);
                job.expires_at = Some(deadline);
                Some((job.snapshot.execution_id.clone(), deadline))
            };
            let notify = (!job.inline && job.watchers == 0)
                .then(|| (job.snapshot.root.clone(), job.snapshot.owner_id.clone()));
            (notify, eviction)
        };
        if let Some((execution_id, deadline)) = eviction {
            self.schedule_eviction(job_id, &execution_id, deadline);
        }
        self.bump();
        if let Some((root, owner)) = notify {
            self.notify(&root, &owner);
        }
    }

    /// Cancel a running job owned by `owner_id`; the run is told via its receiver.
    pub fn cancel(&self, root: &str, job_id: &str, owner_id: &str) -> bool {
        let cancelled = {
            let mut jobs = self.lock();
            match jobs.get_mut(job_id) {
                Some(job) if job.snapshot.root == root
                    && job.snapshot.owner_id == owner_id
                    && job.snapshot.status == JobStatus::Running => {
                    job.snapshot.status = JobStatus::Cancelled;
                    job.snapshot.finished_at = Some(now_ms());
                    let deadline = (!job.inline).then(|| tokio::time::Instant::now() + Duration::from_millis(UNCONSUMED_RETENTION_MS as u64));
                    job.expires_at = deadline;
                    let _ = job.cancel.send(true);
                    Some((job.snapshot.execution_id.clone(), deadline))
                }
                _ => None,
            }
        };
        if let Some((execution_id, deadline)) = cancelled {
            if let Some(deadline) = deadline {
                self.schedule_eviction(job_id, &execution_id, deadline);
            }
            self.bump();
            true
        } else {
            false
        }
    }

    /// Cancel every running job of `root` (parent conversation stopped).
    pub fn cancel_root(&self, root: &str) {
        let evictions = {
            let mut jobs = self.lock();
            let mut evictions = Vec::new();
            for job in jobs.values_mut() {
                if job.snapshot.root == root && job.snapshot.status == JobStatus::Running {
                    job.snapshot.status = JobStatus::Cancelled;
                    job.snapshot.finished_at = Some(now_ms());
                    if !job.inline {
                        let deadline = tokio::time::Instant::now() + Duration::from_millis(UNCONSUMED_RETENTION_MS as u64);
                        job.expires_at = Some(deadline);
                        evictions.push((job.snapshot.id.clone(), job.snapshot.execution_id.clone(), deadline));
                    }
                    let _ = job.cancel.send(true);
                }
            }
            evictions
        };
        for (id, execution_id, deadline) in evictions {
            self.schedule_eviction(&id, &execution_id, deadline);
        }
        self.bump();
    }

    pub fn get(&self, job_id: &str) -> Option<JobSnapshot> {
        self.lock().get(job_id).map(|job| job.snapshot.clone())
    }

    /// Status only, without cloning result/progress (wait checks this on every bump).
    pub fn status(&self, job_id: &str) -> Option<JobStatus> {
        self.lock().get(job_id).map(|job| job.snapshot.status)
    }

    /// Running jobs owned by `owner_id` in `root`, registration order.
    pub fn running(&self, root: &str, owner_id: &str) -> Vec<JobSnapshot> {
        self.select(root, owner_id, |job| !job.inline && job.snapshot.status == JobStatus::Running)
    }

    /// Settled (completed/failed), unconsumed results of `owner_id` — what `wait` returns first.
    pub fn undelivered(&self, root: &str, owner_id: &str) -> Vec<JobSnapshot> {
        self.select(root, owner_id, |job| deliverable(job, true))
    }

    /// Blocking and synchronous runs are consumed by their task invocation.
    pub fn take_inline(&self, root: &str, job_id: &str) -> Option<JobSnapshot> {
        let snapshot = {
            let mut jobs = self.lock();
            let job = jobs.get_mut(job_id)?;
            if !job.inline || job.snapshot.root != root || job.snapshot.status == JobStatus::Running || job.snapshot.consumed {
                return None;
            }
            let snapshot = job.snapshot.clone();
            job.inline = false;
            mark_consumed(job);
            snapshot
        };
        self.schedule_snapshots(std::slice::from_ref(&snapshot));
        self.bump();
        Some(snapshot)
    }

    /// Adopt speculative work into normal asynchronous delivery.
    pub fn promote_inline(&self, root: &str, job_id: &str) -> bool {
        let (snapshot, notify) = {
            let mut jobs = self.lock();
            let Some(job) = jobs.get_mut(job_id) else { return false };
            if !job.inline || job.snapshot.root != root || job.snapshot.consumed {
                return false;
            }
            job.inline = false;
            if job.snapshot.status != JobStatus::Running {
                job.expires_at = Some(tokio::time::Instant::now() + Duration::from_millis(UNCONSUMED_RETENTION_MS as u64));
            }
            let notify = deliverable(job, false)
                .then(|| (job.snapshot.root.clone(), job.snapshot.owner_id.clone()));
            (job.snapshot.clone(), notify)
        };
        self.schedule_snapshots(&[snapshot]);
        self.bump();
        if let Some((root, owner)) = notify {
            self.notify(&root, &owner);
        }
        true
    }

    /// Discarded speculative work must never become visible to a later run.
    pub fn discard_inline(&self, root: &str, job_id: &str) -> bool {
        let removed = {
            let mut jobs = self.lock();
            if !jobs.get(job_id).is_some_and(|job| job.inline && job.snapshot.root == root) {
                return false;
            }
            jobs.remove(job_id)
        };
        if let Some(job) = removed {
            let _ = job.cancel.send(true);
            self.bump();
            true
        } else {
            false
        }
    }

    /// Delivery drain: take unwatched settled results and mark them consumed.
    pub fn take_deliverable(&self, root: &str, owner_id: &str) -> Vec<JobSnapshot> {
        self.take_results(root, owner_id, false)
    }

    /// Reserve a batch until its durable sink commits or restores it.
    pub fn claim_deliverable(&self, root: &str, owner_id: &str) -> Vec<JobSnapshot> {
        self.take_results(root, owner_id, true)
    }

    fn take_results(&self, root: &str, owner_id: &str, in_flight: bool) -> Vec<JobSnapshot> {
        let taken = {
            let mut jobs = self.lock();
            let mut taken: Vec<JobSnapshot> = jobs.values_mut()
                .filter(|job| job.snapshot.root == root && job.snapshot.owner_id == owner_id && deliverable(job, false))
                .map(|job| {
                    let snapshot = job.snapshot.clone();
                    if in_flight {
                        job.snapshot.consumed = true;
                        job.snapshot.result = None;
                        job.delivery_in_flight = true;
                    } else {
                        mark_consumed(job);
                    }
                    snapshot
                })
                .collect();
            taken.sort_by_key(|job| (job.finished_at, job.started_at));
            taken
        };
        if !taken.is_empty() {
            if !in_flight {
                self.schedule_snapshots(&taken);
            }
            self.bump();
        }
        taken
    }

    /// `wait` entry, one lock: claim (consume) every settled, unconsumed result of
    /// `owner_id`, and watch its still-running jobs so none can settle into
    /// auto-delivery before the wait observes it. Returns `(claimed, watched_ids)`.
    pub fn begin_wait(&self, root: &str, owner_id: &str) -> (Vec<JobSnapshot>, Vec<String>) {
        let (claimed, watched) = {
            let mut jobs = self.lock();
            prune(&mut jobs);
            let mut claimed = Vec::new();
            let mut watched = Vec::new();
            for job in jobs.values_mut() {
                if job.inline || job.snapshot.root != root || job.snapshot.owner_id != owner_id {
                    continue;
                }
                if job.snapshot.status == JobStatus::Running {
                    job.watchers += 1;
                    watched.push((job.snapshot.started_at, job.snapshot.id.clone()));
                } else if deliverable(job, true) {
                    claimed.push(job.snapshot.clone());
                    mark_consumed(job);
                }
            }
            claimed.sort_by_key(|job: &JobSnapshot| (job.finished_at, job.started_at));
            watched.sort();
            (claimed, watched.into_iter().map(|(_, id)| id).collect::<Vec<_>>())
        };
        if !claimed.is_empty() {
            self.schedule_snapshots(&claimed);
            self.bump();
        }
        (claimed, watched)
    }

    /// `wait` end: atomically claim the settled, unconsumed results among `job_ids`
    /// (the jobs this wait watched) and mark them consumed.
    pub fn take_wait_results(&self, job_ids: &[String]) -> Vec<JobSnapshot> {
        let taken = {
            let mut jobs = self.lock();
            let mut taken = Vec::new();
            for id in job_ids {
                if let Some(job) = jobs.get_mut(id) {
                    if deliverable(job, true) {
                        taken.push(job.snapshot.clone());
                        mark_consumed(job);
                    }
                }
            }
            taken.sort_by_key(|job| (job.finished_at, job.started_at));
            taken
        };
        if !taken.is_empty() {
            self.schedule_snapshots(&taken);
            self.bump();
        }
        taken
    }

    /// Finish or restore a durable claim, guarding against reused execution ids.
    pub fn finish_delivery(&self, snapshots: &[JobSnapshot], committed: bool) {
        let mut owners = HashSet::new();
        {
            let mut jobs = self.lock();
            for snapshot in snapshots {
                if let Some(job) = jobs.get_mut(&snapshot.id) {
                    if job.delivery_in_flight && job.snapshot.execution_id == snapshot.execution_id {
                        job.delivery_in_flight = false;
                        if committed {
                            mark_consumed(job);
                        } else {
                            job.snapshot.consumed = false;
                            job.snapshot.result = snapshot.result.clone();
                            if deliverable(job, false) {
                                owners.insert((job.snapshot.root.clone(), job.snapshot.owner_id.clone()));
                            }
                        }
                    }
                }
            }
        }
        self.schedule_snapshots(snapshots);
        self.bump();
        for (root, owner) in owners {
            self.notify(&root, &owner);
        }
    }

    /// Mark results consumed (returned by `wait`); later snapshots omit their bodies.
    pub fn consume(&self, job_ids: &[String]) {
        let mut consumed = Vec::new();
        {
            let mut jobs = self.lock();
            for id in job_ids {
                if let Some(job) = jobs.get_mut(id) {
                    if job.snapshot.status != JobStatus::Running && !job.inline && !job.delivery_in_flight {
                        mark_consumed(job);
                        consumed.push(job.snapshot.clone());
                    }
                }
            }
        }
        self.schedule_snapshots(&consumed);
        self.bump();
    }

    pub fn watch_jobs(&self, job_ids: &[String]) {
        let mut jobs = self.lock();
        for id in job_ids {
            if let Some(job) = jobs.get_mut(id) {
                job.watchers += 1;
            }
        }
    }

    /// Stop watching; a settled, unconsumed result becomes auto-deliverable again.
    pub fn unwatch_jobs(&self, job_ids: &[String]) {
        let mut owners = Vec::new();
        {
            let mut jobs = self.lock();
            for id in job_ids {
                if let Some(job) = jobs.get_mut(id) {
                    job.watchers = job.watchers.saturating_sub(1);
                    if deliverable(job, false) {
                        owners.push((job.snapshot.root.clone(), job.snapshot.owner_id.clone()));
                    }
                }
            }
        }
        owners.dedup();
        for (root, owner) in owners {
            self.notify(&root, &owner);
        }
    }

    pub fn remove_root(&self, root: &str) {
        self.lock().retain(|_, job| job.snapshot.root != root);
        self.bump();
    }

    fn select(
        &self,
        root: &str,
        owner_id: &str,
        keep: impl Fn(&Job) -> bool,
    ) -> Vec<JobSnapshot> {
        let mut jobs = self.lock();
        prune(&mut jobs);
        let mut list: Vec<_> = jobs
            .values()
            .filter(|job| job.snapshot.root == root && job.snapshot.owner_id == owner_id && keep(job))
            .map(|job| job.snapshot.clone())
            .collect();
        list.sort_by_key(|job| job.started_at);
        list
    }
}

fn deliverable(job: &Job, include_watched: bool) -> bool {
    !job.inline && matches!(job.snapshot.status, JobStatus::Completed | JobStatus::Failed)
        && !job.snapshot.consumed
        && (include_watched || job.watchers == 0)
}

fn mark_consumed(job: &mut Job) {
    job.snapshot.consumed = true;
    job.snapshot.result = None;
    job.expires_at = Some(tokio::time::Instant::now() + Duration::from_millis(CONSUMED_RETENTION_MS as u64));
}

fn prune(jobs: &mut HashMap<String, Job>) {
    let now = tokio::time::Instant::now();
    jobs.retain(|_, job| job.delivery_in_flight || job.expires_at.is_none_or(|deadline| deadline > now));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn ids_suffix_on_collision_and_results_deliver_once() {
        let jobs = JobManager::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        jobs.set_settled_hook(Arc::new(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let (a, _) = jobs.register("c", "Main", "Scout", "x");
        let (b, _) = jobs.register("c", "Main", "Scout", "y");
        assert_eq!((a.as_str(), b.as_str()), ("Scout", "Scout-2"));
        jobs.settle(&a, Ok("done".into()));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(jobs.take_deliverable("c", "Main").len(), 1);
        assert!(jobs.take_deliverable("c", "Main").is_empty(), "consumed once");
        assert_eq!(jobs.get(&a).unwrap().result, None);
    }

    #[test]
    fn watched_results_skip_delivery_until_unwatched_and_cancel_wins() {
        let jobs = JobManager::default();
        let (a, mut cancelled) = jobs.register("c", "Main", "A", "x");
        jobs.watch_jobs(std::slice::from_ref(&a));
        jobs.settle(&a, Ok("r".into()));
        assert!(jobs.take_deliverable("c", "Main").is_empty());
        assert_eq!(jobs.undelivered("c", "Main").len(), 1);
        jobs.unwatch_jobs(std::slice::from_ref(&a));
        assert_eq!(jobs.take_deliverable("c", "Main").len(), 1);

        let (b, _) = jobs.register("c", "Main", "B", "x");
        assert!(!jobs.cancel("c", &b, "Other"), "only the owner cancels");
        assert!(jobs.cancel("c", &b, "Main"));
        jobs.settle(&b, Ok("late".into()));
        assert_eq!(jobs.get(&b).unwrap().status, JobStatus::Cancelled);
        assert!(jobs.take_deliverable("c", "Main").is_empty());
        let _ = cancelled.borrow_and_update();
    }

    #[test]
    fn inline_runs_are_private_until_taken_or_promoted() {
        let jobs = JobManager::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        jobs.set_settled_hook(Arc::new(move |_, _| { counter.fetch_add(1, Ordering::SeqCst); }));
        let (inline, _) = jobs.register_inline("root", "Main", "Inline", "work");
        assert!(jobs.running("root", "Main").is_empty());
        assert!(jobs.begin_wait("root", "Main").1.is_empty());
        jobs.settle(&inline, Ok("inline result".into()));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert!(jobs.take_deliverable("root", "Main").is_empty());
        assert!(jobs.take_inline("other", &inline).is_none());
        assert_eq!(jobs.take_inline("root", &inline).unwrap().result.as_deref(), Some("inline result"));
        assert!(jobs.take_inline("root", &inline).is_none());

        let (promoted, _) = jobs.register_inline("root", "Main", "Promoted", "work");
        jobs.settle(&promoted, Ok("adopted result".into()));
        assert!(jobs.promote_inline("root", &promoted));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(jobs.take_deliverable("root", "Main")[0].result.as_deref(), Some("adopted result"));
    }

    #[test]
    fn discarded_inline_late_settlement_cannot_change_a_reused_name() {
        let jobs = JobManager::default();
        let (old, cancelled) = jobs.register_inline("root", "Main", "Scout", "old");
        assert!(jobs.discard_inline("root", &old));
        assert!(*cancelled.borrow());
        let (new, _) = jobs.register("root", "Main", "Scout", "new");
        assert_ne!(old, new);
        jobs.settle(&old, Ok("late".into()));
        assert_eq!(jobs.get(&new).unwrap().status, JobStatus::Running);
        assert!(jobs.take_deliverable("root", "Main").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn scheduled_expiry_removes_consumed_and_unconsumed_without_queries() {
        let jobs = JobManager::default();
        let (unconsumed, _) = jobs.register("root", "Main", "Unconsumed", "work");
        let (consumed, _) = jobs.register("root", "Other", "Consumed", "work");
        jobs.settle(&unconsumed, Ok("unread".into()));
        jobs.settle(&consumed, Ok("read".into()));
        assert_eq!(jobs.take_deliverable("root", "Other").len(), 1);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(29_999)).await;
        assert!(jobs.get(&consumed).is_some());
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(jobs.get(&consumed).is_none());
        assert!(jobs.get(&unconsumed).is_some());
        tokio::time::advance(Duration::from_secs(270)).await;
        tokio::task::yield_now().await;
        assert!(jobs.get(&unconsumed).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn durable_claim_is_not_evicted_or_stolen_during_a_slow_sink() {
        let jobs = JobManager::default();
        let (id, _) = jobs.register("root", "Main", "Scout", "work");
        jobs.settle(&id, Ok("preserved".into()));
        let claim = jobs.claim_deliverable("root", "Main");
        assert!(jobs.begin_wait("root", "Main").0.is_empty());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(300)).await;
        tokio::task::yield_now().await;
        assert!(jobs.get(&id).is_some());
        jobs.finish_delivery(&claim, true);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(30)).await;
        tokio::task::yield_now().await;
        assert!(jobs.get(&id).is_none());
    }

    #[test]
    fn terminal_progress_keeps_cancelled_status_and_rejects_stale_updates() {
        let jobs = JobManager::default();
        let (id, _) = jobs.register("root", "Main", "Scout", "work");
        jobs.report_progress(&id, serde_json::json!({ "status": "running" }));
        assert!(jobs.cancel("root", &id, "Main"));
        jobs.report_progress(&id, serde_json::json!({ "status": "completed" }));
        jobs.report_progress(&id, serde_json::json!({ "status": "aborted" }));
        jobs.report_progress(&id, serde_json::json!({ "status": "running" }));
        let snapshot = jobs.get(&id).unwrap();
        assert_eq!(snapshot.status, JobStatus::Cancelled);
        assert_eq!(snapshot.progress.unwrap()["status"], "aborted");
    }
}
