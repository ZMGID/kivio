//! Reuse Zeron's native CLI drivers; Kivio retains configuration and durable data ownership.
use super::*;
use zeron_harness::{
    AcpHarness, ClaudeHarness, CodexHarness, CursorHarness, Harness, OpencodeHarness, PiHarness,
};
use zeron_proto::HarnessId;

pub(super) const AGENTS: &[(&str, HarnessId)] = &[
    ("claude", HarnessId::ClaudeCode),
    ("codex", HarnessId::Codex),
    ("cursor", HarnessId::Cursor),
    ("pi", HarnessId::Pi),
    ("opencode", HarnessId::Opencode),
    ("grok", HarnessId::Grok),
    ("hermes", HarnessId::Hermes),
    ("antigravity", HarnessId::Antigravity),
    ("devin", HarnessId::Devin),
];
pub(super) fn driver(root: &std::path::Path, agent: &str) -> Result<Arc<dyn Harness>, String> {
    let settings = kivio_model_runtime::native_preferences::load(root)?;
    let path = settings
        .chat
        .external_cli_agents
        .get(agent)
        .map(|c| c.path.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    driver_at(root, agent, path)
}
fn driver_at(
    root: &std::path::Path,
    agent: &str,
    path: Option<PathBuf>,
) -> Result<Arc<dyn Harness>, String> {
    macro_rules! configured {
        ($driver:expr) => {{
            let driver = $driver;
            Arc::new(if let Some(path) = path {
                driver.with_executable(path)
            } else {
                driver
            }) as Arc<dyn Harness>
        }};
    }
    Ok(match agent {
        "claude" => configured!(ClaudeHarness::new()),
        "codex" => configured!(CodexHarness::new()),
        "cursor" => configured!(CursorHarness::new()),
        "pi" => configured!(
            PiHarness::new().with_session_store(root.join("external-agent-sessions/pi"))
        ),
        "opencode" => configured!(OpencodeHarness::new()),
        "grok" => configured!(AcpHarness::grok()),
        "hermes" => configured!(AcpHarness::hermes()),
        "antigravity" => configured!(AcpHarness::antigravity()),
        "devin" => configured!(AcpHarness::devin()),
        _ => return Err(format!("未知 CLI：{agent}")),
    })
}
impl KivioService {
    pub(super) async fn harness_catalog(
        &self,
        method: &str,
        params: Value,
    ) -> Result<RpcReply, RpcError> {
        let root = self.directory.clone();
        if method == methods::LIST_HARNESSES {
            let rows = tokio::task::spawn_blocking(move || {
                let settings = kivio_model_runtime::native_preferences::load(&root)?;
                AGENTS.iter().map(|(agent, id)| {
                    let driver = driver(&root, agent)?;
                    Ok(json!({"id":id,"name":driver.display_name(),"installed":driver.installed(),
                        "enabled":!settings.chat.external_cli_agents.get(*agent).is_some_and(|c| c.disabled),
                        "supportsSteering":driver.supports_steering(),"steeringMode":driver.steering_mode(),
                        "reasoningLevels":driver.reasoning_levels()}))
                }).collect::<Result<Vec<_>, String>>()
            }).await.map_err(failed)?.map_err(failed)?;
            return RpcReply::value(&rows);
        }
        let id: HarnessId = serde_json::from_value(params["harness"].clone()).map_err(failed)?;
        let agent = AGENTS
            .iter()
            .find(|(_, h)| *h == id)
            .map(|(a, _)| *a)
            .ok_or_else(|| failed("未知 CLI"))?;
        let driver = driver(&root, agent).map_err(failed)?;
        let cwd = kivio_model_runtime::external_agents::native::workspace(
            &root,
            params["chatId"].as_str().unwrap_or("conv_draft"),
            params["spaceId"].as_str(),
        )
        .map_err(failed)?;
        match method {
            methods::LIST_MODELS => {
                let mut models = driver
                    .model_catalog(params["force"].as_bool().unwrap_or(false))
                    .await
                    .map_err(failed)?
                    .models;
                let settings =
                    kivio_model_runtime::native_preferences::load(&root).map_err(failed)?;
                if let Some(config) = settings.chat.external_cli_agents.get(agent) {
                    for custom in &config.custom_models {
                        if !custom.id.trim().is_empty() && !models.iter().any(|m| m.id == custom.id)
                        {
                            models.push(zeron_proto::Model {
                                id: custom.id.clone(),
                                label: if custom.label.trim().is_empty() {
                                    custom.id.clone()
                                } else {
                                    custom.label.clone()
                                },
                                description: None,
                                reasoning_levels: driver.reasoning_levels().to_vec(),
                                options: Vec::new(),
                            });
                        }
                    }
                }
                RpcReply::value(&models)
            }
            methods::LIST_COMMANDS => {
                RpcReply::value(&driver.commands_for(&cwd).await.map_err(failed)?)
            }
            methods::LIST_SKILLS => RpcReply::value(
                &driver
                    .skills(&cwd)
                    .await
                    .map_err(failed)?
                    .unwrap_or_default(),
            ),
            _ => Err(RpcError::UnknownMethod(method.into())),
        }
    }
}

pub(super) async fn run(
    service: KivioService,
    conversation: Conversation,
    launch: kivio_model_runtime::external_agents::native::Launch,
    events: tokio::sync::mpsc::Sender<
        kivio_model_runtime::external_agents::types::UnifiedAgentEvent,
    >,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    use kivio_model_runtime::external_agents::{
        attachments, native, types::UnifiedAgentEvent as E,
    };
    use zeron_proto::{AgentEvent as A, DoneStatus};
    let root = &service.directory;
    let agent = conversation
        .agent_runtime
        .external_agent_id
        .as_deref()
        .ok_or("缺少 CLI 标识")?;
    let driver = driver_at(root, agent, Some(launch.binary.clone()))?;
    let stored = native::binding(root, &conversation.id)?;
    if stored.is_none()
        && conversation.messages.iter().any(|m| {
            m.role == "assistant"
                && !matches!(m.stream_outcome.as_deref(), Some("error" | "cancelled"))
        })
    {
        return Err("缺少 CLI 会话绑定，无法安全续聊".into());
    }
    let user = conversation.messages.last().ok_or("缺少用户消息")?;
    let dir = root
        .join("conversations")
        .join(format!("{}_attachments", conversation.id));
    let paths = user
        .attachments
        .iter()
        .map(|a| {
            kivio_model_runtime::attachment_files::resolve_attachment_file_path_in_dir(
                Some(&dir),
                &a.path,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut prompt = user.content.clone();
    if !launch.instructions.trim().is_empty() {
        prompt = format!(
            "# Instructions\n\n{}\n\n# User request\n\n{prompt}",
            launch.instructions
        );
    }
    prompt.push_str(&attachments::file_attachments_note(&paths));
    let mut request: zeron_proto::RunRequest = serde_json::from_value(json!({
        "prompt":prompt,"harness":driver.id(),"model":conversation.agent_runtime.external_model,
        "reasoning":conversation.agent_runtime.external_reasoning,"cwd":launch.cwd,
        "modelOptions":conversation.agent_runtime.external_model_options,
        "sandbox":conversation.agent_runtime.external_sandbox.as_deref().unwrap_or("workspace-write"),
        "autoApprove":false,"resume":stored.as_ref().map(|b| &b.session.session_id),
        "attachments":paths.iter().filter(|p| attachments::image_mime_for_path(p).is_some()).map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>()
    })).map_err(|e| e.to_string())?;
    request.env = launch.env.into_iter().map(|e| (e.key, e.value)).collect();
    let interrupt = zeron_harness::CancellationToken::new();
    let _teardown = interrupt.clone().drop_guard();
    let (_steer, steering) = tokio::sync::mpsc::channel(8);
    let ask_service = service.clone();
    let chat_id = conversation.id.clone();
    let controls = zeron_harness::RunControls {
        realtime: None,
        execution_lease: None,
        steering,
        interrupt: interrupt.clone(),
        turn: Default::default(),
        request_input: Box::new(move |q| ask_service.ask(&chat_id, q)),
    };
    let mut stream = tokio::select! {
        _ = &mut cancelled => { interrupt.cancel(); return Err("cancelled".into()); }
        result = driver.run(request, controls) => result.map_err(|e| e.to_string())?,
    };
    let mut outcome = None;
    let mut stopping = false;
    let mut stop_deadline = None;
    loop {
        if stopping && stop_deadline.is_none() {
            stop_deadline = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(10));
        }
        let next = tokio::select! {
            _ = &mut cancelled, if !stopping => { stopping = true; interrupt.cancel(); continue; }
            _ = async {
                match stop_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => return Err("CLI 停止超时，已取消运行并释放连接".into()),
            event = stream.next() => event,
        };
        let Some(next) = next else {
            break;
        };
        let event = match next {
            Ok(event) => event,
            Err(e) => {
                outcome = Some(Err(e.to_string()));
                interrupt.cancel();
                stopping = true;
                continue;
            }
        };
        let projected = match event {
            A::SessionStarted {
                session_id, cwd, ..
            } => {
                if let Some(saved) = &stored {
                    if saved.session.session_id != session_id || saved.cwd != PathBuf::from(&cwd) {
                        outcome = Some(Err("CLI 未恢复原会话或原工作目录，已终止本次运行".into()));
                        interrupt.cancel();
                        stopping = true;
                        continue;
                    }
                }
                native::save_binding(
                    root,
                    &native::Binding {
                        cwd: PathBuf::from(cwd),
                        session:
                            kivio_model_runtime::external_agents::types::ExternalAgentSession {
                                conversation_id: conversation.id.clone(),
                                agent_id: agent.into(),
                                session_id,
                                model: conversation.agent_runtime.external_model.clone(),
                                stable_prompt_hash: None,
                                recorded_prompt_hash: None,
                            },
                    },
                )?;
                None
            }
            A::TextDelta { text } => Some(E::TextDelta { delta: text }),
            A::ReasoningDelta { text } => Some(E::ThinkingDelta { delta: text }),
            A::ToolCall { id, call } => {
                let input = serde_json::to_value(call).map_err(|e| e.to_string())?;
                Some(E::ToolUse {
                    id,
                    name: input["kind"].as_str().unwrap_or("Tool").into(),
                    input,
                })
            }
            A::ToolResult {
                id,
                is_error,
                output,
                ..
            } => Some(E::ToolResult {
                tool_use_id: id,
                content: output.unwrap_or_default(),
                is_error,
            }),
            A::Error { message } => Some(E::Error { message }),
            A::AvailableCommands { .. } | A::AssistantMessageCompleted { .. } => None,
            A::Done { status, error, .. } => {
                if outcome.is_none() {
                    outcome = Some(match status {
                        DoneStatus::Completed => Ok(()),
                        DoneStatus::Interrupted => Err("cancelled".into()),
                        DoneStatus::Errored => Err(error.unwrap_or_else(|| "CLI 运行失败".into())),
                    });
                }
                interrupt.cancel();
                stopping = true;
                None
            }
            A::Usage {
                input_tokens,
                output_tokens,
            } => Some(E::Usage {
                usage: kivio_model_runtime::external_agents::stream::usage_from_parts(
                    kivio_model_runtime::external_agents::stream::CliUsageParts {
                        input: input_tokens,
                        output: output_tokens,
                        ..Default::default()
                    },
                ),
            }),
            _ => None,
        };
        if let Some(event) = projected {
            if events.send(event).await.is_err() {
                interrupt.cancel();
                stopping = true;
            }
        }
    }
    outcome.unwrap_or_else(|| {
        Err(if stopping {
            "cancelled"
        } else {
            "CLI 连接中断，未收到完成事件"
        }
        .into())
    })
}

pub(super) fn apply_config(c: &mut Conversation, value: &Value) -> Result<(), RpcError> {
    let config: zeron_proto::ChatConfig = serde_json::from_value(value.clone()).map_err(failed)?;
    let agent = AGENTS
        .iter()
        .find(|(_, h)| *h == config.harness)
        .map(|(a, _)| *a)
        .ok_or_else(|| failed("未知 CLI"))?;
    if c.agent_runtime.external_agent_id.as_deref() != Some(agent) {
        return Err(failed("已有对话必须继续使用原 CLI；请选择新对话切换"));
    }
    c.agent_runtime.external_model = config.model;
    c.agent_runtime.external_service_tier = config
        .model_options
        .get("serviceTier")
        .and_then(Value::as_str)
        .map(str::to_owned);
    c.agent_runtime.external_model_options = config.model_options;
    c.agent_runtime.external_reasoning = config
        .reasoning
        .and_then(|r| serde_json::to_value(r).ok())
        .and_then(|v| v.as_str().map(str::to_owned));
    c.agent_runtime.external_sandbox = serde_json::to_value(config.sandbox)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned));
    Ok(())
}
