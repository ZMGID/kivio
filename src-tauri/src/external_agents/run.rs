use std::collections::HashMap;
use std::time::Instant;

use chrono::Local;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::chat::agent::AgentRunEntry;
use crate::chat::commands::{
    emit_chat_stream_delta, emit_chat_stream_done, emit_chat_tool_record, push_assistant_message,
};
use crate::chat::memory::l1_prompt_block;
use crate::chat::model::ModelUsage;
use crate::chat::storage::save_conversation;
use crate::chat::types::{
    ChatMessageSegment, ChatMessageSegmentKind, ChatMessageSegmentPhase, ToolCallRecord,
    ToolCallStatus,
};
use crate::chat::Conversation;
use crate::external_agents::detection::detect_single_agent;
use crate::external_agents::prompt::{
    compose_external_prompt, compose_external_prompt_passthrough, cwd_hint, is_cli_slash_input,
};
use crate::external_agents::registry::get_agent_def;
use crate::external_agents::session::acp::{run_acp_session, AcpMcpServer};
use crate::external_agents::session::codex_app_server::run_codex_app_server_session;
use crate::external_agents::session::pi_rpc::run_pi_rpc_session;
use crate::external_agents::session::{persist_delivered_session, resolve_agent_resume_context};
use crate::external_agents::skill_stage::{skill_cwd_alias_segment, stage_active_skill};
use crate::external_agents::slash::{self};
use crate::external_agents::spawn::{
    drain_stderr, read_stdout_lines, resolve_binary, spawn_agent, tail_chars, write_prompt_stdin,
};
use crate::external_agents::stream::create_stream_handler;
use crate::external_agents::types::{
    RuntimeBuildOptions, RuntimeContext, StreamFormat, UnifiedAgentEvent,
};
use crate::external_agents::workspace::{extra_allowed_dirs_for_agent, resolve_effective_cwd};
use crate::skills::read_skill_detail;
use crate::state::AppState;

pub async fn run_external_cli_slash_command(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    slash_command: &str,
) -> Result<(), String> {
    if !is_cli_slash_input(slash_command) {
        return Err("外部 CLI slash 命令必须以 / 开头".to_string());
    }
    run_external_cli_reply(
        app,
        state,
        conversation,
        None,
        slash_command,
        None,
        AgentRunEntry::Send,
    )
    .await
}

