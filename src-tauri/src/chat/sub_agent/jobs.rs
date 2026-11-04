//! Owner-scoped background jobs (OMP `async/job-manager.ts`, `job-control.ts`).
//! A job stands for one subagent run; its result is delivered to `owner_id` once
//! (auto-delivery or `wait`), never both. Job status is independent of agent status.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    consumed_at: Option<i64>,
}

/// Called with `(root, owner_id)` whenever an owner may have a deliverable result.
pub type SettledHook = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub struct JobManager {
    inner: Mutex<HashMap<String, Job>>,
    changed: watch::Sender<u64>,
    hook: Mutex<Option<SettledHook>>,
}

impl Default for JobManager {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
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

    /// Register a running job. The id prefers `agent_id`, suffixing `-2`, `-3`…
    /// on collision, so a job id can differ from its agent id.
    pub fn register(
        &self,
        root: &str,
        owner_id: &str,
        agent_id: &str,
        label: &str,
    ) -> (String, watch::Receiver<bool>) {
        let mut jobs = self.lock();
        prune(&mut jobs);
        let mut id = agent_id.to_string();
        let mut n = 2;
        while jobs.contains_key(&id) {
            id = format!("{agent_id}-{n}");
            n += 1;
        }
        let (cancel, cancelled) = watch::channel(false);
        jobs.insert(
            id.clone(),
            Job {
                snapshot: JobSnapshot {
                    id: id.clone(),
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
                consumed_at: None,
            },
        );
        drop(jobs);
        self.bump();
        (id, cancelled)
    }

    pub fn report_progress(&self, job_id: &str, progress: Value) {
        if let Some(job) = self.lock().get_mut(job_id) {
            if job.snapshot.status == JobStatus::Running {
                job.snapshot.progress = Some(progress);
            }
        }
        self.bump();
    }

    /// Settle a running job. A cancelled job keeps `cancelled` and is not delivered.
    pub fn settle(&self, job_id: &str, outcome: Result<String, String>) {
        let notify = {
            let mut jobs = self.lock();
            let Some(job) = jobs.get_mut(job_id) else {
                return;
            };
            if job.snapshot.status != JobStatus::Running {
                None
            } else {
                let (status, text) = match outcome {
                    Ok(text) => (JobStatus::Completed, text),
                    Err(text) => (JobStatus::Failed, text),
                };
                job.snapshot.status = status;
                job.snapshot.result = Some(text);
                job.snapshot.finished_at = Some(now_ms());
                (job.watchers == 0)
                    .then(|| (job.snapshot.root.clone(), job.snapshot.owner_id.clone()))
            }
        };
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
                Some(job)
                    if job.snapshot.root == root
                        && job.snapshot.owner_id == owner_id
                        && job.snapshot.status == JobStatus::Running =>
                {
                    job.snapshot.status = JobStatus::Cancelled;
                    job.snapshot.finished_at = Some(now_ms());
                    let _ = job.cancel.send(true);
                    true
                }
                _ => false,
            }
        };
        if cancelled {
            self.bump();
        }
        cancelled
    }

    /// Cancel every running job of `root` (parent conversation stopped).
    pub fn cancel_root(&self, root: &str) {
        {
            let mut jobs = self.lock();
            for job in jobs.values_mut() {
                if job.snapshot.root == root && job.snapshot.status == JobStatus::Running {
                    job.snapshot.status = JobStatus::Cancelled;
                    job.snapshot.finished_at = Some(now_ms());
                    let _ = job.cancel.send(true);
                }
            }
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
        self.select(root, owner_id, |job| job.snapshot.status == JobStatus::Running)
    }

    /// Settled (completed/failed), unconsumed results of `owner_id` — what `wait` returns first.
    pub fn undelivered(&self, root: &str, owner_id: &str) -> Vec<JobSnapshot> {
        self.select(root, owner_id, |job| deliverable(job, true))
    }

    /// Delivery drain: take unwatched settled results and mark them consumed.
    pub fn take_deliverable(&self, root: &str, owner_id: &str) -> Vec<JobSnapshot> {
        let taken = {
            let mut jobs = self.lock();
            let now = now_ms();
            let mut taken: Vec<JobSnapshot> = jobs
                .values_mut()
                .filter(|job| {
                    job.snapshot.root == root
                        && job.snapshot.owner_id == owner_id
                        && deliverable(job, false)
                })
                .map(|job| {
                    let snapshot = job.snapshot.clone();
                    mark_consumed(job, now);
                    snapshot
                })
                .collect();
            taken.sort_by_key(|job| (job.finished_at, job.started_at));
            taken
        };
        if !taken.is_empty() {
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
            let now = now_ms();
            let mut claimed = Vec::new();
            let mut watched = Vec::new();
            for job in jobs.values_mut() {
                if job.snapshot.root != root || job.snapshot.owner_id != owner_id {
                    continue;
                }
                if job.snapshot.status == JobStatus::Running {
                    job.watchers += 1;
                    watched.push((job.snapshot.started_at, job.snapshot.id.clone()));
                } else if deliverable(job, true) {
                    claimed.push(job.snapshot.clone());
                    mark_consumed(job, now);
                }
            }
            claimed.sort_by_key(|job: &JobSnapshot| (job.finished_at, job.started_at));
            watched.sort();
            (claimed, watched.into_iter().map(|(_, id)| id).collect::<Vec<_>>())
        };
        if !claimed.is_empty() {
            self.bump();
        }
        (claimed, watched)
    }

    /// `wait` end: atomically claim the settled, unconsumed results among `job_ids`
    /// (the jobs this wait watched) and mark them consumed.
    pub fn take_wait_results(&self, job_ids: &[String]) -> Vec<JobSnapshot> {
        let taken = {
            let mut jobs = self.lock();
            let now = now_ms();
            let mut taken = Vec::new();
            for id in job_ids {
                if let Some(job) = jobs.get_mut(id) {
                    if deliverable(job, true) {
                        taken.push(job.snapshot.clone());
                        mark_consumed(job, now);
                    }
                }
            }
            taken.sort_by_key(|job| (job.finished_at, job.started_at));
            taken
        };
        if !taken.is_empty() {
            self.bump();
        }
        taken
    }

    /// Undo a claim whose delivery failed (e.g. persisting the parent message):
    /// the results become unconsumed again for a later `wait` or delivery. No retry here.
    pub fn restore_deliverable(&self, snapshots: &[JobSnapshot]) {
        {
            let mut jobs = self.lock();
            for snapshot in snapshots {
                if let Some(job) = jobs.get_mut(&snapshot.id) {
                    if job.snapshot.consumed && job.snapshot.status == snapshot.status {
                        job.snapshot.consumed = false;
                        job.snapshot.result = snapshot.result.clone();
                        job.consumed_at = None;
                    }
                }
            }
        }
        self.bump();
    }

    /// Mark results consumed (returned by `wait`); later snapshots omit their bodies.
    pub fn consume(&self, job_ids: &[String]) {
        let now = now_ms();
        {
            let mut jobs = self.lock();
            for id in job_ids {
                if let Some(job) = jobs.get_mut(id) {
                    if job.snapshot.status != JobStatus::Running {
                        mark_consumed(job, now);
                    }
                }
            }
        }
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
    matches!(job.snapshot.status, JobStatus::Completed | JobStatus::Failed)
        && !job.snapshot.consumed
        && (include_watched || job.watchers == 0)
}

fn mark_consumed(job: &mut Job, now: i64) {
    job.snapshot.consumed = true;
    job.snapshot.result = None;
    job.consumed_at = Some(now);
}

fn prune(jobs: &mut HashMap<String, Job>) {
    let now = now_ms();
    jobs.retain(|_, job| match (job.snapshot.finished_at, job.consumed_at) {
        (_, Some(consumed)) => now - consumed < CONSUMED_RETENTION_MS,
        (Some(finished), None) => now - finished < UNCONSUMED_RETENTION_MS,
        (None, None) => true,
    });
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
}
