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
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use store::Store;
use types::{
    RunStatus, RunTrigger, ScheduleRule, ScheduledTask, ScheduledTaskInput, TaskRun, TaskSource,
    TaskStatus, TaskTarget,
};

pub const CHANGED_EVENT: &str = "scheduled-tasks-changed";
/// A fire more than this late (sleep / app closed) is recorded as skipped
/// instead of sending a stale prompt.
const MISFIRE_GRACE_SECS: i64 = 5 * 60;
const MAX_SLEEP_SECS: i64 = 60;
const MISSED_REASON: &str = "错过了计划时间（电脑休眠或应用未运行），本次未发送";
const STILL_QUEUED_REASON: &str = "上一次运行仍在等待对话空闲，本次未重复排队";

pub struct ScheduledTasks {
    store: Store,
    tasks: Mutex<Vec<ScheduledTask>>,
    runs_lock: Mutex<()>,
    wake: tokio::sync::Notify,
    /// Tasks whose bound-conversation run is still waiting in line.
    queued_tasks: Mutex<HashMap<String, String>>,
    /// Conversations currently handling a scheduled prompt (recursion guard for chat tools).
    busy_conversations: Mutex<HashMap<String, usize>>,
}

pub(crate) struct Fire {
    task: ScheduledTask,
    scheduled_at: i64,
}