pub async fn run_external_cli_reply(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    title_from_first_user: Option<&str>,
    latest_user_message: &str,
    active_skill_id: Option<&str>,
    entry: AgentRunEntry,
) -> Result<(), String> {
    let settings = state.settings_read().clone();
    let agent_id = conversation
        .agent_runtime
        .external_agent_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "未选择外部 Agent".to_string())?;

    let def = get_agent_def(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;

    let detected = detect_single_agent(def).await;
    if !detected.available {
        return Err(format!(
            "{} 未安装或不可用，请确认 CLI 在 PATH 中。",
            def.name
        ));
    }

    let resolved_bin = resolve_binary(def)
        .await
        .ok_or_else(|| format!("无法定位 {} 可执行文件", def.bin))?;

    let cwd = resolve_effective_cwd(app, &conversation.id, conversation.project_id.as_deref())?;
    let is_slash = is_cli_slash_input(latest_user_message);

    let skill_detail = if is_slash {
        None
    } else if let Some(skill_id) = active_skill_id.filter(|s| !s.is_empty()) {
        read_skill_detail(app, &settings.chat_tools.skill_scan_paths, skill_id).ok()
    } else {
        None
    };

    let memory_body = if is_slash || !settings.chat_memory.enabled {
        String::new()
    } else {
        l1_prompt_block(app).unwrap_or(None).unwrap_or_default()
    };

    let mut daemon_instructions = String::new();
    if !is_slash {
        if !settings.chat.system_prompt.trim().is_empty() {
            daemon_instructions.push_str(settings.chat.system_prompt.trim());
            daemon_instructions.push_str("\n\n");
        }
        if !memory_body.trim().is_empty() {
            daemon_instructions.push_str("## Memory\n\n");
            daemon_instructions.push_str(memory_body.trim());
            daemon_instructions.push('\n');
        }
    }
    daemon_instructions.push_str(&cwd_hint(cwd.to_string_lossy().as_ref()));

    let resume_ctx = resolve_agent_resume_context(
        app,
        &conversation.id,
        def.id,
        def.resumes_session_via_cli,
        &daemon_instructions,
        conversation.agent_runtime.external_model.as_deref(),
        is_slash,
    );

    let skill_dir = skill_detail.as_ref().and_then(|d| d.meta.path.clone());
    let skill_body = skill_detail.as_ref().map(|d| d.body.clone());
    let skill_folder = skill_dir.as_deref().map(skill_cwd_alias_segment);

    if !is_slash {
        if let (Some(dir), Some(folder)) = (skill_dir.as_deref(), skill_folder.as_deref()) {
            let _ = stage_active_skill(&cwd, folder, std::path::Path::new(dir));
        }
    }

    let composed = if is_slash {
        compose_external_prompt_passthrough(latest_user_message)
    } else {
        compose_external_prompt(
            conversation,
            &daemon_instructions,
            skill_body.as_deref(),
            skill_dir.as_deref(),
            skill_folder.as_deref(),
            resume_ctx.skip_instructions,
            resume_ctx.is_resuming,
            latest_user_message,
        )
    };

    let extra_dirs = extra_allowed_dirs_for_agent(def, &settings.chat_tools.skill_scan_paths);
    let runtime_ctx = RuntimeContext {
        extra_allowed_dirs: extra_dirs,
        resume_session_id: resume_ctx.resume_session_id.clone(),
        new_session_id: resume_ctx.new_session_id.clone(),
        include_partial_messages: true,
    };

    let build_options = RuntimeBuildOptions {
        model: conversation.agent_runtime.external_model.clone(),
        reasoning: conversation.agent_runtime.external_reasoning.clone(),
        sandbox: conversation.agent_runtime.external_sandbox.clone(),
    };

    if let Some(max_bytes) = def.max_prompt_arg_bytes {
        if composed.full_prompt.len() > max_bytes {
            return Err(format!(
                "Prompt 过长（{} 字节），超过 {} 的上限（{} 字节）。请缩短消息或改用 stdin 模式的 Agent。",
                composed.full_prompt.len(),
                def.name,
                max_bytes
            ));
        }
    }

    let prompt_for_args = if def.prompt_via_stdin {
        None
    } else {
        Some(composed.full_prompt.as_str())
    };
    let args = (def.build_args)(&runtime_ctx, &build_options, prompt_for_args);

    let extra_env: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let run_generation = state.next_chat_generation(&conversation.id);
    let run_id = format!("ext-run-{}-{}", run_generation, Uuid::new_v4());
    let assistant_message_id = format!("msg_{}", Uuid::new_v4());

    // Phase 2: codex app-server and ACP-family agents keep the process alive across turns via
    // the live-session registry. Other protocols still spawn one child per turn.
    let persistent = matches!(
        def.stream_format,
        StreamFormat::CodexAppServer | StreamFormat::AcpJsonRpc
    );
    let mut spawned_opt = if persistent {
        None
    } else {
        Some(spawn_agent(def, &resolved_bin, &args, &cwd, &extra_env).await?)
    };
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut raw_output = String::new();
    let mut tool_calls: Vec<ToolCallRecord> = Vec::new();
    let mut tool_map: HashMap<String, usize> = HashMap::new();
    let mut usage: Option<ModelUsage> = None;
    let mut stream_outcome = "completed".to_string();
    let mut segment_order = 0u32;
    let mut segments: Vec<ChatMessageSegment> = Vec::new();
    let mut segment_tracker = StreamSegmentTracker::default();
    let conversation_id = conversation.id.clone();
    let started_at = Instant::now();
    let slash_cache_key = slash::cache_key(&agent_id, &cwd.to_string_lossy());

    let mut emit_event = |event: UnifiedAgentEvent| {
        if let Some(commands) = slash::slash_commands_from_event(&event) {
            state.set_cached_external_slash_commands(slash_cache_key.clone(), commands);
        }
        apply_unified_event(
            app,
            &conversation_id,
            &run_id,
            &assistant_message_id,
            &mut content,
            &mut reasoning,
            &mut raw_output,
            &mut tool_calls,
            &mut tool_map,
            &mut usage,
            &mut segments,
            &mut segment_order,
            &mut segment_tracker,
            event,
        );
    };

    let cancel_check = || !state.is_chat_generation_active(&conversation_id, run_generation);

    // Drain stderr concurrently with the stdout read below: keeps a full stderr pipe from
    // blocking the child, and captures failure text a silent (non-JSON, empty-stdout) run would
    // otherwise lose. Persistent protocols manage their own process, so there's no child here.
    let stderr_task = spawned_opt
        .as_mut()
        .map(|spawned| drain_stderr(&mut spawned.child));

    let read_result = if persistent {
        let persistent_mcp: Vec<AcpMcpServer> = vec![];
        run_persistent_turn(
            app,
            state,
            &conversation_id,
            &agent_id,
            def.stream_format,
            &resolved_bin,
            &args,
            &cwd,
            conversation.agent_runtime.external_model.clone(),
            conversation.agent_runtime.external_reasoning.clone(),
            conversation.agent_runtime.external_sandbox.clone(),
            persistent_mcp,
            &composed.full_prompt,
            latest_user_message,
            &mut emit_event,
            &cancel_check,
        )
        .await
    } else {
        let spawned = spawned_opt
            .as_mut()
            .expect("non-persistent path spawns a child");
        match def.stream_format {
            StreamFormat::PiRpc => {
                let model = conversation.agent_runtime.external_model.as_deref();
                run_pi_rpc_session(
                    &mut spawned.child,
                    &composed.full_prompt,
                    model,
                    |event| emit_event(event),
                    cancel_check,
                )
                .await
            }
            StreamFormat::CodexAppServer => {
                let model = conversation.agent_runtime.external_model.as_deref();
                let reasoning = conversation.agent_runtime.external_reasoning.as_deref();
                run_codex_app_server_session(
                    &mut spawned.child,
                    &composed.full_prompt,
                    model,
                    reasoning,
                    &cwd,
                    |event| emit_event(event),
                    cancel_check,
                )
                .await
            }
            StreamFormat::AcpJsonRpc => {
                let model = conversation.agent_runtime.external_model.as_deref();
                let mcp_servers: Vec<AcpMcpServer> = vec![];
                run_acp_session(
                    &mut spawned.child,
                    &composed.full_prompt,
                    &cwd,
                    model,
                    &mcp_servers,
                    |event| emit_event(event),
                    cancel_check,
                )
                .await
            }
            _ => {
                if def.prompt_via_stdin {
                    write_prompt_stdin(&mut spawned.child, def, &composed.full_prompt).await?;
                }
                let mut handler = create_stream_handler(def.stream_format, def.json_event_parser);
                read_stdout_lines(
                    &mut spawned.child,
                    |line| {
                        handler.handle_line(line, &mut |event| emit_event(event));
                        Ok(())
                    },
                    cancel_check,
                )
                .await
            }
        }
    };

    // Non-persistent path waits on (and drops/kills) the per-turn child. Persistent sessions
    // keep their process alive in the registry, so there is nothing to wait on here.
    let exit_code: Option<i32> = match spawned_opt {
        Some(mut spawned) => {
            let status = spawned.child.wait().await.map_err(|e| e.to_string())?;
            status.code()
        }
        None => None,
    };
    let stderr_output = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    if let Err(err) = &read_result {
        stream_outcome = if err == "cancelled" {
            "cancelled"
        } else {
            "error"
        }
        .to_string();
        if err != "cancelled" && content.trim().is_empty() && raw_output.trim().is_empty() {
            raw_output = format!("{} 读取输出失败：{}", def.name, err);
        }
    } else if exit_code.map(|code| code != 0).unwrap_or(false) {
        if content.trim().is_empty() {
            stream_outcome = "error".to_string();
        }
    }

    // Fill empty content from the richest available fallback: captured raw stdout lines first,
    // then stderr (as an explicit failure), then the slash / no-output placeholders.
    if content.trim().is_empty() {
        if !raw_output.trim().is_empty() {
            content = raw_output.trim().to_string();
        } else if !stderr_output.trim().is_empty() {
            stream_outcome = "error".to_string();
            content = format!(
                "{} 执行失败：\n\n{}",
                def.name,
                truncate_for_preview(stderr_output.trim(), 4000)
            );
        } else if stream_outcome == "completed" {
            if is_slash {
                content = format!("{} 命令已执行", def.name);
            } else {
                stream_outcome = "error".to_string();
                content = format!(
                    "{} 未产生输出（exit={:?}，耗时 {}ms）",
                    def.name,
                    exit_code,
                    started_at.elapsed().as_millis()
                );
            }
        }
    }

    // A nonzero exit with stderr is a failure even if the CLI also produced some stdout — append
    // the stderr (unless it's already the content) so the error is visible, not swallowed.
    if exit_code.map(|code| code != 0).unwrap_or(false) && !stderr_output.trim().is_empty() {
        stream_outcome = "error".to_string();
        if !content.contains(stderr_output.trim()) {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&format!(
                "{} stderr：\n\n{}",
                def.name,
                truncate_for_preview(stderr_output.trim(), 4000)
            ));
        }
    }

    emit_chat_stream_done(
        app,
        &conversation_id,
        &run_id,
        &assistant_message_id,
        &stream_outcome,
        &content,
    );

    persist_delivered_session(
        app,
        &conversation_id,
        def.id,
        &resume_ctx,
        if composed.instructions_block.is_empty() {
            &daemon_instructions
        } else {
            &composed.instructions_block
        },
        is_slash,
    )?;

    push_assistant_message(
        app,
        state,
        &settings,
        conversation,
        assistant_message_id,
        content,
        if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        },
        vec![],
        tool_calls,
        vec![],
        segments,
        active_skill_id,
        title_from_first_user,
        Some(match entry {
            AgentRunEntry::Send => "send",
            AgentRunEntry::Regenerate => "regenerate",
        }),
        Some(&stream_outcome),
        usage,
        None,
        None,
    )
    .await?;

    save_conversation(app, conversation)?;
    Ok(())
}

