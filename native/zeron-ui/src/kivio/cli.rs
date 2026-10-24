//! The imported UI uses the existing CLI protocol and Kivio conversation store.
use super::*;
use kivio_model_runtime::{
    conversation_data::{AgentRuntimeKind, ChatMessage, ToolCallStatus},
    external_agents::{native::Launch, types::UnifiedAgentEvent as Event},
};
use std::collections::HashMap;

impl KivioService {
    pub(super) async fn send_cli(
        &self,
        params: Value,
        existing: Option<Conversation>,
    ) -> Result<RpcReply, RpcError> {
        let id = params["chatId"]
            .as_str()
            .ok_or_else(|| failed("chatId missing"))?
            .to_owned();
        let text = params["text"].as_str().unwrap_or("").to_owned();
        let agent = existing
            .as_ref()
            .and_then(|c| c.agent_runtime.external_agent_id.as_deref())
            .unwrap_or_else(|| params["runtime"].as_str().unwrap_or(""));
        let project_id = existing
            .as_ref()
            .and_then(|c| c.project_id.as_deref())
            .or(params["projectId"].as_str());
        let driver = super::harnesses::driver(&self.directory, agent).map_err(failed)?;
        let launch = Launch::prepare_resolved(
            &self.directory,
            &id,
            agent,
            project_id,
            existing.is_none(),
            driver.executable_path(),
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
        let agent = agent.to_owned();
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
                None => self.store.create(serde_json::from_value(json!({
                    "id":id,"title":text.chars().take(40).collect::<String>(),"provider_id":"","model":"",
                    "messages":[],"created_at":now,"updated_at":now,"project_id":params["projectId"],
                    "agent_runtime":{"kind":"external","externalAgentId":agent,"externalSandbox":"workspace-write"}
                })).map_err(failed)?).map_err(failed)?,
            };
            if c.agent_runtime.kind != AgentRuntimeKind::External {
                return Err(failed("不能更换已有对话的运行时"));
            }
            if let Some(config) = params.get("cliConfig").filter(|v| !v.is_null()) {
                super::harnesses::apply_config(&mut c, config)?;
            }
            let message_id = uuid::Uuid::new_v4().to_string();
            c.messages.push(serde_json::from_value(json!({"id":message_id,"role":"user","content":text,"timestamp":now,"attachments":batch.attachments()})).map_err(failed)?);
            let revision = c.revision;
            let c = self.store.save(c, revision).map_err(failed)?;
            batch.commit();
            Ok((c, message_id))
        })();
        let (conversation, message_id) = match prepared {
            Ok(v) => v,
            Err(e) => {
                self.runs.lock().unwrap().remove(&id);
                return Err(e);
            }
        };
        let reply = json!({"chat":self.chat(&conversation),"messageId":message_id});
        let service = self.clone();
        self.changes.send_modify(|n| *n += 1);
        self.catalog_changes.send_modify(|n| *n += 1);
        let task = tokio::spawn(async move {
            let (events_tx, mut events) = tokio::sync::mpsc::channel(128);
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let mut cancel_tx = Some(cancel_tx);
            let call = async {
                if conversation.agent_runtime.external_agent_id.as_deref() == Some("codex") {
                    service
                        .run_codex(&conversation, launch, events_tx, cancel_rx)
                        .await
                } else {
                    super::harnesses::run(
                        service.clone(),
                        conversation.clone(),
                        launch,
                        events_tx,
                        cancel_rx,
                    )
                    .await
                }
            };
            tokio::pin!(call);
            let mut message: ChatMessage = serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"role":"assistant","content":"","timestamp":chrono::Utc::now().timestamp()})).unwrap();
            service
                .live
                .lock()
                .unwrap()
                .insert(id.clone(), message.clone());
            let mut items = Vec::new();
            let mut indices = HashMap::new();
            let mut failure = None;
            let result = loop {
                tokio::select! {
                    _ = cancellation.cancelled(), if cancel_tx.is_some() => { let _ = cancel_tx.take().unwrap().send(()); }
                    result = &mut call => break result,
                    Some(event) = events.recv() => {
                        project(event, &mut message, &mut items, &mut indices, &mut failure);
                        service.live.lock().unwrap().insert(id.clone(), message.clone());
                        service.changes.send_modify(|n| *n += 1);
                    }
                }
            };
            while let Ok(event) = events.try_recv() {
                project(event, &mut message, &mut items, &mut indices, &mut failure);
            }
            let outcome = if cancellation.is_cancelled() {
                "cancelled"
            } else if result.is_err() || failure.is_some() {
                "error"
            } else {
                "completed"
            };
            if outcome == "error" {
                message.content.push_str(&format!(
                    "\n\nCLI 请求失败：{}",
                    failure.or_else(|| result.err()).unwrap_or_default()
                ));
            }
            for tool in &mut message.tool_calls {
                if matches!(
                    tool.status,
                    ToolCallStatus::Pending | ToolCallStatus::Running
                ) {
                    tool.status = if outcome == "cancelled" {
                        ToolCallStatus::Cancelled
                    } else {
                        ToolCallStatus::Error
                    };
                    tool.error = Some("CLI 未返回此工具的完成结果".into());
                }
            }
            message.stream_outcome = Some(outcome.into());
            service.finish_reply(&id, message);
        });
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|t| !t.is_finished());
        tasks.push(task);
        RpcReply::value(&reply)
    }
}

fn project(
    event: Event,
    message: &mut ChatMessage,
    items: &mut Vec<String>,
    indices: &mut HashMap<String, usize>,
    failure: &mut Option<String>,
) {
    match event {
        Event::TextDelta { delta } => {
            items.push(delta);
            message.content = items.join("");
        }
        Event::TextItemDelta { id, delta } => {
            let index = *indices.entry(id).or_insert_with(|| {
                items.push(String::new());
                items.len() - 1
            });
            items[index].push_str(&delta);
            message.content = items.join("");
        }
        Event::TextItemCompleted { id, text } => {
            let index = *indices.entry(id).or_insert_with(|| {
                items.push(String::new());
                items.len() - 1
            });
            items[index] = text;
            message.content = items.join("");
        }
        Event::ThinkingDelta { delta } => message
            .reasoning
            .get_or_insert_with(String::new)
            .push_str(&delta),
        Event::ToolUse { id, name, input } => {
            if let Some(tool) = message.tool_calls.iter_mut().find(|t| t.id == id) {
                tool.arguments = input.to_string();
            } else {
                message.tool_calls.push(serde_json::from_value(json!({"id":id,"name":name,"source":"external_cli","arguments":input.to_string(),"status":"running"})).expect("tool record"));
            }
        }
        Event::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            if let Some(tool) = message.tool_calls.iter_mut().find(|t| t.id == tool_use_id) {
                tool.status = if is_error {
                    ToolCallStatus::Error
                } else {
                    ToolCallStatus::Success
                };
                tool.result_preview = Some(content);
            }
        }
        Event::Usage { usage } => message.usage = Some(usage),
        Event::Error { message } => *failure = Some(message),
        Event::Raw { line } => {
            items.push(line);
            message.content = items.join("");
        }
        _ => {}
    }
}
