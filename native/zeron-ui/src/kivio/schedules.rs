//! Scheduled sends use the same admission gate, runtime and conversation store as chat.
use super::*;
use kivio_model_runtime::scheduled_tasks::{self, ScheduledTasks, types::*};

fn scheduled_reply(
    store: &ConversationStore,
    conversation: &str,
    user_message: &str,
) -> Option<Result<(), String>> {
    let saved = match store.load(conversation) {
        Ok(saved) => saved,
        Err(e) => return Some(Err(e.to_string())),
    };
    let start = saved.messages.iter().position(|m| m.id == user_message)?;
    for message in &saved.messages[start + 1..] {
        if message.role == "assistant" {
            return Some(if message.stream_outcome.as_deref() == Some("completed") {
                Ok(())
            } else {
                Err("回复失败或已停止；请打开绑定对话查看。".into())
            });
        }
        if message.role == "user" {
            return Some(Err("该次回复缺失；请打开绑定对话查看。".into()));
        }
    }
    None
}

impl KivioService {
    pub fn scheduled_tasks(&self) -> Result<Arc<ScheduledTasks>, String> {
        self.schedule_owner.clone()
    }

    pub fn start_scheduler(self: &Arc<Self>) {
        let Ok(owner) = self.scheduled_tasks() else {
            return;
        };
        let mut slot = self.scheduler.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let service = Arc::downgrade(self);
        let stop = self.schedule_stop.clone();
        *slot = Some(tokio::spawn(async move {
            loop {
                if stop.is_cancelled() {
                    break;
                }
                let Some(service) = service.upgrade() else {
                    break;
                };
                let (fires, _) = owner.claim_due(scheduled_tasks::now_secs());
                for fire in fires {
                    let (task, at) = fire.into_parts();
                    if let Err(error) =
                        service.run_scheduled_definition(task, RunTrigger::Schedule, Some(at))
                    {
                        tracing::warn!(%error, "scheduled task admission failed");
                    }
                }
                drop(service);
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = owner.wait_for_wake() => {},
                }
            }
        }));
    }

    /// Resolve a new target before saving; existing targets retain their runtime and model.
    pub fn save_scheduled(&self, mut input: ScheduledTaskInput) -> Result<ScheduledTask, String> {
        let owner = self.scheduled_tasks()?;
        let name = scheduled_tasks::validated_name(&input)?;
        let source = input
            .id
            .as_deref()
            .map(|id| owner.get(id).map(|t| t.source))
            .transpose()?
            .unwrap_or(TaskSource::User);
        let mut created = None;
        let id = match &input.target {
            TaskTarget::Conversation { conversation_id } => {
                let id = conversation_id.trim();
                self.store
                    .load(id)
                    .map_err(|e| format!("无法读取绑定对话：{e}"))?;
                id.to_owned()
            }
            TaskTarget::NewConversation {
                provider_id,
                model,
                project_id,
                thinking_level,
            } => {
                let clean = |s: &Option<String>| {
                    s.as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                };
                let (provider_id, model) = (clean(provider_id), clean(model));
                if provider_id.is_some() != model.is_some() {
                    return Err("模型需要同时指定服务商和模型".into());
                }
                let thinking_level = clean(thinking_level);
                if thinking_level
                    .as_deref()
                    .is_some_and(|v| !matches!(v, "off" | "low" | "medium" | "high"))
                {
                    return Err("思考等级无效".into());
                }
                let project_id = clean(project_id);
                if let Some(id) = &project_id {
                    let projects =
                        group_catalog::load_project_index(&self.directory.join("conversations"))?;
                    if !projects.projects.iter().any(|p| &p.id == id) {
                        return Err("项目不存在".into());
                    }
                }
                let prefs = kivio_model_runtime::native_preferences::load(&self.directory)?;
                let runtime = if provider_id.is_some() {
                    Default::default()
                } else {
                    prefs.chat.default_agent_runtime
                };
                let (provider, model) = if runtime.kind
                    == kivio_model_runtime::conversation_data::AgentRuntimeKind::External
                {
                    (String::new(), String::new())
                } else {
                    let selection = provider_id.as_deref().zip(model.as_deref());
                    let (provider, model) = crate::providers::load_selection(
                        &self.directory.join("conversations/providers.json"),
                        selection,
                    )?;
                    (provider.id, model)
                };
                let now = scheduled_tasks::now_secs();
                let c: Conversation = serde_json::from_value(json!({
                    "id": format!("conv_{}", uuid::Uuid::new_v4()), "title": name,
                    "provider_id": provider, "model": model, "messages": [],
                    "created_at": now, "updated_at": now, "agent_runtime": runtime,
                    "project_id": project_id, "thinking_level": thinking_level,
                }))
                .map_err(|e| e.to_string())?;
                let c = self.store.create(c).map_err(|e| e.to_string())?;
                let id = c.id.clone();
                created = Some(c);
                id
            }
        };
        input.target = TaskTarget::Conversation {
            conversation_id: id,
        };
        let saved = owner.save(input, source, scheduled_tasks::now_secs());
        if saved.is_err() {
            if let Some(c) = created {
                self.store
                    .delete(&c.id, c.revision)
                    .map_err(|e| format!("任务保存失败，清理新建对话也失败：{e}"))?;
            }
        } else {
            self.catalog_changes.send_modify(|n| *n += 1);
        }
        saved
    }

    pub fn run_scheduled(&self, id: &str) -> Result<TaskRun, String> {
        let task = self.scheduled_tasks()?.get(id)?;
        self.run_scheduled_definition(task, RunTrigger::Manual, None)
    }

    fn run_scheduled_definition(
        &self,
        task: ScheduledTask,
        trigger: RunTrigger,
        at: Option<i64>,
    ) -> Result<TaskRun, String> {
        if self.schedule_stop.is_cancelled() {
            return Err("应用正在退出".into());
        }
        let owner = self.scheduled_tasks()?;
        let (run, cancel) = owner.queue_run(&task, trigger, at, scheduled_tasks::now_secs())?;
        let Some(mut cancel) = cancel else {
            return Ok(run);
        };
        let initial = run.clone();
        let service = self.clone();
        let worker = tokio::spawn(async move {
            let mut changed = service.catalog_changes.subscribe();
            let result = loop {
                if service.schedule_stop.is_cancelled() {
                    break (
                        RunStatus::Interrupted,
                        Some("应用退出时该次运行尚未完成".into()),
                        false,
                    );
                }
                if *cancel.borrow() {
                    break (
                        RunStatus::Skipped,
                        Some("任务已暂停、删除或修改，本次未发送".into()),
                        false,
                    );
                }
                let admission = tokio::select! {
                    _ = service.schedule_stop.cancelled() => continue,
                    _ = cancel.changed() => continue,
                    guard = service.send_gate.lock() => guard,
                };
                if service.schedule_stop.is_cancelled() || *cancel.borrow() {
                    drop(admission);
                    continue;
                }
                if !service
                    .runs
                    .lock()
                    .unwrap()
                    .contains_key(&task.conversation_id)
                {
                    if !owner.begin_send(&task, &run.id) {
                        break (
                            RunStatus::Skipped,
                            Some("任务已暂停、删除或修改，本次未发送".into()),
                            false,
                        );
                    }
                    let sent = service
                        .send_admitted(
                            json!({"chatId": task.conversation_id, "text": task.prompt}),
                            true,
                        )
                        .await;
                    drop(admission);
                    let message_id = match sent {
                        Err(error) => {
                            let missing = std::fs::metadata(
                                service
                                    .directory
                                    .join("conversations")
                                    .join(format!("{}.json", task.conversation_id)),
                            )
                            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
                            break (RunStatus::Failed, Some(error.to_string()), missing);
                        }
                        Ok(RpcReply::Value(reply)) => {
                            owner.mark_running(&run.id, scheduled_tasks::now_secs());
                            reply["messageId"].as_str().unwrap_or_default().to_owned()
                        }
                        Ok(_) => {
                            break (RunStatus::Failed, Some("发送未返回消息标识".into()), false);
                        }
                    };
                    // Once admitted, edits do not cancel the reply already being sent.
                    loop {
                        if scheduled_reply(&service.store, &task.conversation_id, &message_id)
                            .is_some()
                            || !service
                                .runs
                                .lock()
                                .unwrap()
                                .contains_key(&task.conversation_id)
                        {
                            break;
                        }
                        tokio::select! {
                            _ = service.schedule_stop.cancelled() => break,
                            _ = changed.changed() => {},
                        }
                    }
                    if service.schedule_stop.is_cancelled() {
                        break (
                            RunStatus::Interrupted,
                            Some("应用退出时该次运行尚未完成".into()),
                            false,
                        );
                    }
                    let outcome =
                        scheduled_reply(&service.store, &task.conversation_id, &message_id)
                            .unwrap_or_else(|| Err("回复未能保存；请打开绑定对话查看。".into()));
                    break match outcome {
                        Ok(()) => (RunStatus::Succeeded, None, false),
                        Err(error) => (RunStatus::Failed, Some(error), false),
                    };
                }
                drop(admission);
                tokio::select! {
                    _ = cancel.changed() => {},
                    _ = service.schedule_stop.cancelled() => {},
                    _ = changed.changed() => {},
                }
            };
            owner.end_send(&run.id);
            owner.finish_run(
                &task,
                &run.id,
                result.0,
                result.1,
                result.2,
                scheduled_tasks::now_secs(),
            );
        });
        let mut workers = self.schedule_workers.lock().unwrap();
        workers.retain(|w| !w.is_finished());
        workers.push(worker);
        Ok(initial)
    }
}
