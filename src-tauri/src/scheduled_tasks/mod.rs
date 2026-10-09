//! 定时任务：按计划把一段提示词以用户消息发送到一条绑定的对话里。
//!
//! Independent of the canvas `automation` domain (no shared code or state).
//! Every task is bound to exactly one conversation: the editor either picks an
//! existing one or has a new one created (named after the task) when the task
//! is saved; chat tools bind the current conversation. Sending goes through
//! Chat's normal send transaction and waits in line while the conversation is
//! still replying.
//!
//! `ScheduledTasks` is the single owner of task/run state; the scheduler loop,
//! Tauri commands and chat tools all call into it.

pub mod commands;
mod rule;
mod store;
pub mod tools;
pub mod types;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use store::Store;
use types::{
    RunStatus, RunTrigger, ScheduleRule, ScheduledTask, ScheduledTaskInput, ScheduledTasksSnapshot,
    TaskRun, TaskSource, TaskStatus, TaskTarget,
};

pub const CHANGED_EVENT: &str = "scheduled-tasks-changed";
/// A fire more than this late (sleep / app closed) is recorded as skipped
/// instead of sending a stale prompt.
const MISFIRE_GRACE_SECS: i64 = 5 * 60;
const MAX_SLEEP_SECS: i64 = 60;
const MISSED_REASON: &str = "错过了计划时间（电脑休眠或应用未运行），本次未发送";
const STILL_QUEUED_REASON: &str = "上一次运行仍在等待对话空闲，本次未重复排队";
const CANCELLED_REASON: &str = "任务已暂停、删除或修改，本次未发送";

pub struct ScheduledTasks {
    store: Store,
    tasks: Mutex<Vec<ScheduledTask>>,
    runs_lock: Mutex<()>,
    wake: tokio::sync::Notify,
    /// All in-flight runs, including replies already sending while another run queues.
    active_runs: Mutex<HashMap<String, ActiveRun>>,
}

pub(crate) struct Fire {
    task: ScheduledTask,
    scheduled_at: i64,
}

struct ActiveRun {
    run: TaskRun,
    cancel: tokio::sync::watch::Sender<bool>,
    /// Admission after obtaining Chat's send reservation is the cancellation boundary.
    sending: bool,
    reply_active: bool,
}

fn same_execution(a: &ScheduledTask, b: &ScheduledTask) -> bool {
    a.revision == b.revision
        && a.prompt == b.prompt
        && a.conversation_id == b.conversation_id
        && a.schedule == b.schedule
}

impl ScheduledTasks {
    pub fn load(dir: PathBuf) -> Result<Self, String> {
        let store = Store::new(dir);
        let tasks = store.load_tasks()?;
        // A run cannot survive a restart: close out whatever was in flight.
        let now = now_secs();
        for task in &tasks {
            let mut runs = store.load_runs(&task.id)?;
            let mut changed = false;
            for run in runs.iter_mut().filter(|run| !run.status.is_terminal()) {
                run.status = RunStatus::Interrupted;
                run.error = Some("应用退出时该次运行尚未完成".into());
                run.finished_at = Some(now);
                changed = true;
            }
            if changed {
                if let Err(err) = store.save_runs(&task.id, &runs) {
                    eprintln!("[scheduled-tasks] reconcile runs failed: {err}");
                }
            }
        }
        Ok(Self {
            store,
            tasks: Mutex::new(tasks),
            runs_lock: Mutex::new(()),
            wake: tokio::sync::Notify::new(),
            active_runs: Mutex::new(HashMap::new()),
        })
    }

    pub fn list(&self) -> Vec<ScheduledTask> {
        let mut tasks = self.lock_tasks().clone();
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        tasks
    }

    pub fn get(&self, id: &str) -> Result<ScheduledTask, String> {
        self.lock_tasks()
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .ok_or_else(|| "定时任务不存在".to_string())
    }

