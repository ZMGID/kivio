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

use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
include!("core.rs");

pub fn service(app: &AppHandle) -> tauri::State<'_, ScheduledTasks> {
    app.state::<ScheduledTasks>()
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