impl ScheduledTasks {
    pub fn load(dir: PathBuf) -> Self {
        let store = Store::new(dir);
        let tasks = store.load_tasks();
        // A run cannot survive a restart: close out whatever was in flight.
        let now = now_secs();
        for task in &tasks {
            let mut runs = store.load_runs(&task.id);
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
        Self {
            store,
            tasks: Mutex::new(tasks),
            runs_lock: Mutex::new(()),
            wake: tokio::sync::Notify::new(),
            queued_tasks: Mutex::new(HashMap::new()),
            busy_conversations: Mutex::new(HashMap::new()),
        }
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
    fn mutate_tasks<T>(
        &self,
        mutate: impl FnOnce(&mut Vec<ScheduledTask>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut current = self.lock_tasks();
        let mut candidate = current.clone();
        let result = mutate(&mut candidate)?;
        self.store.save_tasks(&candidate)?;
        *current = candidate;
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

        let task = self.mutate_tasks(|tasks| {
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
        self.mutate_tasks(|tasks| {
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
        let task = self.mutate_tasks(|tasks| {
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
            task.updated_at = now;
            Ok(task.clone())
        })?;
        self.wake.notify_one();
        Ok(task)
    }

    pub fn runs(&self, task_id: &str) -> Vec<TaskRun> {
        let _runs = self.lock_runs();
        self.store.load_runs(task_id)
    }

    pub fn delete_run(&self, task_id: &str, run_id: &str) -> Result<(), String> {
        let _runs = self.lock_runs();
        let mut runs = self.store.load_runs(task_id);
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

    fn record_run(&self, run: &TaskRun) {
        let _runs = self.lock_runs();
        if let Err(err) = self.store.upsert_run(run) {
            eprintln!("[scheduled-tasks] save run failed: {err}");
        }
    }

    fn note_fired(&self, task_id: &str, now: i64) {
        let mut tasks = self.lock_tasks();
        if let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) {
            task.run_count += 1;
            task.last_run_at = Some(now);
            if let Err(err) = self.store.save_tasks(&tasks) {
                eprintln!("[scheduled-tasks] save after fire failed: {err}");
            }
        }
    }

    fn note_outcome(&self, task_id: &str, error: Option<String>, disable: bool) {
        let mut tasks = self.lock_tasks();
        if let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) {
            task.last_error = error;
            if disable {
                task.enabled = false;
            }
            if let Err(err) = self.store.save_tasks(&tasks) {
                eprintln!("[scheduled-tasks] save outcome failed: {err}");
            }
        }
    }

    /// True while `conversation_id` is handling (or waiting to handle) a scheduled prompt.
    pub fn is_scheduled_conversation_busy(&self, conversation_id: &str) -> bool {
        self.lock_busy().contains_key(conversation_id)
    }

    fn enter_conversation(&self, conversation_id: &str) {
        *self
            .lock_busy()
            .entry(conversation_id.to_string())
            .or_default() += 1;
    }

    fn leave_conversation(&self, conversation_id: &str) {
        let mut busy = self.lock_busy();
        if let Some(count) = busy.get_mut(conversation_id) {
            *count -= 1;
            if *count == 0 {
                busy.remove(conversation_id);
            }
        }
    }

    fn lock_tasks(&self) -> std::sync::MutexGuard<'_, Vec<ScheduledTask>> {
        self.tasks.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_runs(&self) -> std::sync::MutexGuard<'_, ()> {
        self.runs_lock.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_busy(&self) -> std::sync::MutexGuard<'_, HashMap<String, usize>> {
        self.busy_conversations
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    fn claim_queue(&self, task_id: &str, run_id: &str) -> bool {
        let mut queued = self.lock_queued();
        if queued.contains_key(task_id) {
            return false;
        }
        queued.insert(task_id.to_string(), run_id.to_string());
        true
    }

    fn release_queue(&self, task_id: &str, run_id: &str) {
        let mut queued = self.lock_queued();
        if queued.get(task_id).is_some_and(|owner| owner == run_id) {
            queued.remove(task_id);
        }
    }

    fn lock_queued(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.queued_tasks
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
                start_run(
                    &app,
                    fire.task,
                    RunTrigger::Schedule,
                    Some(fire.scheduled_at),
                );
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
) -> TaskRun {
    let svc = service(app);
    let now = now_secs();
    let mut run = TaskRun {
        id: new_run_id(),
        task_id: task.id.clone(),
        trigger,
        scheduled_at,
        status: RunStatus::Queued,
        conversation_id: None,
        error: None,
        created_at: now,
        started_at: None,
        finished_at: None,
    };
    // Every task sends into one conversation; while a run is still waiting
    // for it, further fires are skipped instead of piling up.
    if !svc.claim_queue(&task.id, &run.id) {
        run.status = RunStatus::Skipped;
        run.error = Some(STILL_QUEUED_REASON.into());
        run.finished_at = Some(now);
        svc.record_run(&run);
        emit_changed(app, &task.id, Some(&run));
        return run;
    }
    svc.note_fired(&task.id, now);
    svc.record_run(&run);
    emit_changed(app, &task.id, Some(&run));

    let app = app.clone();
    let initial = run.clone();
    tauri::async_runtime::spawn(async move {
        let started_at = AtomicI64::new(0);
        let outcome = execute(&app, &task, &mut run, &started_at).await;
        let svc = service(&app);
        svc.release_queue(&task.id, &run.id);
        run.started_at = match started_at.load(Ordering::SeqCst) {
            0 => None,
            value => Some(value),
        };
        run.finished_at = Some(now_secs());
        let (error, disable) = match outcome {
            Ok(()) => {
                run.status = RunStatus::Succeeded;
                (None, false)
            }
            Err(RunError { message, disable }) => {
                run.status = RunStatus::Failed;
                run.error = Some(message.clone());
                (Some(message), disable)
            }
        };
        svc.record_run(&run);
        svc.note_outcome(&task.id, error, disable);
        emit_changed(&app, &task.id, Some(&run));
    });
    initial
}

struct RunError {
    message: String,
    /// The target can never work again (bound conversation deleted).
    disable: bool,
}

impl From<String> for RunError {
    fn from(message: String) -> Self {
        Self {
            message,
            disable: false,
        }
    }
}

async fn execute(
    app: &AppHandle,
    task: &ScheduledTask,
    run: &mut TaskRun,
    started_at: &AtomicI64,
) -> Result<(), RunError> {
    // Read the binding at run time: the task may have been edited since it fired.
    let conversation_id = service(app)
        .get(&task.id)
        .map(|latest| latest.conversation_id)
        .unwrap_or_else(|_| task.conversation_id.clone());
    if crate::chat::storage::load_conversation(app, &conversation_id).is_err() {
        return Err(RunError {
            message: "绑定的对话已不存在，任务已停用".into(),
            disable: true,
        });
    }
    run.conversation_id = Some(conversation_id.clone());
    let svc = service(app);
    svc.record_run(run);
    emit_changed(app, &task.id, Some(run));

    let running = {
        let mut running = run.clone();
        running.status = RunStatus::Running;
        running
    };
    let on_user_message_saved = || {
        let now = now_secs();
        started_at.store(now, Ordering::SeqCst);
        let svc = service(app);
        svc.release_queue(&task.id, &running.id);
        let mut running = running.clone();
        running.started_at = Some(now);
        svc.record_run(&running);
        emit_changed(app, &task.id, Some(&running));
    };

    svc.enter_conversation(&conversation_id);
    let outcome = crate::chat::commands::send::send_user_message_when_idle(
        app,
        &conversation_id,
        task.prompt.clone(),
        &on_user_message_saved,
    )
    .await;
    service(app).leave_conversation(&conversation_id);
    outcome.map_err(RunError::from)
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
        ScheduledTasks::load(dir.to_path_buf())
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
        assert_eq!(svc.runs(&task.id).len(), 1);
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
        svc.note_fired(&task.id, 1_010);
        svc.record_run(&TaskRun {
            id: "run_1".into(),
            task_id: task.id.clone(),
            trigger: RunTrigger::Manual,
            scheduled_at: None,
            status: RunStatus::Running,
            conversation_id: Some("conv_1".into()),
            error: None,
            created_at: 1_010,
            started_at: Some(1_011),
            finished_at: None,
        });
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
        assert_eq!(reloaded.runs(&task.id)[0].status, RunStatus::Interrupted);
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
        // A directory at the destination deterministically fails the atomic rename.
        std::fs::create_dir(dir.path().join("tasks.json")).unwrap();
        let service = ScheduledTasks::load(dir.path().to_path_buf());
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
    fn old_run_completion_cannot_release_a_new_waiter() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service_in(dir.path());
        assert!(svc.claim_queue("task", "run_a"));
        assert!(!svc.claim_queue("task", "run_b"));
        svc.release_queue("task", "run_a"); // A begins sending.
        assert!(svc.claim_queue("task", "run_b"));
        svc.release_queue("task", "run_a"); // A finishes while B still waits.
        assert!(!svc.claim_queue("task", "run_c"));
        assert!(svc.claim_queue("other_task", "run_c"));
        svc.release_queue("task", "run_b");
        assert!(svc.claim_queue("task", "run_c"));
    }
}