    /// Publish only after the candidate snapshot is durable. Keep the lock across
    /// both steps so concurrent edits and the scheduler see one committed value.
    /// Invalidating edits cancel waiters inside the same lock as the durable commit.
    fn mutate_tasks<T>(
        &self,
        invalidate: Option<&str>,
        mutate: impl FnOnce(&mut Vec<ScheduledTask>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut current = self.lock_tasks();
        let mut candidate = current.clone();
        let result = mutate(&mut candidate)?;
        self.store.save_tasks(&candidate)?;
        *current = candidate;
        if let Some(id) = invalidate {
            self.cancel_waiters(id);
        }
        Ok(result)
    }

    /// Creates (no id) or fully replaces a task bound to an existing
    /// conversation. Run history and counters survive edits. A
    /// `NewConversation` target must first be resolved by [`save_task`].
    pub fn save(
        &self,
        input: ScheduledTaskInput,
        source: TaskSource,
        now: i64,
    ) -> Result<ScheduledTask, String> {
        let TaskTarget::Conversation { conversation_id } = &input.target else {
            return Err("新对话需要先创建再绑定".into());
        };
        let conversation_id = conversation_id.trim().to_string();
        if conversation_id.is_empty() {
            return Err("请选择要发送到的对话".into());
        }
        let ValidInput {
            name,
            prompt,
            schedule,
            enabled,
            next_run_at,
        } = validate_input(&input, now)?;

        let task = self.mutate_tasks(input.id.as_deref().map(str::trim), |tasks| {
            let task = match input
                .id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                Some(id) => {
                    let existing = tasks
                        .iter_mut()
                        .find(|task| task.id == id)
                        .ok_or("定时任务不存在")?;
                    existing.name = name;
                    existing.prompt = prompt;
                    existing.schedule = schedule;
                    existing.conversation_id = conversation_id;
                    existing.enabled = enabled;
                    existing.status = TaskStatus::Active;
                    existing.next_run_at = next_run_at;
                    existing.last_error = None;
                    existing.updated_at = now;
                    existing.revision = existing.revision.wrapping_add(1);
                    existing.clone()
                }
                None => {
                    let task = ScheduledTask {
                        id: format!("sched_{}", Uuid::new_v4()),
                        name,
                        prompt,
                        schedule,
                        conversation_id,
                        enabled,
                        status: TaskStatus::Active,
                        next_run_at,
                        last_run_at: None,
                        run_count: 0,
                        last_error: None,
                        source,
                        created_at: now,
                        updated_at: now,
                        revision: 0,
                    };
                    tasks.push(task.clone());
                    task
                }
            };
            Ok(task)
        })?;
        self.wake.notify_one();
        Ok(task)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.mutate_tasks(Some(id), |tasks| {
            let before = tasks.len();
            tasks.retain(|task| task.id != id);
            if tasks.len() == before {
                return Err("定时任务不存在".into());
            }
            Ok(())
        })?;
        let _runs = self.lock_runs();
        self.store.delete_runs(id);
        Ok(())
    }

    /// Re-enabling reschedules from now so a paused task never fires a stale slot.
    pub fn set_enabled(&self, id: &str, enabled: bool, now: i64) -> Result<ScheduledTask, String> {
        let task = self.mutate_tasks(Some(id), |tasks| {
            let task = tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or("定时任务不存在")?;
            if enabled && !task.enabled {
                let next = rule::next_after(&task.schedule, now)?;
                if next.is_none() {
                    return Err("计划时间已过，请编辑任务选择新的时间".into());
                }
                task.status = TaskStatus::Active;
                task.next_run_at = next;
            }
            task.enabled = enabled;
            task.revision = task.revision.wrapping_add(1);
            task.updated_at = now;
            Ok(task.clone())
        })?;
        self.wake.notify_one();
        Ok(task)
    }

    pub fn runs(&self, task_id: &str) -> Result<Vec<TaskRun>, String> {
        let _runs = self.lock_runs();
        self.store.load_runs(task_id)
    }

    pub fn delete_run(&self, task_id: &str, run_id: &str) -> Result<(), String> {
        let _runs = self.lock_runs();
        let mut runs = self.store.load_runs(task_id)?;
        let index = runs
            .iter()
            .position(|run| run.id == run_id)
            .ok_or("运行记录不存在")?;
        if !runs[index].status.is_terminal() {
            return Err("排队或运行中的记录不能删除".into());
        }
        runs.remove(index);
        self.store.save_runs(task_id, &runs)
    }

    /// Takes every due task, advances its schedule and returns what to send.
    /// Fires later than the grace window are recorded as skipped instead.
    pub(crate) fn claim_due(&self, now: i64) -> (Vec<Fire>, Vec<TaskRun>) {
        let mut fires = Vec::new();
        let mut missed = Vec::new();
        let mut current = self.lock_tasks();
        let mut tasks = current.clone();
        let mut changed = false;
        for task in tasks.iter_mut() {
            let Some(due) = task.next_run_at.filter(|due| *due <= now) else {
                continue;
            };
            if !task.enabled || task.status != TaskStatus::Active {
                continue;
            }
            changed = true;
            match rule::next_after(&task.schedule, now) {
                Ok(Some(next)) => task.next_run_at = Some(next),
                Ok(None) => {
                    task.next_run_at = None;
                    task.status = TaskStatus::Completed;
                }
                Err(err) => {
                    task.next_run_at = None;
                    task.enabled = false;
                    task.last_error = Some(err);
                    continue;
                }
            }
            task.updated_at = now;
            if now - due > MISFIRE_GRACE_SECS {
                missed.push(TaskRun {
                    id: new_run_id(),
                    task_id: task.id.clone(),
                    trigger: RunTrigger::Schedule,
                    scheduled_at: Some(due),
                    status: RunStatus::Skipped,
                    conversation_id: None,
                    error: Some(MISSED_REASON.into()),
                    created_at: now,
                    started_at: None,
                    finished_at: Some(now),
                });
            } else {
                fires.push(Fire {
                    task: task.clone(),
                    scheduled_at: due,
                });
            }
        }
        if changed {
            if let Err(err) = self.store.save_tasks(&tasks) {
                eprintln!("[scheduled-tasks] save after claim failed: {err}");
                return (Vec::new(), Vec::new());
            }
        }
        *current = tasks;
        drop(current);
        for run in &missed {
            self.record_run(run);
        }
        (fires, missed)
    }