#[derive(Default)]
struct StreamSegmentTracker {
    active_text_idx: Option<usize>,
    active_reasoning_idx: Option<usize>,
}

impl StreamSegmentTracker {
    fn reset_text(&mut self) {
        self.active_text_idx = None;
    }

    fn reset_reasoning(&mut self) {
        self.active_reasoning_idx = None;
    }

    fn append(
        &mut self,
        kind: ChatMessageSegmentKind,
        segments: &mut Vec<ChatMessageSegment>,
        segment_order: &mut u32,
        tool_calls_len: usize,
        delta: &str,
    ) -> ChatMessageSegment {
        let phase = text_phase_for_tool_count(tool_calls_len);
        let active = match kind {
            ChatMessageSegmentKind::Reasoning => &mut self.active_reasoning_idx,
            _ => &mut self.active_text_idx,
        };
        if let Some(idx) = *active {
            if let Some(segment) = segments.get_mut(idx) {
                if segment.kind == kind && segment.phase == phase {
                    let merged = format!("{}{}", segment.text.as_deref().unwrap_or(""), delta);
                    segment.text = Some(merged);
                    return segment.clone();
                }
            }
        }

        *segment_order += 1;
        let segment = ChatMessageSegment {
            id: format!("seg_{}", Uuid::new_v4()),
            kind,
            phase,
            order: *segment_order,
            step_number: None,
            round: if tool_calls_len == 0 { None } else { Some(1) },
            text: Some(delta.to_string()),
            tool_call_id: None,
        };
        *active = Some(segments.len());
        segments.push(segment.clone());
        segment
    }
}

