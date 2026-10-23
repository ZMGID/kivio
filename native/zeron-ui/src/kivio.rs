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

mod attachments;
mod terminal;

pub const DEVICE: &str = "kivio-local";
#[derive(Clone)]
pub struct KivioService {
    directory: PathBuf,
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
impl KivioService {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        let (changes, _) = tokio::sync::watch::channel(0);
        let (catalog_changes, _) = tokio::sync::watch::channel(0);
        Arc::new(Self {
            store: Arc::new(ConversationStore::new(directory.join("conversations"))),
            directory,
            terminals: Arc::new(terminal::Terminals::new()),
            changes,
            catalog_changes,
            runs: Default::default(),
            live: Default::default(),
            tasks: Default::default(),
        })
    }
    async fn send(&self, params: Value) -> Result<RpcReply, RpcError> {
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
        let existing = self
            .store
            .load_all()
            .map_err(failed)?
            .into_iter()
            .find(|c| c.id == id);
        if existing.as_ref().is_some_and(|c| {
            c.agent_runtime.kind
                == kivio_model_runtime::conversation_data::AgentRuntimeKind::External
        }) {
            return Err(failed("此对话使用 CLI 运行时，尚未接入当前移植界面"));
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
            let batch = attachments::prepare(
                self.directory
                    .join("conversations")
                    .join(format!("{id}_attachments")),
                &params,
            )?;
            let now = chrono::Utc::now().timestamp();
            let mut c = match existing {
                Some(c) => c,
                None => {
                    let c=serde_json::from_value::<Conversation>(json!({"id":id,"title":text.chars().take(40).collect::<String>(),"provider_id":provider.id,"model":model,"messages":[],"created_at":now,"updated_at":now,"project_id":params["projectId"]})).map_err(failed)?;
                    self.store.create(c).map_err(failed)?
                }
            };
            let message_id = uuid::Uuid::new_v4().to_string();
            let message = serde_json::from_value(
                json!({"id":message_id,"role":"user","content":text,"timestamp":now,"attachments":batch.attachments()}),
            )
            .map_err(failed)?;
            c.messages.push(message);
            c.provider_id = provider.id.clone();
            c.model = model.clone();
            let revision = c.revision;
            let c = self.store.save(c, revision).map_err(failed)?;
            batch.commit();
            let run = chat_runtime::start(
                provider,
                model,
                c.messages.clone(),
                id.clone(),
                self.directory.join("conversations").join(&id),
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
            let assistant_id = uuid::Uuid::new_v4().to_string();
            let mut message: kivio_model_runtime::conversation_data::ChatMessage=serde_json::from_value(json!({
                "id":assistant_id,"role":"assistant","content":"","timestamp":chrono::Utc::now().timestamp()
            })).expect("valid assistant record");
            let outcome = loop {
                let event = tokio::select! { _=cancellation.cancelled() => break "cancelled", event=run.events.recv()=>event };
                match event {
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
            // Reload the latest durable revision so renaming during a run is preserved.
            let saved = (|| {
                let mut c = service.store.load(&id).map_err(failed)?;
                let revision = c.revision;
                c.messages.push(message.clone());
                service.store.save(c, revision).map_err(failed)
            })();
            if let Err(error) = saved {
                message
                    .content
                    .push_str(&format!("\n\n保存回复失败：{error}"));
                service.live.lock().unwrap().insert(id.clone(), message);
            } else {
                service.live.lock().unwrap().remove(&id);
            }
            service.runs.lock().unwrap().remove(&id);
            service.catalog_changes.send_modify(|n| *n += 1);
            service.changes.send_modify(|n| *n += 1);
        });
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
        RpcReply::value(&reply)
    }
    pub async fn shutdown(&self) {
        self.terminals.shutdown();
        for cancel in self.runs.lock().unwrap().values() {
            cancel.cancel();
        }
        for task in self.tasks.lock().await.drain(..) {
            let _ = task.await;
        }
    }
    fn chat(&self, c: &Conversation) -> Value {
        json!({"id":c.id,"deviceId":DEVICE,"title":c.title,"archived":c.archived,
            "createdAt":date(c.created_at),"lastMessageAt":date(c.updated_at),
            "lastSeenAt":date(c.updated_at), "spaceId":c.project_id,
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
                        parts.push(json!({"kind":"tool","id":tool.id,"call":{"kind":"unknown","name":tool.name,"input":input},
                            "resolved":resolved,"isError":matches!(tool.status, ToolCallStatus::Error | ToolCallStatus::Cancelled),
                            "output":tool.error.as_ref().or(tool.result_preview.as_ref())}));
                    }
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
            "KivioCancel" => {
                let id = params["chatId"]
                    .as_str()
                    .ok_or_else(|| RpcError::BadParams("chatId missing".into()))?;
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
            | methods::LIST_COMMANDS => RpcReply::value(&json!([])),
            methods::MUTATE => {
                let id = params["chatId"].as_str().unwrap_or("");
                match params["op"].as_str().unwrap_or("") {
                    "markChatSeen" => return RpcReply::value(&json!({})),
                    "renameChat" | "setChatArchived" | "deleteChat" => {
                        let mut c = self.store.load(id).map_err(failed)?;
                        let revision = c.revision;
                        match params["op"].as_str().unwrap() {
                            "deleteChat" => {
                                if self.runs.lock().map_err(failed)?.contains_key(id) {
                                    return Err(failed("请先停止回复再删除对话"));
                                }
                                self.store.delete(id, revision).map_err(failed)?;
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

pub fn open_provider_settings(directory: PathBuf, cx: &mut gpui_kit::App) {
    use gpui_kit::*;
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Kivio · 供应商".into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(900.), px(760.)),
            cx,
        ))),
        ..Default::default()
    };
    if let Err(error) = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| {
            crate::providers::ProviderSettings::new(
                directory.join("conversations/providers.json"),
                window,
                cx,
            )
        })
    }) {
        tracing::error!(%error,"cannot open provider settings");
    }
}