    fn next_wake(&self) -> Option<i64> {
        self.lock_tasks()
            .iter()
            .filter(|task| task.enabled && task.status == TaskStatus::Active)
            .filter_map(|task| task.next_run_at)
            .min()
    }

    /// Lock order: tasks -> active runs -> run files. No lock crosses an await.
    pub fn snapshot(&self) -> ScheduledTasksSnapshot {
        let tasks = self.lock_tasks();
        let active = self.lock_active();
        let live_runs = active
            .values()
            .filter(|entry| tasks.iter().any(|task| task.id == entry.run.task_id))
            .map(|entry| entry.run.clone())
            .collect();
        let mut tasks = tasks.clone();
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        ScheduledTasksSnapshot { tasks, live_runs }
    }

    fn persist_run(&self, run: &TaskRun) -> Result<(), String> {
        let _runs = self.lock_runs();
        self.store.upsert_run(run)
    }

    fn record_run(&self, run: &TaskRun) {
        let tasks = self.lock_tasks();
        if !tasks.iter().any(|task| task.id == run.task_id) {
            return;
        }
        if let Err(err) = self.persist_run(run) {
            eprintln!("[scheduled-tasks] save run failed: {err}");
        }
    }

    fn queue_run(
        &self,
        task: &ScheduledTask,
        trigger: RunTrigger,
        scheduled_at: Option<i64>,
        now: i64,
    ) -> Result<(TaskRun, Option<tokio::sync::watch::Receiver<bool>>), String> {
        let mut tasks = self.lock_tasks();
        let latest = tasks
            .iter_mut()
            .find(|latest| latest.id == task.id)
            .ok_or("定时任务不存在")?;
        if !same_execution(latest, task) || (trigger == RunTrigger::Schedule && !latest.enabled) {
            return Err(CANCELLED_REASON.into());
        }
        let mut active = self.lock_active();
        let duplicate = active
            .values()
            .any(|entry| entry.run.task_id == task.id && entry.run.status == RunStatus::Queued);
        let run = TaskRun {
            id: new_run_id(),
            task_id: task.id.clone(),
            trigger,
            scheduled_at,
            status: if duplicate {
                RunStatus::Skipped
            } else {
                RunStatus::Queued
            },
            conversation_id: Some(task.conversation_id.clone()),
            error: duplicate.then(|| STILL_QUEUED_REASON.into()),
            created_at: now,
            started_at: None,
            finished_at: duplicate.then_some(now),
        };
        self.persist_run(&run)?;
        if duplicate {
            return Ok((run, None));
        }
        latest.run_count += 1;
        latest.last_run_at = Some(now);
        if let Err(err) = self.store.save_tasks(&tasks) {
            eprintln!("[scheduled-tasks] save after fire failed: {err}");
        }
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        active.insert(
            run.id.clone(),
            ActiveRun {
                run: run.clone(),
                cancel,
                sending: false,
                reply_active: false,
            },
        );
        Ok((run, Some(receiver)))
    }

    /// Called only after Chat has atomically reserved the idle conversation.
    fn begin_send(&self, task: &ScheduledTask, run_id: &str) -> bool {
        let tasks = self.lock_tasks();
        if !tasks
            .iter()
            .any(|latest| latest.id == task.id && same_execution(latest, task))
        {
            return false;
        }
        let mut active = self.lock_active();
        let Some(entry) = active.get_mut(run_id) else {
            return false;
        };
        entry.sending = true;
        entry.reply_active = true;
        true
    }

    fn end_send(&self, run_id: &str) {
        if let Some(entry) = self.lock_active().get_mut(run_id) {
            entry.reply_active = false;
        }
    }

    fn mark_running(&self, run_id: &str, now: i64) -> Option<TaskRun> {
        let tasks = self.lock_tasks();
        let mut active = self.lock_active();
        let entry = active.get_mut(run_id)?;
        entry.run.status = RunStatus::Running;
        entry.run.started_at = Some(now);
        if !tasks.iter().any(|task| task.id == entry.run.task_id) {
            return None;
        }
        if let Err(err) = self.persist_run(&entry.run) {
            eprintln!("[scheduled-tasks] save running state failed: {err}");
        }
        Some(entry.run.clone())
    }