/// Phase 2: run one turn against a persistent live session, reusing the conversation's existing
/// session, resuming a persisted one after a restart, or connecting fresh. The CLI process is kept
/// alive in the registry between turns, so a reused/resumed session sends only the latest user
/// message (the server holds prior context), while a fresh session gets the full composed prompt.
#[allow(clippy::too_many_arguments)]
async fn run_persistent_turn<E, C>(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation_id: &str,
    agent_id: &str,
    protocol: StreamFormat,
    resolved_bin: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    model: Option<String>,
    reasoning: Option<String>,
    sandbox: Option<String>,
    mcp_servers: Vec<AcpMcpServer>,
    first_prompt: &str,
    reuse_prompt: &str,
    emit: &mut E,
    cancel: &C,
) -> Result<(), String>
where
    E: FnMut(UnifiedAgentEvent),
    C: Fn() -> bool,
{
    use crate::external_agents::session::live::{LiveSession, SessionCommand};
    use crate::external_agents::session::{
        clear_live_handle, load_live_handle, save_live_handle, LiveSessionHandle,
    };
    use tokio::sync::{mpsc, oneshot};

    let cwd_str = cwd.to_string_lossy().to_string();
    let protocol_tag = persistent_protocol_tag(protocol);

    // 1. Reuse a live session already in the registry; 2. resume a persisted one; 3. fresh.
    let (control, prompt) =
        match state.external_live_session_control(conversation_id, agent_id, &cwd_str) {
            Some(control) => (control, reuse_prompt.to_string()),
            None => {
                let resume_native = load_live_handle(app, conversation_id)
                    .filter(|h| {
                        h.agent_id == agent_id && h.cwd == cwd_str && h.protocol == protocol_tag
                    })
                    .map(|h| h.native_id);
                let (control, native_id, resumed) = connect_persistent_session(
                    protocol,
                    resolved_bin,
                    args,
                    cwd,
                    model.as_deref(),
                    sandbox.as_deref(),
                    &mcp_servers,
                    resume_native,
                )
                .await?;
                let _ = save_live_handle(
                    app,
                    conversation_id,
                    &LiveSessionHandle {
                        agent_id: agent_id.to_string(),
                        protocol: protocol_tag.to_string(),
                        native_id,
                        cwd: cwd_str.clone(),
                    },
                );
                state.register_external_live_session(
                    conversation_id.to_string(),
                    LiveSession {
                        control: control.clone(),
                        agent_id: agent_id.to_string(),
                        cwd: cwd_str.clone(),
                        last_activity: std::time::Instant::now(),
                    },
                );
                // A resumed session already holds history → send only the latest message.
                let prompt = if resumed {
                    reuse_prompt.to_string()
                } else {
                    first_prompt.to_string()
                };
                (control, prompt)
            }
        };

    let (events_tx, mut events_rx) = mpsc::channel::<UnifiedAgentEvent>(64);
    let (done_tx, done_rx) = oneshot::channel::<Result<(), String>>();
    if control
        .send(SessionCommand::RunTurn {
            prompt,
            model,
            reasoning,
            events: events_tx,
            done: done_tx,
        })
        .await
        .is_err()
    {
        state.remove_external_live_session(conversation_id);
        clear_live_handle(app, conversation_id);
        return Err("外部 CLI 会话已结束，请重试".to_string());
    }

    let mut done_rx = done_rx;
    let mut events_open = true;
    let mut cancel_sent = false;
    loop {
        tokio::select! {
            biased;
            result = &mut done_rx => {
                while let Ok(event) = events_rx.try_recv() {
                    emit(event);
                }
                let outcome = result.unwrap_or_else(|_| Err("session actor dropped".to_string()));
                // A non-cancel failure means the process likely died — drop the live session and
                // its persisted handle so the next turn connects fresh.
                if let Err(ref e) = outcome {
                    state.remove_external_live_session(conversation_id);
                    if e != "cancelled" {
                        clear_live_handle(app, conversation_id);
                    }
                }
                return outcome;
            }
            maybe_event = events_rx.recv(), if events_open => {
                match maybe_event {
                    Some(event) => emit(event),
                    None => events_open = false,
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
        }
        if !cancel_sent && cancel() {
            cancel_sent = true;
            let _ = control.send(SessionCommand::Cancel).await;
        }
    }
}

fn persistent_protocol_tag(protocol: StreamFormat) -> &'static str {
    match protocol {
        StreamFormat::CodexAppServer => "codex_app_server",
        StreamFormat::AcpJsonRpc => "acp_json_rpc",
        _ => "unknown",
    }
}

/// Connect (or resume) a persistent protocol session, returning its control channel, native id,
/// and whether a resume actually succeeded. Falls back to a fresh session if resume fails.
async fn connect_persistent_session(
    protocol: StreamFormat,
    resolved_bin: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    model: Option<&str>,
    sandbox: Option<&str>,
    mcp_servers: &[AcpMcpServer],
    resume_native: Option<String>,
) -> Result<
    (
        tokio::sync::mpsc::Sender<crate::external_agents::session::live::SessionCommand>,
        String,
        bool,
    ),
    String,
> {
    use crate::external_agents::session::acp::{spawn_acp_session_actor, AcpSession};
    use crate::external_agents::session::codex_app_server::{
        spawn_codex_session_actor, CodexAppServerSession,
    };

    match protocol {
        StreamFormat::CodexAppServer => {
            if let Some(tid) = resume_native.as_deref() {
                if let Ok(session) = CodexAppServerSession::connect(
                    resolved_bin,
                    args,
                    cwd,
                    model,
                    sandbox,
                    Some(tid),
                )
                .await
                {
                    let id = session.thread_id().to_string();
                    return Ok((spawn_codex_session_actor(session), id, true));
                }
            }
            let session =
                CodexAppServerSession::connect(resolved_bin, args, cwd, model, sandbox, None)
                    .await?;
            let id = session.thread_id().to_string();
            Ok((spawn_codex_session_actor(session), id, false))
        }
        StreamFormat::AcpJsonRpc => {
            if let Some(sid) = resume_native.as_deref() {
                if let Ok(session) =
                    AcpSession::connect(resolved_bin, args, cwd, model, mcp_servers, Some(sid))
                        .await
                {
                    let id = session.session_id().to_string();
                    return Ok((spawn_acp_session_actor(session), id, true));
                }
            }
            let session =
                AcpSession::connect(resolved_bin, args, cwd, model, mcp_servers, None).await?;
            let id = session.session_id().to_string();
            Ok((spawn_acp_session_actor(session), id, false))
        }
        _ => Err("protocol does not support persistent sessions".to_string()),
    }
}

fn text_phase_for_tool_count(tool_calls_len: usize) -> ChatMessageSegmentPhase {
    if tool_calls_len == 0 {
        ChatMessageSegmentPhase::Plain
    } else {
        ChatMessageSegmentPhase::ToolLoop
    }
}

fn push_tool_segment(
    segments: &mut Vec<ChatMessageSegment>,
    segment_order: &mut u32,
    tool_call_id: &str,
) -> ChatMessageSegment {
    *segment_order += 1;
    let segment = ChatMessageSegment {
        id: format!("seg_{}", Uuid::new_v4()),
        kind: ChatMessageSegmentKind::Tool,
        phase: ChatMessageSegmentPhase::ToolLoop,
        order: *segment_order,
        step_number: None,
        round: Some(1),
        text: None,
        tool_call_id: Some(tool_call_id.to_string()),
    };
    segments.push(segment.clone());
    segment
}

fn apply_unified_event(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    content: &mut String,
    reasoning: &mut String,
    raw_output: &mut String,
    tool_calls: &mut Vec<ToolCallRecord>,
    tool_map: &mut HashMap<String, usize>,
    usage: &mut Option<ModelUsage>,
    segments: &mut Vec<ChatMessageSegment>,
    segment_order: &mut u32,
    segment_tracker: &mut StreamSegmentTracker,
    event: UnifiedAgentEvent,
) {
    let now = Local::now().timestamp();
    match event {
        UnifiedAgentEvent::TextDelta { delta } => {
            content.push_str(&delta);
            let segment = segment_tracker.append(
                ChatMessageSegmentKind::Text,
                segments,
                segment_order,
                tool_calls.len(),
                &delta,
            );
            emit_chat_stream_delta(
                app,
                conversation_id,
                run_id,
                message_id,
                &delta,
                None,
                Some(&segment),
            );
        }
        UnifiedAgentEvent::ThinkingDelta { delta } => {
            reasoning.push_str(&delta);
            let segment = segment_tracker.append(
                ChatMessageSegmentKind::Reasoning,
                segments,
                segment_order,
                tool_calls.len(),
                &delta,
            );
            emit_chat_stream_delta(
                app,
                conversation_id,
                run_id,
                message_id,
                "",
                Some(&delta),
                Some(&segment),
            );
        }
        UnifiedAgentEvent::ToolUse { id, name, input } => {
            segment_tracker.reset_text();
            segment_tracker.reset_reasoning();
            let segment = push_tool_segment(segments, segment_order, &id);
            emit_chat_stream_delta(
                app,
                conversation_id,
                run_id,
                message_id,
                "",
                None,
                Some(&segment),
            );
            let record = ToolCallRecord {
                id: id.clone(),
                name: name.clone(),
                source: "external_cli".to_string(),
                server_id: None,
                arguments: input.to_string(),
                status: ToolCallStatus::Running,
                result_preview: None,
                error: None,
                duration_ms: None,
                started_at: Some(now),
                completed_at: None,
                round: 1,
                sensitive: false,
                artifacts: vec![],
                trace_id: None,
                span_id: None,
                structured_content: Some(input),
            };
            tool_map.insert(id.clone(), tool_calls.len());
            tool_calls.push(record.clone());
            emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);
        }
        UnifiedAgentEvent::ToolResult {
            tool_use_id,
            content: result_content,
            is_error,
        } => {
            if let Some(idx) = tool_map.get(&tool_use_id).copied() {
                if let Some(record) = tool_calls.get_mut(idx) {
                    record.status = if is_error {
                        ToolCallStatus::Error
                    } else {
                        ToolCallStatus::Success
                    };
                    record.result_preview = Some(truncate_for_preview(&result_content, 800));
                    record.completed_at = Some(now);
                    emit_chat_tool_record(app, conversation_id, run_id, message_id, record);
                }
            }
        }
        UnifiedAgentEvent::Usage { usage: u } => {
            *usage = Some(u);
        }
        UnifiedAgentEvent::Error { message, .. } => {
            eprintln!("[external-agent] stream error: {message}");
        }
        UnifiedAgentEvent::Raw { line } => {
            // Unparsed stdout line — accumulate (capped) as a fallback surfaced only if the run
            // produced no structured content.
            if !raw_output.is_empty() {
                raw_output.push('\n');
            }
            raw_output.push_str(&line);
            if raw_output.chars().count() > 8192 {
                *raw_output = tail_chars(raw_output, 8192);
            }
        }
        _ => {}
    }
}

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    let mut out: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_segment_tracker_reuses_text_segment_for_deltas() {
        let mut segments = Vec::new();
        let mut order = 0u32;
        let mut tracker = StreamSegmentTracker::default();

        let first = tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            0,
            "你",
        );
        let second = tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            0,
            "好",
        );

        assert_eq!(segments.len(), 1);
        assert_eq!(first.id, second.id);
        assert_eq!(segments[0].text.as_deref(), Some("你好"));
        assert_eq!(segments[0].phase, ChatMessageSegmentPhase::Plain);
    }

    #[test]
    fn push_tool_segment_increments_order_and_sets_tool_kind() {
        let mut segments = Vec::new();
        let mut order = 2u32;
        let first = push_tool_segment(&mut segments, &mut order, "tool-1");
        let second = push_tool_segment(&mut segments, &mut order, "tool-2");

        assert_eq!(segments.len(), 2);
        assert_eq!(first.kind, ChatMessageSegmentKind::Tool);
        assert_eq!(first.order, 3);
        assert_eq!(first.tool_call_id.as_deref(), Some("tool-1"));
        assert_eq!(second.order, 4);
        assert_eq!(second.phase, ChatMessageSegmentPhase::ToolLoop);
    }

    #[test]
    fn stream_segment_tracker_starts_new_text_segment_after_tool_use() {
        let mut segments = Vec::new();
        let mut order = 0u32;
        let mut tracker = StreamSegmentTracker::default();

        tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            0,
            "before",
        );
        tracker.reset_text();
        let after = tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            1,
            "after",
        );

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text.as_deref(), Some("before"));
        assert_eq!(segments[1].text.as_deref(), Some("after"));
        assert_eq!(after.phase, ChatMessageSegmentPhase::ToolLoop);
    }
}
