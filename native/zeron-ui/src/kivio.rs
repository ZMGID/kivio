//! Direct bridge from the imported UI contract to Kivio's existing Rust stores.
//! No daemon, WebSocket, cloud account, or Zeron engine is started.
use async_trait::async_trait;
use futures::{StreamExt, stream};
use kivio_model_runtime::{
    conversation_data::Conversation, conversation_files::ConversationStore, group_catalog,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use zeron_rpc::{RpcError, RpcReply, RpcService, methods};

mod agent;
mod attachments;
mod cli;
mod harnesses;
mod input;
pub(crate) mod lens;
mod schedules;
mod terminal;

pub const DEVICE: &str = "kivio-local";
#[derive(Clone)]
pub struct KivioService {
    directory: PathBuf,
    schedule_owner: Result<Arc<kivio_model_runtime::scheduled_tasks::ScheduledTasks>, String>,
    schedule_stop: tokio_util::sync::CancellationToken,
    scheduler: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    schedule_workers: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    send_gate: Arc<tokio::sync::Mutex<()>>,
    native_tools: Arc<kivio_model_runtime::native_tool_runtime::NativeToolExecutor>,
    terminals: Arc<terminal::Terminals>,
    store: Arc<ConversationStore>,
    changes: tokio::sync::watch::Sender<u64>,
    catalog_changes: tokio::sync::watch::Sender<u64>,
    runs: Arc<
        std::sync::Mutex<std::collections::HashMap<String, tokio_util::sync::CancellationToken>>,
    >,
    live: Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, kivio_model_runtime::conversation_data::ChatMessage>,
        >,
    >,
    consents: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    inputs: Arc<std::sync::Mutex<std::collections::HashMap<String, input::Pending>>>,
    tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}
fn failed(error: impl std::fmt::Display) -> RpcError {
    RpcError::Failed(error.to_string())
}
fn date(t: i64) -> chrono::DateTime<chrono::Utc> {
    if t > 100_000_000_000 {
        chrono::DateTime::from_timestamp_millis(t)
    } else {
        chrono::DateTime::from_timestamp(t, 0)
    }
    .unwrap_or_default()
}
enum UserSubmission { Append(Option<String>), Resume(String) }
impl KivioService {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        let (changes, _) = tokio::sync::watch::channel(0);
        let (catalog_changes, _) = tokio::sync::watch::channel(0);
        Arc::new(Self {
            store: Arc::new(ConversationStore::new(directory.join("conversations"))),
            schedule_owner: kivio_model_runtime::scheduled_tasks::shared(directory.join("scheduled_tasks")),
            schedule_stop: Default::default(),
            scheduler: Default::default(),
            schedule_workers: Default::default(),
            send_gate: Default::default(),
            native_tools: Arc::new(kivio_model_runtime::native_tool_runtime::NativeToolExecutor::new(directory.clone())),
            directory,
            terminals: Arc::new(terminal::Terminals::new()),
            changes,
            catalog_changes,
            runs: Default::default(),
            live: Default::default(),
            tasks: Default::default(),
            inputs: Default::default(),
            consents: Default::default(),
        })
    }
    async fn send(&self, params: Value) -> Result<RpcReply, RpcError> {
        let _admission = self.send_gate.lock().await;
        self.send_admitted(params, false).await
    }

    async fn send_admitted(&self, params: Value, require_existing: bool) -> Result<RpcReply, RpcError> {
        self.send_user_admitted(params, require_existing, UserSubmission::Append(None)).await
    }
    async fn send_user_admitted(&self, params: Value, require_existing: bool, submission: UserSubmission) -> Result<RpcReply, RpcError> {
        use crate::chat_runtime::{self, ChatEvent};
        let id = params["chatId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RpcError::BadParams("chatId missing".into()))?
            .to_owned();
        let text = params["text"].as_str().unwrap_or("").to_owned();
        if text.trim().is_empty()
            && !params["attachments"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        {
            return Err(failed("消息不能为空"));
        }
        let existing = self.store.load_optional(&id).map_err(failed)?;
        if require_existing && existing.is_none() {
            return Err(failed("绑定对话不存在，定时任务不会重建已删除的对话"));
        }
        if existing.as_ref().is_some_and(|c| {
            c.agent_runtime.kind
                == kivio_model_runtime::conversation_data::AgentRuntimeKind::External
        }) || (existing.is_none() && params["runtime"].as_str().is_some_and(|r| !matches!(r, "builtin" | "chat"))) {
            return self.send_cli(params, existing).await;
        }
        let selection = existing
            .as_ref()
            .map(|c| (c.provider_id.as_str(), c.model.as_str()));
        let (provider, model) = crate::providers::load_selection(
            &self.directory.join("conversations/providers.json"),
            selection,
        )
        .map_err(failed)?;
        let cancellation = tokio_util::sync::CancellationToken::new();
        {
            let mut runs = self.runs.lock().map_err(failed)?;
            if runs.contains_key(&id) {
                return Err(failed("此对话仍在回复，请先停止当前回复"));
            }
            runs.insert(id.clone(), cancellation.clone());
        }
        let prepared = (|| {
            let (c, message_id) = match submission {
                UserSubmission::Resume(message_id) => {
                    let c = existing.ok_or_else(|| failed("待恢复的 Lens 对话不存在"))?;
                    if !c.messages.last().is_some_and(|message| message.id == message_id && message.role == "user" && message.content == text) {
                        return Err(failed("Lens 问题已被修改或已有后续消息，无法恢复旧提交"));
                    }
                    (c, message_id)
                }
                UserSubmission::Append(fixed_id) => {
                    let batch = attachments::prepare(self.directory.join("conversations").join(format!("{id}_attachments")), &params)?;
                    let now = chrono::Utc::now().timestamp();
                    let mut c = match existing {
                        Some(c) => c,
                        None => {
                            let c=serde_json::from_value::<Conversation>(json!({"id":id,"title":text.chars().take(40).collect::<String>(),"provider_id":provider.id,"model":model,"messages":[],"created_at":now,"updated_at":now,"project_id":params["projectId"],"thinking_level":params["thinkingLevel"],"agent_runtime":{"kind":if params["runtime"] == "chat" {"chat"} else {"builtin"}}})).map_err(failed)?;
                            self.store.create(c).map_err(failed)?
                        }
                    };
                    let message_id = fixed_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    if c.messages.iter().any(|message| message.id == message_id) { return Err(failed("提交消息已经存在")); }
                    let message = serde_json::from_value(json!({"id":message_id,"role":"user","content":text,"timestamp":now,"attachments":batch.attachments()})).map_err(failed)?;
                    c.messages.push(message); c.provider_id = provider.id.clone(); c.model = model.clone();
                    let revision = c.revision;
                    let c = self.store.save(c, revision).map_err(failed)?;
                    batch.commit();
                    (c, message_id)
                }
            };
            let run = chat_runtime::start_with_tools(
                provider,
                model,
                c.messages.clone(),
                id.clone(),
                self.directory.join("conversations").join(&id),
                c.thinking_level.clone(),
                c.agent_runtime.kind == kivio_model_runtime::conversation_data::AgentRuntimeKind::Chat,
                Some(self.native_tools.clone()),
            )
            .map_err(failed)?;
            Ok((c, message_id, run))
        })();
        let (c, message_id, run) = match prepared {
            Ok(value) => value,
            Err(error) => {
                self.runs.lock().unwrap().remove(&id);
                return Err(error);
            }
        };
        let reply = json!({"chat":self.chat(&c),"messageId":message_id});
        let service = self.clone();
        self.changes.send_modify(|n| *n += 1);
        self.catalog_changes.send_modify(|n| *n += 1);
        let task = tokio::spawn(async move {
            let assistant_id = run.assistant_id.clone();
            let mut message: kivio_model_runtime::conversation_data::ChatMessage=serde_json::from_value(json!({
                "id":assistant_id,"role":"assistant","content":"","timestamp":chrono::Utc::now().timestamp()
            })).expect("valid assistant record");
            let mut compaction = (None, None);
            let outcome = loop {
                let event = tokio::select! { _=cancellation.cancelled() => break "cancelled", event=run.events.recv()=>event };
                match event {
                    Ok(ChatEvent::Agent(event)) => service.agent_event(&id, &mut message, event),
                    Ok(ChatEvent::Delta(delta)) => message.content.push_str(&delta),
                    Ok(ChatEvent::Reasoning(delta)) => message
                        .reasoning
                        .get_or_insert_with(String::new)
                        .push_str(&delta),
                    Ok(ChatEvent::Finished(output)) => {
                        message.content = output.text;
                        message.reasoning = output.reasoning;
                        message.model_messages = output.provider_messages;
                        message.usage = output.usage;
                        if let Some(result) = output.agent_result {
                            compaction = (result.compaction_boundary, result.compaction_summary);
                            message.artifacts = kivio_model_runtime::agent::completion::generated_image_artifacts(&result.images);
                            message.tool_calls = result.tool_records;
                            message.segments = result.segments;
                            message.api_messages = result.api_messages;
                            message.anchor_usage = result.last_step_usage;
                            message.degraded = result.degraded;
                            message.cache_pair_input = result.cache_pairs.map(|pair| pair.0);
                            message.cache_pair_read = result.cache_pairs.map(|pair| pair.1);
                        }
                        break "completed";
                    }
                    Ok(ChatEvent::Failed(error)) => {
                        message.content.push_str(&format!("\n\n请求失败：{error}"));
                        break "error";
                    }
                    Err(_) => {
                        message.content.push_str("\n\n回复连接已中断");
                        break "error";
                    }
                }
                service
                    .live
                    .lock()
                    .unwrap()
                    .insert(id.clone(), message.clone());
                service.changes.send_modify(|n| *n += 1);
            };
            drop(run);
            message.stream_outcome = Some(outcome.into());
            service.finish_reply_with_compaction(&id, message, compaction);
        });
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
        RpcReply::value(&reply)
    }
    fn finish_reply(&self, id: &str, message: kivio_model_runtime::conversation_data::ChatMessage) {
        self.finish_reply_with_compaction(id, message, (None, None));
    }
    fn finish_reply_with_compaction(&self, id: &str, mut message: kivio_model_runtime::conversation_data::ChatMessage,
        compaction: (Option<kivio_model_runtime::conversation_data::CompactionBoundaryRecord>, Option<kivio_model_runtime::conversation_data::ConversationContextSummary>)) {
        // Reload the latest durable revision so renaming during a run is preserved.
        let saved = (|| {
            let mut c = self.store.load(id).map_err(failed)?;
            let revision = c.revision;
            kivio_model_runtime::agent::completion::apply_compaction(&mut c, compaction.0, compaction.1);
            c.messages.push(message.clone());
            self.store.save(c, revision).map_err(failed)
        })();
        if let Err(error) = saved {
            message
                .content
                .push_str(&format!("\n\n保存回复失败：{error}"));
            self.live.lock().unwrap().insert(id.to_owned(), message);
        } else {
            self.live.lock().unwrap().remove(id);
            if message.stream_outcome.as_deref() == Some("completed") && kivio_model_runtime::native_preferences::load(&self.directory).is_ok_and(|settings| settings.chat_completion_notifications) {
                crate::notify::post("Kivio", "回复已完成", Some(id));
            }
        }
        self.inputs.lock().unwrap().retain(|_, p| p.chat_id != id);
        self.runs.lock().unwrap().remove(id);
        self.catalog_changes.send_modify(|n| *n += 1);
        self.changes.send_modify(|n| *n += 1);
    }
    pub async fn shutdown(&self) {
        self.schedule_stop.cancel();
        let scheduler = self.scheduler.lock().unwrap().take();
        if let Some(task) = scheduler { let _ = task.await; }
        self.terminals.shutdown();
        for cancel in self.runs.lock().unwrap().values() {
            cancel.cancel();
        }
        self.native_tools.shutdown().await;
        for task in self.tasks.lock().await.drain(..) {
            let _ = task.await;
        }
        let workers = std::mem::take(&mut *self.schedule_workers.lock().unwrap());
        for worker in workers { let _ = worker.await; }
    }
    fn chat(&self, c: &Conversation) -> Value {
        json!({"id":c.id,"deviceId":DEVICE,"title":c.title,"archived":c.archived,
            "createdAt":date(c.created_at),"lastMessageAt":date(c.updated_at),
            "lastSeenAt":date(c.updated_at), "spaceId":c.project_id,
            "cwd":kivio_model_runtime::external_agents::native::workspace(&self.directory, &c.id, c.project_id.as_deref()).ok(),
            "config": harnesses::AGENTS.iter().find(|(agent, _)| c.agent_runtime.external_agent_id.as_deref() == Some(*agent)).map(|(_, harness)|
                json!({"harness":harness,"model":c.agent_runtime.external_model,"modelOptions":c.agent_runtime.external_model_options,"reasoning":c.agent_runtime.external_reasoning,"sandbox":c.agent_runtime.external_sandbox.as_deref().unwrap_or("workspace-write")})),
            "lastMessagePreview":c.messages.last().map(|m| m.content.chars().take(160).collect::<String>())})
    }
    fn snapshot(&self, method: &str, params: &Value) -> Result<Value, RpcError> {
        Ok(match method {
            methods::WATCH_CHATS => Value::Array(self.store.load_all().map_err(failed)?.iter().map(|c| self.chat(c)).collect()),
            methods::WATCH_SPACES => Value::Array(group_catalog::load_project_index(&self.directory.join("conversations")).map_err(failed)?.projects.iter().map(|p| json!({
                "id":p.id,"deviceId":DEVICE,"path":p.root_path.clone().unwrap_or_default(),"name":p.name,"createdAt":date(p.created_at)
            })).collect()),
            methods::WATCH_DEVICES => json!([{"id":DEVICE,"name":"此 Mac","platform":std::env::consts::OS}]),
            methods::WATCH_SESSIONS => json!(self.runs.lock().map_err(failed)?.keys().map(|id| json!({"chatId":id,"deviceId":DEVICE,"status":"working","updatedAt":chrono::Utc::now(),"startedAt":chrono::Utc::now()})).collect::<Vec<_>>()),
            methods::WATCH_TRANSFERS => json!([]),
            methods::WATCH_CONNECTIVITY => json!({"state":"disabled"}),
            methods::WATCH_SIDEBAR_PREFERENCES => json!({"revision":0,"synced":true,"initialized":true,"pinnedSessionIds":self.store.load_all().map_err(failed)?.iter().filter(|c| c.pinned).map(|c| c.id.clone()).collect::<Vec<_>>(),"sections":[]}),
            methods::AUTH_STATUS => json!({"state":"signedOut"}),
            methods::WATCH_QUEUE => json!({"items":[]}),
            methods::WATCH_DOC_MESSAGES => {
                let id=params["chatId"].as_str().ok_or_else(|| RpcError::BadParams("chatId missing".into()))?;
                let mut c=self.store.load(id).map_err(failed)?;
                if let Some(message)=self.live.lock().map_err(failed)?.get(id) { c.messages.push(message.clone()); }
                let entries=c.messages.iter().map(|m| {
                    let mut parts=vec![json!({"kind":"text","id":format!("{}-text",m.id),"text": if m.attachments.is_empty() { m.content.clone() } else {
                        let directory = self.directory.join("conversations").join(format!("{id}_attachments"));
                        let paths = m.attachments.iter().map(|a| kivio_model_runtime::attachment_files::resolve_attachment_file_path_in_dir(Some(&directory), &a.path).map(|p| p.to_string_lossy().into_owned())).collect::<Result<Vec<_>,_>>().unwrap_or_default();
                        crate::attachments::with_attachments(&m.content, &paths)
                    }})];
                    if let Some(reasoning)=&m.reasoning { parts.insert(0,json!({"kind":"reasoning","id":format!("{}-reasoning",m.id),"text":reasoning})); }
                    for tool in &m.tool_calls {
                        use kivio_model_runtime::conversation_data::ToolCallStatus;
                        let input = serde_json::from_str::<Value>(&tool.arguments).unwrap_or_else(|_| json!({"raw":tool.arguments}));
                        let resolved = !matches!(tool.status, ToolCallStatus::Pending | ToolCallStatus::Running);
                        parts.push(json!({"kind":"tool","id":tool.id,"call":if input.get("kind").is_some() { input.clone() } else { json!({"kind":"unknown","name":tool.name,"input":input}) },
                            "resolved":resolved,"isError":matches!(tool.status, ToolCallStatus::Error | ToolCallStatus::Cancelled),
                            "output":tool.error.as_ref().or(tool.result_preview.as_ref())}));
                    }
                    if self.live.lock().unwrap().get(id).is_some_and(|live| live.id == m.id) { parts.extend(self.input_parts(id)); }
                    json!({"id":m.id,"role":m.role,"parts":parts,"createdAt":date(m.timestamp).timestamp_millis(),"deviceId":DEVICE})
                }).collect::<Vec<_>>();
                json!({"reset":entries})
            }
            _ => return Err(RpcError::UnknownMethod(method.into())),
        })
    }
    fn stream(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        // The subscription remains open. Every durable mutation invalidates its snapshot.
        let rx = if matches!(
            method,
            methods::WATCH_DOC_MESSAGES | methods::WATCH_SESSIONS
        ) {
            self.changes.subscribe()
        } else {
            self.catalog_changes.subscribe()
        };
        let initial = self.snapshot(method, &params)?;
        let service = self.clone();
        let method = method.to_string();
        let updates = stream::unfold(
            (rx, service, method, params),
            |(mut rx, service, method, params)| async move {
                rx.changed().await.ok()?;
                let value = service.snapshot(&method, &params).ok()?;
                Some((value, (rx, service, method, params)))
            },
        );
        Ok(RpcReply::Stream(
            stream::once(async { initial }).chain(updates).boxed(),
        ))
    }
}
#[async_trait]
impl RpcService for KivioService {
    async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        match method {
            methods::READ_ATTACHMENT_CHUNK => attachments::read(&self.directory, &params),
            methods::OPEN_TERMINAL => {
                let cwd = if let Some(cwd) = params["cwd"].as_str().filter(|s| !s.is_empty()) {
                    cwd.to_owned()
                } else {
                    let project = params["chatId"]
                        .as_str()
                        .and_then(|id| self.store.load(id).ok())
                        .and_then(|c| c.project_id);
                    let projects =
                        group_catalog::load_project_index(&self.directory.join("conversations"))
                            .map_err(failed)?;
                    let path = project
                        .and_then(|id| projects.projects.into_iter().find(|p| p.id == id))
                        .and_then(|p| p.root_path);
                    match path {
                        Some(path) => path,
                        None => {
                            let path = self.directory.join("workspace");
                            std::fs::create_dir_all(&path).map_err(failed)?;
                            path.to_string_lossy().into_owned()
                        }
                    }
                };
                self.terminals
                    .open(
                        cwd,
                        terminal::dimension(&params, "cols", 80)?,
                        terminal::dimension(&params, "rows", 24)?,
                    )
                    .await
            }
            methods::WRITE_TERMINAL
            | methods::RESIZE_TERMINAL
            | methods::CLOSE_TERMINAL
            | methods::SUBSCRIBE_TERMINAL => self.terminals.handle(method, params),
            "KivioSend" => self.send(params).await,
            "KivioLensHandoff" => self.lens_handoff(params).await,
            methods::QUEUE_COMMAND => self.respond_input(params),
            "KivioCancel" => {
                let id = params["chatId"]
                    .as_str()
                    .ok_or_else(|| RpcError::BadParams("chatId missing".into()))?;
                self.inputs.lock().map_err(failed)?.retain(|_, p| p.chat_id != id);
                if let Some(cancel) = self.runs.lock().map_err(failed)?.get(id) {
                    cancel.cancel();
                }
                RpcReply::value(&json!({}))
            }
            methods::WATCH_CHATS
            | methods::WATCH_SPACES
            | methods::WATCH_DEVICES
            | methods::WATCH_SESSIONS
            | methods::WATCH_CONNECTIVITY
            | methods::WATCH_TRANSFERS
            | methods::WATCH_SIDEBAR_PREFERENCES
            | methods::AUTH_STATUS
            | methods::WATCH_QUEUE
            | methods::WATCH_DOC_MESSAGES => self.stream(method, params),
            methods::LOCAL_DEVICE => RpcReply::value(&json!({"deviceId":DEVICE})),
            methods::ENGINE_INFO => RpcReply::value(
                &json!({"deviceId":DEVICE,"workspaceScope":"local","capabilities":[]}),
            ),
            methods::FOCUS_CHAT | methods::SET_VIEWPORT_FOCUS | methods::PROBE_SYNC => {
                RpcReply::value(&json!({}))
            }
            methods::LIST_HARNESSES
            | methods::LIST_MODELS
            | methods::LIST_SKILLS
            | methods::LIST_COMMANDS => self.harness_catalog(method, params).await,
            "KivioExportSet" | "KivioImportSet" => {
                let path = params["path"].as_str().ok_or_else(|| RpcError::BadParams("path missing".into()))?;
                let path = PathBuf::from(path);
                let root = self.directory.clone();
                let importing = method == "KivioImportSet";
                let set_id = params["setId"].as_str().unwrap_or_default().to_owned();
                let result = tokio::task::spawn_blocking(move || {
                    if importing {
                        let set = kivio_model_runtime::set_backup::import(&root, &path)?;
                        serde_json::to_value(set).map_err(|e| e.to_string())
                    } else {
                        kivio_model_runtime::set_backup::export(&root, &set_id, &path)?;
                        Ok(json!({}))
                    }
                }).await.map_err(failed)?.map_err(failed)?;
                if importing { self.catalog_changes.send_modify(|n| *n += 1); }
                RpcReply::value(&result)
            }
            methods::MUTATE => {
                let id = params["chatId"].as_str().unwrap_or("");
                match params["op"].as_str().unwrap_or("") {
                    "setChatConfig" => {
                        if self.runs.lock().map_err(failed)?.contains_key(id) { return Err(failed("请在本轮结束后更换模型或权限")); }
                        let mut c = self.store.load(id).map_err(failed)?;
                        let revision = c.revision;
                        harnesses::apply_config(&mut c, &params["config"])?;
                        self.store.save(c, revision).map_err(failed)?;
                        self.catalog_changes.send_modify(|n| *n += 1);
                        RpcReply::value(&json!({}))
                    }
                    "batchLibrary" => {
                        let action = params["action"].as_object().ok_or_else(|| RpcError::BadParams("action missing".into()))?;
                        if !matches!(action.get("op").and_then(Value::as_str), Some("setChatPinned" | "setChatArchived" | "deleteChat" | "moveChat")) {
                            return Err(RpcError::BadParams("unsupported library action".into()));
                        }
                        let ids = params["chatIds"].as_array().ok_or_else(|| RpcError::BadParams("chatIds missing".into()))?;
                        let mut succeeded = Vec::new();
                        let mut failed_items = Vec::new();
                        let mut seen = std::collections::HashSet::new();
                        for id in ids {
                            let id = id.as_str().ok_or_else(|| RpcError::BadParams("invalid chatId".into()))?;
                            if !seen.insert(id) { continue; }
                            let mut item = action.clone();
                            item.insert("chatId".into(), json!(id));
                            match self.handle(methods::MUTATE, Value::Object(item)).await {
                                Ok(_) => succeeded.push(id.to_owned()),
                                Err(error) => failed_items.push(json!({"id":id,"error":error.to_string()})),
                            }
                        }
                        RpcReply::value(&json!({"succeeded":succeeded,"failed":failed_items}))
                    }
                    "moveChat" => {
                        if self.runs.lock().map_err(failed)?.contains_key(id) {
                            return Err(failed("请先停止回复再移动对话"));
                        }
                        let mut conversation = self.store.load(id).map_err(failed)?;
                        let revision = conversation.revision;
                        match params["groupKind"].as_str() {
                            Some("set") => {
                                let target = params["groupId"].as_str().ok_or_else(|| RpcError::BadParams("groupId missing".into()))?;
                                let set = group_catalog::load_set_index(&self.directory.join("conversations")).map_err(failed)?.sets.into_iter().find(|set| set.id == target).ok_or_else(|| failed("集已不存在，请刷新后重试"))?;
                                conversation.set_id = Some(set.id);
                                conversation.project_id = None;
                                conversation.folder = None;
                            }
                            Some("project") => {
                                let target = params["groupId"].as_str().ok_or_else(|| RpcError::BadParams("groupId missing".into()))?;
                                let project = group_catalog::load_project_index(&self.directory.join("conversations")).map_err(failed)?.projects.into_iter().find(|project| project.id == target).ok_or_else(|| failed("项目已不存在，请刷新后重试"))?;
                                conversation.project_id = Some(project.id);
                                conversation.folder = Some(project.name);
                                conversation.set_id = None;
                            }
                            Some("none") => {
                                conversation.project_id = None;
                                conversation.folder = None;
                                conversation.set_id = None;
                            }
                            _ => return Err(RpcError::BadParams("invalid groupKind".into())),
                        }
                        use kivio_model_runtime::conversation_data::GoalStatus;
                        if let Some(goal) = conversation.goal_state.as_mut().filter(|goal| matches!(goal.status, GoalStatus::Active | GoalStatus::Verifying)) {
                            goal.version += 1;
                            goal.status = GoalStatus::Paused;
                            goal.status_reason = Some("Paused because the execution context changed".into());
                            goal.active_run_id = None;
                            goal.updated_at = chrono::Local::now().timestamp();
                        }
                        self.store.save(conversation, revision).map_err(failed)?;
                        self.catalog_changes.send_modify(|n| *n += 1);
                        RpcReply::value(&json!({}))
                    }
                    "markChatSeen" => return RpcReply::value(&json!({})),
                    "renameChat" | "setChatArchived" | "setChatPinned" | "deleteChat" => {
                        let mut c = self.store.load(id).map_err(failed)?;
                        let revision = c.revision;
                        match params["op"].as_str().unwrap() {
                            "deleteChat" => {
                                if self.runs.lock().map_err(failed)?.contains_key(id) {
                                    return Err(failed("请先停止回复再删除对话"));
                                }
                                self.store.delete(id, revision).map_err(failed)?;
                            }
                            "setChatPinned" => {
                                c.pinned = params["pinned"].as_bool().ok_or_else(|| {
                                    RpcError::BadParams("pinned missing".into())
                                })?;
                                self.store.save(c, revision).map_err(failed)?;
                            }
                            "renameChat" => {
                                c.title = params["title"]
                                    .as_str()
                                    .ok_or_else(|| RpcError::BadParams("title missing".into()))?
                                    .into();
                                self.store.save(c, revision).map_err(failed)?;
                            }
                            _ => {
                                c.archived = params["archived"].as_bool().ok_or_else(|| {
                                    RpcError::BadParams("archived missing".into())
                                })?;
                                self.store.save(c, revision).map_err(failed)?;
                            }
                        }
                        self.catalog_changes.send_modify(|n| *n += 1);
                        RpcReply::value(&json!({}))
                    }
                    _ => Err(RpcError::Failed("此操作尚未接入 Kivio Rust 业务".into())),
                }
            }
            _ => Err(RpcError::Failed(format!("Kivio 尚未接入此功能：{method}"))),
        }
    }
}

/// A built-in chat's own model and thinking level, as the header shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatChoice {
    pub provider_id: String,
    pub model: String,
    pub thinking: Option<String>,
    pub chat_mode: bool,
}

pub fn chat_choice(directory: &std::path::Path, chat_id: &str) -> Option<ChatChoice> {
    let store = ConversationStore::new(directory.join("conversations"));
    let chat = store.load(chat_id).ok()?;
    Some(ChatChoice {
        provider_id: chat.provider_id,
        model: chat.model,
        thinking: chat.thinking_level,
        chat_mode: chat.agent_runtime.kind
            == kivio_model_runtime::conversation_data::AgentRuntimeKind::Chat,
    })
}

pub fn set_chat_choice(
    directory: &std::path::Path,
    chat_id: &str,
    choice: ChatChoice,
) -> Result<(), String> {
    let store = ConversationStore::new(directory.join("conversations"));
    let mut chat = store.load(chat_id).map_err(|e| e.to_string())?;
    let revision = chat.revision;
    chat.provider_id = choice.provider_id;
    chat.model = choice.model;
    chat.thinking_level = choice.thinking;
    store.save(chat, revision).map_err(|e| e.to_string())?;
    Ok(())
}