    fn finish_run(
        &self,
        task: &ScheduledTask,
        run_id: &str,
        status: RunStatus,
        error: Option<String>,
        disable: bool,
        now: i64,
    ) -> Option<TaskRun> {
        let mut tasks = self.lock_tasks();
        let mut active = self.lock_active();
        let mut run = active.remove(run_id)?.run;
        drop(active);
        run.status = status;
        run.error = error.clone();
        run.finished_at = Some(now);
        let latest = tasks.iter_mut().find(|latest| latest.id == task.id)?;
        if let Err(err) = self.persist_run(&run) {
            eprintln!("[scheduled-tasks] save outcome failed: {err}");
        }
        // A reply from an old definition must not clear errors or disable its replacement.
        if same_execution(latest, task) && status != RunStatus::Skipped {
            latest.last_error = error;
            if disable {
                latest.enabled = false;
                latest.revision = latest.revision.wrapping_add(1);
            }
            if let Err(err) = self.store.save_tasks(&tasks) {
                eprintln!("[scheduled-tasks] save task outcome failed: {err}");
            }
            if disable {
                self.cancel_waiters(&task.id);
            }
        }
        Some(run)
    }

    /// Caller holds the task-definition lock after a successful edit commit.
    fn cancel_waiters(&self, task_id: &str) {
        self.lock_active().retain(|_, entry| {
            if entry.run.task_id != task_id || entry.sending {
                return true;
            }
            entry.cancel.send_replace(true);
            entry.run.status = RunStatus::Skipped;
            entry.run.error = Some(CANCELLED_REASON.into());
            entry.run.finished_at = Some(now_secs());
            if let Err(err) = self.persist_run(&entry.run) {
                eprintln!("[scheduled-tasks] save cancellation failed: {err}");
            }
            false
        });
    }

    /// Only actual scheduled sends/replies suppress recursive task mutations.
    pub fn is_scheduled_conversation_busy(&self, conversation_id: &str) -> bool {
        self.lock_active().values().any(|entry| {
            entry.reply_active && entry.run.conversation_id.as_deref() == Some(conversation_id)
        })
    }

    fn lock_tasks(&self) -> std::sync::MutexGuard<'_, Vec<ScheduledTask>> {
        self.tasks.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_runs(&self) -> std::sync::MutexGuard<'_, ()> {
        self.runs_lock.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_active(&self) -> std::sync::MutexGuard<'_, HashMap<String, ActiveRun>> {
        self.active_runs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}

pub fn service(app: &AppHandle) -> tauri::State<'_, ScheduledTasks> {
    app.state::<ScheduledTasks>()
}

pub fn now_secs() -> i64 {
    chrono::Local::now().timestamp()
}

fn new_run_id() -> String {
    format!("run_{}", Uuid::new_v4())
}

struct ValidInput {
    name: String,
    prompt: String,
    schedule: ScheduleRule,
    enabled: bool,
    next_run_at: Option<i64>,
}

/// Everything about a task except its conversation; checked before a new
/// conversation is created so a bad schedule never leaves an orphan behind.
fn validate_input(input: &ScheduledTaskInput, now: i64) -> Result<ValidInput, String> {
    let prompt = input.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("提示词不能为空".into());
    }
    let name = match input.name.trim() {
        "" => prompt.chars().take(24).collect(),
        name => name.to_string(),
    };
    let schedule = normalize_schedule(input.schedule.clone(), now)?;
    let enabled = input.enabled.unwrap_or(true);
    let next_run_at = rule::next_after(&schedule, now)?;
    if next_run_at.is_none() && enabled {
        return Err("计划时间已过，请选择将来的时间".into());
    }
    Ok(ValidInput {
        name,
        prompt,
        schedule,
        enabled,
        next_run_at,
    })
}

/// Editor/command entry: resolves the target to one concrete conversation
/// (creating it for `NewConversation`), then saves the task bound to it.
pub async fn save_task(
    app: &AppHandle,
    mut input: ScheduledTaskInput,
    source: TaskSource,
) -> Result<ScheduledTask, String> {
    let now = now_secs();
    let valid = validate_input(&input, now)?;
    let conversation_id = match &input.target {
        TaskTarget::NewConversation {
            provider_id,
            model,
            project_id,
            thinking_level,
        } => {
            let clean = |value: &Option<String>| {
                value
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            };
            let (provider_id, model) = (clean(provider_id), clean(model));
            if provider_id.is_some() != model.is_some() {
                return Err("模型需要同时指定服务商和模型".into());
            }
            let thinking_level = clean(thinking_level);
            if thinking_level
                .as_deref()
                .is_some_and(|level| !matches!(level, "off" | "low" | "medium" | "high"))
            {
                return Err("思考等级无效".into());
            }
            create_task_conversation(
                app,
                &valid.name,
                provider_id,
                model,
                clean(project_id),
                thinking_level,
            )
            .await?
        }
        TaskTarget::Conversation { conversation_id } => {
            let conversation_id = conversation_id.trim().to_string();
            if crate::chat::storage::load_conversation(app, &conversation_id).is_err() {
                return Err("选择的对话不存在".into());
            }
            conversation_id
        }
    };
    input.target = TaskTarget::Conversation { conversation_id };
    service(app).save(input, source, now)
}

fn normalize_schedule(schedule: ScheduleRule, now: i64) -> Result<ScheduleRule, String> {
    rule::validate(&schedule)?;
    Ok(match schedule {
        ScheduleRule::Interval { minutes, anchor_at } => ScheduleRule::Interval {
            minutes,
            anchor_at: Some(anchor_at.unwrap_or(now + i64::from(minutes) * 60)),
        },
        ScheduleRule::Weekly {
            mut weekdays,
            hour,
            minute,
        } => {
            weekdays.sort_unstable();
            weekdays.dedup();
            ScheduleRule::Weekly {
                weekdays,
                hour,
                minute,
            }
        }
        ScheduleRule::Monthly {
            mut days,
            hour,
            minute,
        } => {
            days.sort_unstable();
            days.dedup();
            ScheduleRule::Monthly { days, hour, minute }
        }
        ScheduleRule::Cron { expr } => ScheduleRule::Cron {
            expr: expr.split_whitespace().collect::<Vec<_>>().join(" "),
        },
        other => other,
    })
}

pub fn preview(schedule: &ScheduleRule) -> Result<Vec<i64>, String> {
    let now = now_secs();
    let schedule = normalize_schedule(schedule.clone(), now)?;
    rule::preview(&schedule, now, 5)
}

fn emit_changed(app: &AppHandle, task_id: &str, run: Option<&TaskRun>) {
    let _ = app.emit(
        CHANGED_EVENT,
        serde_json::json!({ "taskId": task_id, "run": run }),
    );
}

/// Scheduler loop: sleeps until the earliest next run (at most a minute, so
/// wall-clock jumps after sleep are noticed), woken early by task edits.
pub fn spawn_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let svc = service(&app);
            let notified = svc.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let now = now_secs();
            let (fires, missed) = svc.claim_due(now);
            for run in &missed {
                emit_changed(&app, &run.task_id, Some(run));
            }
            for fire in fires {
                if let Err(err) = start_run(
                    &app,
                    fire.task,
                    RunTrigger::Schedule,
                    Some(fire.scheduled_at),
                ) {
                    eprintln!("[scheduled-tasks] start run failed: {err}");
                }
            }
            let wait = svc
                .next_wake()
                .map(|next| (next - now).clamp(1, MAX_SLEEP_SECS))
                .unwrap_or(MAX_SLEEP_SECS);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait as u64)) => {}
                _ = notified => {}
            }
        }
    });
}

/// Records the run and executes it in the background. Returns the initial record.
pub fn start_run(
    app: &AppHandle,
    task: ScheduledTask,
    trigger: RunTrigger,
    scheduled_at: Option<i64>,
) -> Result<TaskRun, String> {
    let (run, receiver) = service(app).queue_run(&task, trigger, scheduled_at, now_secs())?;
    emit_changed(app, &task.id, Some(&run));
    let Some(receiver) = receiver else {
        return Ok(run);
    };
    let initial = run.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use crate::chat::commands::send::QueuedSendOutcome;
        let outcome = execute(&app, &task, &run.id, receiver).await;
        let (status, error, disable) = match outcome {
            Ok(QueuedSendOutcome::Sent) => (RunStatus::Succeeded, None, false),
            Ok(QueuedSendOutcome::Cancelled) => {
                (RunStatus::Skipped, Some(CANCELLED_REASON.into()), false)
            }
            Err(RunError { message, disable }) => (RunStatus::Failed, Some(message), disable),
        };
        if let Some(finished) =
            service(&app).finish_run(&task, &run.id, status, error, disable, now_secs())
        {
            emit_changed(&app, &task.id, Some(&finished));
        }
    });
    Ok(initial)
}

struct RunError {
    message: String,
    /// The target can never work again (bound conversation deleted).
    disable: bool,
}

async fn execute(
    app: &AppHandle,
    task: &ScheduledTask,
    run_id: &str,
    cancelled: tokio::sync::watch::Receiver<bool>,
) -> Result<crate::chat::commands::send::QueuedSendOutcome, RunError> {
    // The binding and prompt are always from the same claimed definition.
    let on_send_ready = || service(app).begin_send(task, run_id);
    let on_user_message_saved = || {
        if let Some(running) = service(app).mark_running(run_id, now_secs()) {
            emit_changed(app, &task.id, Some(&running));
        }
    };
    let on_send_finished = || service(app).end_send(run_id);
    let outcome = crate::chat::commands::send::send_user_message_when_idle(
        app,
        &task.conversation_id,
        task.prompt.clone(),
        cancelled,
        &on_send_ready,
        &on_user_message_saved,
        &on_send_finished,
    )
    .await;
    outcome.map_err(|message| RunError {
        // A binding deleted during the idle wait must also disable the task.
        disable: crate::chat::storage::load_conversation(app, &task.conversation_id).is_err(),
        message,
    })
}

/// Creates the conversation a new task is bound to, named after the task.
async fn create_task_conversation(
    app: &AppHandle,
    name: &str,
    provider_id: Option<String>,
    model: Option<String>,
    project_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<String, String> {
    let state = app.state::<crate::state::AppState>();
    let pinned_model = provider_id.is_some() && model.is_some();
    let conversation = crate::chat::commands::catalog::create_chat_conversation_internal(
        app,
        state.inner(),
        provider_id,
        model,
        None,
        project_id,
        None,
        None,
        false,
    )
    .await?;
    let title = name.to_string();
    crate::chat::repository::repository(app)
        .mutate(app, &conversation.id, move |latest| {
            latest.title = title;
            latest.thinking_level = thinking_level;
            if pinned_model {
                latest.agent_runtime = crate::chat::types::AgentRuntimeConfig::default();
            }
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    Ok(conversation.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service_in(dir: &std::path::Path) -> ScheduledTasks {
        ScheduledTasks::load(dir.to_path_buf()).unwrap()
    }

    fn input(schedule: ScheduleRule) -> ScheduledTaskInput {
        ScheduledTaskInput {
            id: None,
            name: "日报".into(),
            prompt: "总结今天的进展".into(),
            schedule,
            target: TaskTarget::Conversation {
                conversation_id: "conv_1".into(),
            },
            enabled: None,
        }
    }

    #[test]
    fn due_task_fires_once_and_advances() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Interval {
                    minutes: 10,
                    anchor_at: None,
                }),
                TaskSource::User,
                1_000,
            )
            .unwrap();
        assert_eq!(task.next_run_at, Some(1_600));

        let (fires, missed) = svc.claim_due(1_599);
        assert!(fires.is_empty() && missed.is_empty());

        let (fires, missed) = svc.claim_due(1_610);
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].scheduled_at, 1_600);
        assert!(missed.is_empty());
        assert_eq!(svc.get(&task.id).unwrap().next_run_at, Some(2_200));
        assert!(
            svc.claim_due(1_620).0.is_empty(),
            "the same slot must not fire twice"
        );
    }

    #[test]
    fn long_missed_slot_is_recorded_as_skipped_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Once { at: 5_000 }),
                TaskSource::User,
                1_000,
            )
            .unwrap();

        let (fires, missed) = svc.claim_due(5_000 + MISFIRE_GRACE_SECS + 1);
        assert!(fires.is_empty());
        assert_eq!(missed.len(), 1);
        assert_eq!(missed[0].status, RunStatus::Skipped);
        let task = svc.get(&task.id).unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.next_run_at, None);
        assert_eq!(svc.runs(&task.id).unwrap().len(), 1);
    }

    #[test]
    fn paused_task_does_not_fire_and_resumes_from_now() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Interval {
                    minutes: 10,
                    anchor_at: Some(1_600),
                }),
                TaskSource::User,
                1_000,
            )
            .unwrap();
        svc.set_enabled(&task.id, false, 1_100).unwrap();
        assert!(svc.claim_due(1_700).0.is_empty());

        let resumed = svc.set_enabled(&task.id, true, 5_000).unwrap();
        assert_eq!(resumed.next_run_at, Some(5_200));
    }

    #[test]
    fn past_one_shot_cannot_be_saved_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        assert!(svc
            .save(
                input(ScheduleRule::Once { at: 500 }),
                TaskSource::User,
                1_000
            )
            .is_err());
    }

    #[test]
    fn edit_keeps_counters_and_restart_interrupts_inflight_runs() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1_000,
            )
            .unwrap();
        let (run, _) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1_010)
            .unwrap();
        assert!(svc.begin_send(&task, &run.id));
        svc.mark_running(&run.id, 1_011).unwrap();
        let mut edit = input(ScheduleRule::Daily {
            hour: 10,
            minute: 0,
        });
        edit.id = Some(task.id.clone());
        let edited = svc.save(edit, TaskSource::User, 1_020).unwrap();
        assert_eq!(edited.run_count, 1);
        assert_eq!(edited.created_at, 1_000);

        let reloaded = service_in(dir.path());
        assert_eq!(reloaded.list().len(), 1);
        assert_eq!(
            reloaded.runs(&task.id).unwrap()[0].status,
            RunStatus::Interrupted
        );
    }

    #[test]
    fn service_only_saves_tasks_bound_to_a_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let mut unresolved = input(ScheduleRule::Daily { hour: 9, minute: 0 });
        unresolved.target = TaskTarget::NewConversation {
            provider_id: None,
            model: None,
            project_id: None,
            thinking_level: None,
        };
        assert!(svc.save(unresolved, TaskSource::User, 1_000).is_err());
        assert!(svc.list().is_empty());
    }
    #[test]
    fn failed_schedule_save_must_not_activate_task() {
        let dir = tempfile::tempdir().unwrap();
        let service = service_in(dir.path());
        // A directory at the destination deterministically fails the atomic rename.
        std::fs::create_dir(dir.path().join("tasks.json")).unwrap();
        let result = service.save(
            ScheduledTaskInput {
                id: None,
                name: "failed task".into(),
                prompt: "must not run".into(),
                schedule: ScheduleRule::Interval {
                    minutes: 1,
                    anchor_at: Some(1060),
                },
                target: TaskTarget::Conversation {
                    conversation_id: "conv_test".into(),
                },
                enabled: Some(true),
            },
            TaskSource::User,
            1000,
        );
        assert!(result.is_err());
        assert!(
            service.list().is_empty(),
            "failed save left an enabled task in memory: {:?}",
            service.list()
        );
    }

    #[test]
    fn failed_task_edits_leave_committed_state_and_allow_retry() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Once { at: 2000 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let before = serde_json::to_value(svc.list()).unwrap();
        let path = dir.path().join("tasks.json");
        let backup = dir.path().join("committed.json");
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        let mut edit = input(ScheduleRule::Once { at: 3000 });
        edit.id = Some(task.id.clone());
        edit.prompt = "new prompt".into();
        assert!(svc.save(edit.clone(), TaskSource::User, 1100).is_err());
        assert!(svc.set_enabled(&task.id, false, 1100).is_err());
        assert!(svc.delete(&task.id).is_err());
        assert!(svc.claim_due(2000).0.is_empty());
        assert_eq!(serde_json::to_value(svc.list()).unwrap(), before);
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&backup, &path).unwrap();
        assert_eq!(
            serde_json::to_value(service_in(dir.path()).list()).unwrap(),
            before
        );
        svc.save(edit, TaskSource::User, 1100).unwrap();
        assert_eq!(
            service_in(dir.path()).get(&task.id).unwrap().prompt,
            "new prompt"
        );
        svc.set_enabled(&task.id, false, 1200).unwrap();
        assert!(!service_in(dir.path()).get(&task.id).unwrap().enabled);
        svc.delete(&task.id).unwrap();
        assert!(service_in(dir.path()).list().is_empty());
    }

    #[test]
    fn old_completion_keeps_the_next_waiter_and_busy_guard() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let (first, _) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1010)
            .unwrap();
        assert!(!svc.is_scheduled_conversation_busy("conv_1"));
        assert!(svc.begin_send(&task, &first.id));
        svc.mark_running(&first.id, 1011).unwrap();
        let (second, _) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1012)
            .unwrap();
        assert_eq!(svc.snapshot().live_runs.len(), 2);
        svc.finish_run(&task, &first.id, RunStatus::Succeeded, None, false, 1013)
            .unwrap();
        assert!(!svc.is_scheduled_conversation_busy("conv_1"));
        let (duplicate, receiver) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1014)
            .unwrap();
        assert!(receiver.is_none());
        assert_eq!(duplicate.status, RunStatus::Skipped);
        assert_eq!(svc.snapshot().live_runs[0].id, second.id);
        assert!(svc.begin_send(&task, &second.id));
        assert!(svc.is_scheduled_conversation_busy("conv_1"));
    }

    #[test]
    fn pausing_cancels_waiters_but_preserves_an_admitted_send() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let (running, _) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1010)
            .unwrap();
        assert!(svc.begin_send(&task, &running.id));
        svc.mark_running(&running.id, 1011).unwrap();
        let (waiting, receiver) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1012)
            .unwrap();
        svc.set_enabled(&task.id, false, 1013).unwrap();
        assert!(*receiver.unwrap().borrow());
        assert!(!svc.begin_send(&task, &waiting.id));
        assert_eq!(svc.snapshot().live_runs[0].id, running.id);
        assert_eq!(svc.runs(&task.id).unwrap()[0].status, RunStatus::Skipped);
        svc.finish_run(&task, &running.id, RunStatus::Succeeded, None, false, 1014)
            .unwrap();
        assert!(svc.snapshot().live_runs.is_empty());
        assert!(!svc.get(&task.id).unwrap().enabled);
    }

    #[test]
    fn editing_cancels_old_snapshot_even_when_edit_timestamps_match() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let (waiting, receiver) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1000)
            .unwrap();
        let mut edit = input(task.schedule.clone());
        edit.id = Some(task.id.clone());
        edit.prompt = "new prompt".into();
        edit.target = TaskTarget::Conversation {
            conversation_id: "conv_2".into(),
        };
        let updated = svc.save(edit, TaskSource::User, 1000).unwrap();
        assert!(*receiver.unwrap().borrow());
        assert!(!svc.begin_send(&task, &waiting.id));
        assert!(svc
            .queue_run(&task, RunTrigger::Manual, None, 1000)
            .is_err());
        let (new_run, _) = svc
            .queue_run(&updated, RunTrigger::Manual, None, 1000)
            .unwrap();
        assert_eq!(new_run.conversation_id.as_deref(), Some("conv_2"));
        assert!(svc.begin_send(&updated, &new_run.id));
        assert!(svc.is_scheduled_conversation_busy("conv_2"));
        assert!(!svc.is_scheduled_conversation_busy("conv_1"));
    }

    #[test]
    fn deletion_cancels_waiters_and_late_completion_cannot_recreate_history() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let (running, _) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1010)
            .unwrap();
        assert!(svc.begin_send(&task, &running.id));
        svc.mark_running(&running.id, 1011).unwrap();
        let (waiting, receiver) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1012)
            .unwrap();
        svc.delete(&task.id).unwrap();
        assert!(*receiver.unwrap().borrow());
        assert!(!svc.begin_send(&task, &waiting.id));
        assert!(svc.is_scheduled_conversation_busy("conv_1"));
        assert!(svc
            .finish_run(&task, &running.id, RunStatus::Succeeded, None, false, 1013)
            .is_none());
        svc.record_run(&running);
        assert!(!svc.is_scheduled_conversation_busy("conv_1"));
        assert!(svc.snapshot().tasks.is_empty());
        assert!(svc.snapshot().live_runs.is_empty());
        assert!(!dir
            .path()
            .join("runs")
            .join(format!("{}.json", task.id))
            .exists());
    }

    #[test]
    fn failed_pause_or_edit_does_not_cancel_committed_waiter() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let (waiting, receiver) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1010)
            .unwrap();
        let path = dir.path().join("tasks.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(svc.set_enabled(&task.id, false, 1011).is_err());
        assert!(svc.delete(&task.id).is_err());
        assert!(!*receiver.unwrap().borrow());
        assert!(svc.begin_send(&task, &waiting.id));
        assert_eq!(svc.runs(&task.id).unwrap()[0].status, RunStatus::Queued);
    }

    #[test]
    fn old_reply_completion_does_not_disable_an_edited_binding() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let (run, _) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1010)
            .unwrap();
        assert!(svc.begin_send(&task, &run.id));
        svc.mark_running(&run.id, 1011).unwrap();
        svc.end_send(&run.id);
        // The next user turn can mutate schedules before outcome persistence completes.
        assert!(!svc.is_scheduled_conversation_busy("conv_1"));
        let mut edit = input(task.schedule.clone());
        edit.id = Some(task.id.clone());
        edit.target = TaskTarget::Conversation {
            conversation_id: "conv_2".into(),
        };
        svc.save(edit, TaskSource::User, 1012).unwrap();
        svc.finish_run(
            &task,
            &run.id,
            RunStatus::Failed,
            Some("old binding deleted".into()),
            true,
            1013,
        )
        .unwrap();
        let latest = svc.get(&task.id).unwrap();
        assert!(latest.enabled);
        assert_eq!(latest.last_error, None);
        assert_eq!(latest.conversation_id, "conv_2");
        assert_eq!(svc.runs(&task.id).unwrap()[0].status, RunStatus::Failed);
    }

    #[test]
    fn deleted_binding_failure_cancels_the_next_waiter() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        let task = svc
            .save(
                input(ScheduleRule::Daily { hour: 9, minute: 0 }),
                TaskSource::User,
                1000,
            )
            .unwrap();
        let (running, _) = svc
            .queue_run(&task, RunTrigger::Manual, None, 1010)
            .unwrap();
        assert!(svc.begin_send(&task, &running.id));
        svc.mark_running(&running.id, 1011).unwrap();
        let (waiting, receiver) = svc
            .queue_run(&task, RunTrigger::Schedule, Some(1012), 1012)
            .unwrap();
        svc.finish_run(
            &task,
            &running.id,
            RunStatus::Failed,
            Some("binding deleted".into()),
            true,
            1013,
        )
        .unwrap();
        assert!(*receiver.unwrap().borrow());
        assert!(!svc.begin_send(&task, &waiting.id));
        assert!(!svc.get(&task.id).unwrap().enabled);
        assert!(svc.snapshot().live_runs.is_empty());
        assert_eq!(svc.runs(&task.id).unwrap()[0].status, RunStatus::Skipped);
    }
}
