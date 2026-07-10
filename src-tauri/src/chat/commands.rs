use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_shell::ShellExt;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::chat::agent::{prepare as agent_prepare, stop as agent_stop};
use crate::chat::attachments::{
    compose_user_content_for_api, is_attachable_file_name, read_attachment_as_data_url,
    resolve_attachment_file_path, save_message_attachments, save_pasted_attachment,
    save_pasted_image, stored_image_paths_for_attachments, title_source_for_user_message,
    PastedAttachmentSave, PastedImageSave,
};
use crate::chat::model::{
    model_messages_from_openai_messages, MessagePart, ModelMessage, ModelRole,
};
#[cfg(test)]
use crate::chat::model::openai_messages_from_model_messages;
use crate::chat::model_metadata::{
    chat_max_output_tokens_for_model, model_can_generate_images_directly,
    reasoning_efforts_for_model,
};
use crate::mcp::types::ChatToolArtifact;
use crate::mcp::{self, ChatToolDefinition};
use crate::settings::{ModelProvider, SessionModel, Settings};
use crate::skills;
use crate::state::AppState;

use super::model_call::{
    call_chat_completion_message, chat_missing_model_error, format_chat_missing_api_key_error,
    session_model_for_conversation,
};
#[cfg(test)]
use super::vision::AuxiliaryVisionResult;
use super::vision::{
    analyze_chat_images_with_auxiliary_model, auxiliary_vision_model_for_images,
    auxiliary_vision_tool_record, finish_auxiliary_vision_tool_record, image_content_part,
    user_content_with_auxiliary_vision_result,
};
use super::storage::{
    assistant_snapshot, conversation_attachments_dir, create_project,
    delete_conversation as delete_conv, find_project_by_id, find_project_by_name, find_set_by_id,
    get_projects, load_conversation, save_conversation, update_project,
};
use super::{
    AgentPlanState, AgentTodoState, ChatMessage, ChatMessageSegment, ChatMessageSegmentKind,
    ChatMessageSegmentPhase, CompactionBoundaryRecord, Conversation, ConversationContextState,
    ForkOrigin, ToolCallRecord,
    ToolCallStatus,
};

pub(crate) mod catalog;

pub(crate) use catalog::create_assistant_via_builder;
use catalog::{
    chat_memory_prompt_for_request, create_chat_conversation_internal, is_builder_conversation,
    project_prompt_context_for, reconcile_conversation_orphan_tool_segments,
    strip_transcripts_for_frontend,
};
#[cfg(test)]
use catalog::assistant_from_builder_args;

pub(crate) mod context;

use context::{
    build_chat_api_messages, compress_conversation_context, compute_context_state,
    context_likely_over_limit, emit_chat_compaction_state, emit_chat_context_state,
    mark_summary_stale_if_needed, resolve_usage_anchor, rollback_user_message_after_failed_send,
    should_auto_compress_context, try_auto_compress_context_after_update,
};
#[cfg(test)]
use context::{
    count_tokens_in_value, estimate_image_tokens_for_dimensions,
    group_answer_excluded_from_context,
};
#[cfg(test)]
use super::ConversationContextSummary;

const DIRECT_IMAGE_GENERATION_PENDING: &str = "[[KIVIO_DIRECT_IMAGE_GENERATION_PENDING]]";
const CHAT_REPLY_BUSY_ERROR: &str = "该对话正在生成中，请稍后再试";
/// 多模型一问多答的并排上限（决策 D4）。超过此数不允许发送。
const MAX_REPLY_MODELS: usize = 4;

/// 由会话级 `reply_models` 解析出本次发送要 fan-out 的「臂」列表。
/// 返回去重后（按 provider_id+model）、保序的 `(provider_id, model)`。
/// - 0 或 1 个有效臂 → 返回长度 ≤1（调用方走单模型现状路径，行为不变）。
/// - ≥2 个 → 多模型 fan-out。
/// 校验：上限 `MAX_REPLY_MODELS`（超出 `Err`）；provider 必须存在（不存在的臂跳过）；
/// 空 model 跳过。
fn resolve_reply_arms(
    settings: &Settings,
    reply_models: &[crate::chat::ModelRef],
) -> Result<Vec<(String, String)>, String> {
    if reply_models.len() > MAX_REPLY_MODELS {
        return Err(format!(
            "多模型并行回答最多同时选择 {MAX_REPLY_MODELS} 个模型（当前 {}）。",
            reply_models.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut arms = Vec::new();
    for model_ref in reply_models {
        let provider_id = model_ref.provider_id.trim();
        let model = model_ref.model.trim();
        if provider_id.is_empty() || model.is_empty() {
            continue;
        }
        if settings.get_provider(provider_id).is_none() {
            continue;
        }
        let key = format!("{provider_id}\u{0}{model}");
        if seen.insert(key) {
            arms.push((provider_id.to_string(), model.to_string()));
        }
    }
    Ok(arms)
}

/// 命令入口的哨兵预留守卫：原子地「busy 检查 + 占一个哨兵槽位」，关闭 busy 判定与真实
/// per-run 槽位注册之间的 TOCTOU 窗口（防同会话并发发送同时通过 busy 检查）。哨兵槽位只占
/// `chat_active_replies`、不参与 generation/取消，命令任意退出路径 drop 时释放。
/// 真实 per-run 槽位（`ChatReplyGuard`）在哨兵存活期间额外注册，二者按不同 run_id 共存。
struct ChatSendReservation<'a> {
    state: &'a AppState,
    conversation_id: String,
    run_id: String,
}

impl<'a> ChatSendReservation<'a> {
    /// 尝试预留某会话的发送哨兵。返回 None 表示该会话已有 run 在跑（busy）。
    fn try_acquire(state: &'a AppState, conversation_id: &str) -> Option<Self> {
        let run_id = format!("chat-send-reservation-{}", Uuid::new_v4());
        if !state.try_reserve_chat_send(conversation_id, &run_id) {
            return None;
        }
        Some(Self {
            state,
            conversation_id: conversation_id.to_string(),
            run_id,
        })
    }
}

impl Drop for ChatSendReservation<'_> {
    fn drop(&mut self) {
        self.state.end_chat_reply(&self.conversation_id, &self.run_id);
    }
}

/// RAII 守卫：占住某条 run 的回复槽位与活跃 generation，函数任意退出路径都释放。
/// 同一会话允许多条 run 并存（多模型一问多答），每条 run 各持一个守卫。
struct ChatReplyGuard<'a> {
    state: &'a AppState,
    conversation_id: String,
    run_id: String,
    generation: u64,
}

impl<'a> ChatReplyGuard<'a> {
    /// 注册一条 run 的回复槽位。返回 None 表示同一 (conversation_id, run_id) 已在进行中。
    /// `generation` 一并登记，drop 时随槽位一起退役（不影响同会话其它在跑 run）。
    fn try_new(
        state: &'a AppState,
        conversation_id: &str,
        run_id: &str,
        generation: u64,
    ) -> Option<Self> {
        if !state.try_begin_chat_reply(conversation_id, run_id) {
            return None;
        }
        Some(Self {
            state,
            conversation_id: conversation_id.to_string(),
            run_id: run_id.to_string(),
            generation,
        })
    }
}

impl Drop for ChatReplyGuard<'_> {
    fn drop(&mut self) {
        self.state.end_chat_reply(&self.conversation_id, &self.run_id);
        self.state
            .end_chat_generation(&self.conversation_id, self.generation);
    }
}

/// 多模型一问多答（任务 06-30）单条「臂」的覆盖配置。`complete_assistant_reply`
/// 收到 `Some(arm)` 时：用该臂自己的 provider/model（而非会话级），把 `group_id`/
/// provider/model 写进 assistant 消息，**自动批准工具**（避免 N 个并发 run 各弹一次审批），
/// 并且 **不直接落盘**——产出的 assistant `ChatMessage` 由协调者（`chat_send_message`）回收后
/// 统一 upsert + 一次性 save，避开 N 条并发 run 同写一个 `conversations/{id}.json` 的竞态。
/// 单模型路径传 `None`，行为与改造前完全一致。
struct ReplyArm {
    group_id: String,
    provider_id: String,
    model: String,
}

/// 多模型臂运行后回收的结果。协调者据此把 assistant 消息合并进真正的会话并落盘。
/// 单模型路径（`arm = None`）`message` 为 None（已在函数内自行落盘）。
struct ArmReplyOutcome {
    message: Option<ChatMessage>,
}

/// 取走外部入口排队给 Chat 前端发送的消息。
#[tauri::command]
pub(crate) fn chat_take_external_sends(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let requests = {
        let mut pending = state
            .pending_chat_external_sends
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *pending)
    };

    Ok(serde_json::json!({
        "success": true,
        "requests": requests,
    }))
}

#[tauri::command]
pub(crate) fn chat_set_agent_plan_mode(
    app: AppHandle,
    conversation_id: String,
    mode: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let mode = crate::chat::plan::mode_from_str(&mode)?;
    conversation.agent_plan_state =
        crate::chat::plan::with_mode(&conversation.agent_plan_state, mode);
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_plan_state(&app, &conversation.id, &conversation.agent_plan_state);

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
        "planState": conversation.agent_plan_state,
    }))
}

#[tauri::command]
pub(crate) fn chat_execute_agent_plan(
    app: AppHandle,
    conversation_id: String,
    message_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    approve_agent_plan_for_execution(&mut conversation, message_id.as_deref())?;
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_plan_state(&app, &conversation.id, &conversation.agent_plan_state);

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
        "planState": conversation.agent_plan_state,
    }))
}

fn approve_agent_plan_for_execution(
    conversation: &mut Conversation,
    message_id: Option<&str>,
) -> Result<(), String> {
    let selected_plan = if let Some(message_id) = message_id
        .map(str::trim)
        .filter(|id| !id.is_empty()) {
        Some({
            let message = conversation
                .messages
                .iter_mut()
                .find(|message| message.id == message_id && message.role == "assistant")
                .ok_or_else(|| "计划消息不存在".to_string())?;
            let plan_state = message
                .agent_plan
                .as_ref()
                .ok_or_else(|| "该消息不是可执行计划".to_string())?;
            if crate::chat::plan::executable_plan_text(plan_state).is_none() {
                return Err("该消息不是可执行计划".to_string());
            }
            let approved = crate::chat::plan::approve(plan_state);
            message.agent_plan = Some(approved.clone());
            approved
        })
    } else {
        None
    };
    conversation.agent_plan_state = selected_plan.unwrap_or_else(|| {
        crate::chat::plan::approve(&conversation.agent_plan_state)
    });
    Ok(())
}

/// 由「每对话思考等级」解析出实际下发给模型的 `(thinking_enabled, thinking_level)`。
/// chat 不再跟随全局思考开关（全局开关只服务 lens / 快速翻译），未显式选档时落到默认档「high」。
/// - `"off"` → 强制关思考，不带等级。
/// - `"low"|"medium"|"high"|"xhigh"|"max"` → 开思考并带等级（适配器按家族映射为
///   reasoning_effort / output_config.effort）。等级是否被某模型接受由前端按模型 id 门控；
///   `xhigh` 仅 OpenAI GPT-5/Anthropic，`max` 仅 Anthropic。
/// - `None` 或其它未知值 → 默认档「high」（与前端 `ThinkingLevelSelector` 的 DEFAULT_LEVEL 一致）。
pub(crate) fn resolve_thinking(
    conv_level: Option<&str>,
    _global_enabled: bool,
) -> (bool, Option<String>) {
    match conv_level {
        Some("off") => (false, None),
        Some(level @ ("low" | "medium" | "high" | "xhigh" | "max")) => {
            (true, Some(level.to_string()))
        }
        _ => (true, Some("high".to_string())),
    }
}

/// 返回某模型支持的思考等级列表（数据来自模型库 `reasoningEfforts`）。供前端等级选择器决定显示哪些档。
#[tauri::command]
pub(crate) fn chat_reasoning_efforts_for_model(
    model: String,
    api_format: Option<String>,
) -> Vec<String> {
    reasoning_efforts_for_model(&model, api_format.as_deref().unwrap_or(""))
}

/// 发送消息
#[tauri::command]
pub(crate) async fn chat_send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    attachments: Vec<String>,
    active_skill_id: Option<String>,
) -> Result<serde_json::Value, String> {
    // Busy 拒绝：该会话仍有任意一条 run 在跑（含多模型并发组）时不允许再发新消息。
    // 用原子的哨兵预留替代「先 check 后 register」，关闭并发发送同时通过 busy 检查的 TOCTOU 窗口。
    // 哨兵在本命令返回前一直存活；实际的 per-run 槽位 / generation 在 `complete_assistant_reply`
    // 内 run_id 生成处额外注册，与哨兵按不同 run_id 共存。
    let Some(_send_reservation) = ChatSendReservation::try_acquire(state.inner(), &conversation_id)
    else {
        return Ok(serde_json::json!({
            "success": false,
            "error": CHAT_REPLY_BUSY_ERROR,
        }));
    };

    let mut conversation = load_conversation(&app, &conversation_id)?;

    // Backend slash-trigger preprocessing (承重路径): plain text `/commit msg`
    // pins the skill and rewrites the body even without the front-end popover
    // (also covers paste / external API / mobile entry points).
    // External CLI conversations pass slash commands straight through to the agent.
    let (content, active_skill_id) = if conversation.agent_runtime.is_external() {
        (content, active_skill_id)
    } else {
        let settings = state.settings_read().clone();
        let registry =
            skills::build_registry(&app, &settings.chat_tools.skill_scan_paths).unwrap_or_default();
        match try_apply_skill_slash_trigger(
            &registry,
            &settings.chat_tools,
            conversation.assistant_snapshot.as_ref(),
            &content,
            &settings.email_accounts,
            crate::settings::obsidian_connector_configured(&settings.obsidian_vault_path),
        ) {
            Some((skill_id, rewritten)) => (rewritten, Some(skill_id)),
            None => (content, active_skill_id),
        }
    };

    let message_attachments = save_message_attachments(&app, &conversation_id, attachments)?;
    let attachments_dir = if message_attachments.is_empty() {
        None
    } else {
        Some(conversation_attachments_dir(&app, &conversation_id)?)
    };
    let api_content =
        compose_user_content_for_api(&content, &message_attachments, attachments_dir.as_deref());
    let title_source = title_source_for_user_message(&content, &message_attachments);
    let last_user_image_paths =
        stored_image_paths_for_attachments(&app, &conversation_id, &message_attachments)?;

    // 多模型一问多答（任务 06-30）：从会话级 reply_models 解析本次要并行的「臂」。
    // 0/1 个有效臂 → 单模型现状路径（行为完全不变，防回归 AC5）。≥2 → fan-out。
    // 仅普通（Act）模式生效（R11）：plan / orchestrate 模式下不 fan-out。
    let reply_arms = {
        let settings = state.settings_read();
        resolve_reply_arms(&settings, &conversation.reply_models)?
    };
    let plan_or_orchestrate = crate::chat::plan::is_plan_mode(&conversation.agent_plan_state)
        || crate::chat::plan::is_orchestrate_mode(&conversation.agent_plan_state);
    let fan_out = reply_arms.len() >= 2 && !plan_or_orchestrate;
    // fan-out 时所有臂共享一个 group_id；用户消息也打上它，便于前端把这一问的 N 答聚成一组。
    let group_id = if fan_out {
        Some(format!("grp_{}", Uuid::new_v4()))
    } else {
        None
    };

    // 创建用户消息
    let user_message = ChatMessage {
        id: format!("msg_{}", Uuid::new_v4()),
        role: "user".to_string(),
        content: content.clone(),
        attachments: message_attachments,
        reasoning: None,
        artifacts: Vec::new(),
        tool_calls: Vec::new(),
        segments: Vec::new(),
        agent_plan: None,
        api_messages: Vec::new(),
        model_messages: Vec::new(),
        active_skill_id: None,
        run_entry: None,
        stream_outcome: None,
        usage: None,
        anchor_usage: None,
        group_id: group_id.clone(),
        provider_id: None,
        model: None,
        timestamp: chrono::Local::now().timestamp(),
    };

    conversation.messages.push(user_message.clone());
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

    match compute_context_state(
        &app,
        &state,
        &conversation,
        Some(api_content.as_str()),
        &last_user_image_paths,
    )
    .await
    {
        Ok(context_state) => {
            conversation.context_state = context_state;
            if should_auto_compress_context(&conversation.context_state, &conversation) {
                match compress_conversation_context(&app, &state, &mut conversation, "auto").await {
                    Ok(()) => {
                        let refreshed = compute_context_state(
                            &app,
                            &state,
                            &conversation,
                            Some(api_content.as_str()),
                            &last_user_image_paths,
                        )
                        .await?;
                        conversation.context_state = refreshed.clone();
                        conversation.updated_at = chrono::Local::now().timestamp();
                        save_conversation(&app, &conversation)?;
                        emit_chat_context_state(&app, &conversation.id, &refreshed);
                    }
                    Err(err) => {
                        eprintln!("Auto context compression failed: {err}");
                        if context_likely_over_limit(&conversation.context_state) {
                            rollback_user_message_after_failed_send(
                                &app,
                                &state,
                                &mut conversation,
                                &user_message.id,
                            )
                            .await?;
                            strip_transcripts_for_frontend(&mut conversation);
                            return Ok(serde_json::json!({
                                "success": false,
                                "conversation": conversation,
                                "error": format!(
                                    "Context is likely over the model limit and automatic compression failed: {err}. Please compress manually or switch to a larger-context model."
                                ),
                            }));
                        }
                        conversation.context_state.warning = Some(format!(
                            "Automatic compression failed: {err}. The uncompressed request was sent because the estimate is still within the model window."
                        ));
                        save_conversation(&app, &conversation)?;
                        emit_chat_context_state(
                            &app,
                            &conversation.id,
                            &conversation.context_state,
                        );
                    }
                }
            } else {
                let context_state = conversation.context_state.clone();
                save_conversation(&app, &conversation)?;
                emit_chat_context_state(&app, &conversation.id, &context_state);
            }
        }
        Err(err) => {
            eprintln!("Context usage estimate failed before send: {err}");
        }
    }

    let forced_skill_id = active_skill_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);

    if fan_out {
        let group_id = group_id.expect("fan_out implies group_id set");
        let fan_out_outcome = run_reply_fan_out(
            &app,
            &state,
            &mut conversation,
            &reply_arms,
            &group_id,
            Some(api_content.as_str()),
            &last_user_image_paths,
            forced_skill_id.as_deref(),
        )
        .await;
        strip_transcripts_for_frontend(&mut conversation);
        return match fan_out_outcome {
            Ok(()) => Ok(serde_json::json!({
                "success": true,
                "conversation": conversation,
            })),
            // 全部臂都失败（非取消）才算硬失败；部分成功在 run_reply_fan_out 内已合并落盘并返回 Ok。
            Err(err) if err == "cancelled" => Ok(serde_json::json!({
                "success": true,
                "conversation": conversation,
            })),
            Err(err) => Ok(serde_json::json!({
                "success": false,
                "conversation": conversation,
                "error": err,
            })),
        };
    }

    let reply_outcome = complete_assistant_reply(
        &app,
        &state,
        &mut conversation,
        Some(title_source.as_str()),
        Some(api_content.as_str()),
        &last_user_image_paths,
        forced_skill_id.as_deref(),
        crate::chat::agent::AgentRunEntry::Send,
    )
    .await;
    // 剥离按臂做、且在各臂最后一次写盘之后。发送前超上下文那条提前返回的分支会先 rollback
    // 再 save_conversation，若在 match 前统一剥，就会把剥光的对话写回磁盘、永久丢掉盘上转录。
    match reply_outcome {
        Ok(()) => {
            strip_transcripts_for_frontend(&mut conversation);
            Ok(serde_json::json!({
                "success": true,
                "conversation": conversation,
            }))
        }
        Err(err) if err == "cancelled" => {
            strip_transcripts_for_frontend(&mut conversation);
            Ok(serde_json::json!({
                "success": true,
                "conversation": conversation,
            }))
        }
        Err(err) => {
            // 生成中途硬失败（403 / 空响应 等）发生在用户消息已落盘之后。**不要回滚**——
            // 把问题留在线程里，用户可一键重试而无需重打（与 chat_regenerate_message 的
            // 错误路径一致：那条路径报错时也保留用户消息）。盘上已是「用户消息、无 assistant」
            // 的干净状态（run_agent_loop 的 Err 在 push_assistant_message 之前冒泡），直接返回即可。
            strip_transcripts_for_frontend(&mut conversation);
            Ok(serde_json::json!({
                "success": false,
                "conversation": conversation,
                "error": err,
            }))
        }
    }
}

/// 取消指定对话的当前 Chat 生成或工具执行。
#[tauri::command]
pub(crate) fn chat_cancel_stream(
    state: State<AppState>,
    conversation_id: String,
) -> Result<(), String> {
    state.cancel_chat_generation(&conversation_id);
    Ok(())
}

/// 响应敏感工具调用确认。
#[tauri::command]
pub(crate) fn chat_confirm_tool_call(
    state: State<AppState>,
    tool_call_id: String,
    approved: bool,
) -> Result<(), String> {
    let sender = state
        .pending_chat_tool_approvals
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&tool_call_id);
    if let Some(sender) = sender {
        let _ = sender.send(approved);
    }
    Ok(())
}

/// 返回开发者「请求调试」缓冲快照（最新在前）。仅内存，未开启开关时通常为空。
#[tauri::command]
pub(crate) fn get_request_debug_records(
    state: State<AppState>,
) -> Vec<crate::chat::request_debug::RequestDebugRecord> {
    crate::chat::request_debug::snapshot(&state)
}

/// 清空开发者「请求调试」缓冲。
#[tauri::command]
pub(crate) fn clear_request_debug_records(state: State<AppState>) {
    crate::chat::request_debug::clear(&state);
}

/// 列出当前仍在运行的后台命令（chat agent 用 `run_command background:true` 起的）。
/// 只返回 Running 的——UI 仅在有后台任务时才显示指示器，终止/退出的不必展示。
#[tauri::command]
pub(crate) fn chat_list_background_commands(state: State<AppState>) -> Vec<serde_json::Value> {
    let map = state.background_commands_handle();
    let map = map.lock().unwrap_or_else(|e| e.into_inner());
    let mut jobs: Vec<&crate::native_tools::BackgroundCommand> = map
        .values()
        .filter(|j| matches!(j.status, crate::native_tools::BackgroundCommandStatus::Running))
        .collect();
    jobs.sort_by_key(|j| j.started_at);
    jobs.into_iter()
        .map(|j| {
            serde_json::json!({
                "jobId": j.job_id,
                "command": j.command,
                "cwd": j.cwd,
                "pid": j.pid,
                "elapsedSecs": j.started_at.elapsed().map(|d| d.as_secs()).unwrap_or(0),
            })
        })
        .collect()
}

/// 从 UI 终止一个后台命令。复用 agent 的 `kill_background`（整组杀 + 标记 Killed）。
#[tauri::command]
pub(crate) fn chat_kill_background_command(
    state: State<AppState>,
    job_id: String,
) -> Result<(), String> {
    crate::native_tools::kill_background(&state, &serde_json::json!({ "job_id": job_id })).map(|_| ())
}

/// 响应会话级文件/命令工具授权请求(按 conversation_id)。
#[tauri::command]
pub(crate) fn chat_respond_session_consent(
    state: State<AppState>,
    conversation_id: String,
    granted: bool,
) -> Result<(), String> {
    let sender = state
        .pending_chat_session_consents
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&conversation_id);
    if let Some(sender) = sender {
        let _ = sender.send(granted);
    }
    Ok(())
}

/// 回答 ask_user 澄清卡片。
#[tauri::command]
pub(crate) fn chat_submit_user_choice(
    state: State<AppState>,
    tool_call_id: String,
    answers: HashMap<String, crate::chat::ask_user::AskUserAnswer>,
    skipped: bool,
) -> Result<(), String> {
    let response = {
        let pending = state
            .pending_chat_user_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(pending) = pending.get(&tool_call_id) else {
            return Err("Clarification is no longer awaiting a response".to_string());
        };
        if skipped {
            crate::chat::ask_user::skipped_response()
        } else {
            crate::chat::ask_user::validate_response(
                &pending.prompt,
                crate::chat::ask_user::AskUserResponseResult {
                    phase: crate::chat::ask_user::ASK_USER_PHASE_ANSWERED.to_string(),
                    answers,
                },
            )?
        }
    };
    let pending = state
        .pending_chat_user_prompts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&tool_call_id);
    let Some(pending) = pending else {
        return Err("Clarification is no longer awaiting a response".to_string());
    };
    let _ = pending.sender.send(response);
    Ok(())
}

/// 前端 Pyodide 执行完成后回传结果。
#[tauri::command]
pub(crate) fn chat_python_complete(
    state: State<AppState>,
    run_id: String,
    content: String,
    is_error: bool,
    artifacts: Option<Vec<ChatToolArtifact>>,
) -> Result<(), String> {
    let pending = state
        .pending_python_runs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    if let Some(pending) = pending {
        let _ = pending.sender.send(crate::mcp::types::PythonRunResult {
            content,
            is_error,
            artifacts: artifacts.unwrap_or_default(),
        });
    }
    Ok(())
}

/// 读取附件为 data URL，供前端 `<img>` 预览。`conversation_id` 为空时按本机绝对路径读取（发送前预览）。
#[tauri::command]
pub(crate) fn chat_read_attachment(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<serde_json::Value, String> {
    let full = resolve_attachment_file_path(&app, conversation_id.as_deref(), &path)?;
    let data_url = read_attachment_as_data_url(&full)?;
    Ok(serde_json::json!({
        "success": true,
        "data": data_url,
    }))
}

/// 用系统默认应用打开附件。
#[tauri::command]
#[allow(deprecated)]
pub(crate) fn chat_open_attachment(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<(), String> {
    let full = resolve_attachment_file_path(&app, conversation_id.as_deref(), &path)?;
    let path_str = full.to_string_lossy().into_owned();
    app.shell().open(path_str, None).map_err(|e| e.to_string())
}

/// 用系统默认应用打开生成产物文件。仅允许打开 Kivio sandbox export 目录下的文件。
#[tauri::command]
#[allow(deprecated)]
pub(crate) fn chat_open_generated_artifact(app: AppHandle, path: String) -> Result<(), String> {
    let full = crate::native_tools::resolve_sandbox_export_file_path(&path)?;
    let path_str = full.to_string_lossy().into_owned();
    app.shell().open(path_str, None).map_err(|e| e.to_string())
}

/// 在文件系统中打开生成产物所在目录。仅允许 Kivio sandbox export 目录下的文件。
#[tauri::command]
#[allow(deprecated)]
pub(crate) fn chat_reveal_generated_artifact(app: AppHandle, path: String) -> Result<(), String> {
    let full = crate::native_tools::resolve_sandbox_export_file_path(&path)?;
    let parent = full
        .parent()
        .ok_or_else(|| "Generated file has no parent directory".to_string())?;
    let path_str = parent.to_string_lossy().into_owned();
    app.shell().open(path_str, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn chat_save_pasted_image(
    name: String,
    mime_type: String,
    data_base64: String,
) -> Result<serde_json::Value, String> {
    match save_pasted_image(&name, &mime_type, &data_base64)? {
        PastedImageSave::Saved {
            path,
            name,
            mime_type,
        } => Ok(serde_json::json!({
            "success": true,
            "path": path.to_string_lossy(),
            "name": name,
            "mimeType": mime_type,
        })),
        PastedImageSave::Failed { error } => Ok(serde_json::json!({
            "success": false,
            "error": error,
        })),
    }
}

#[tauri::command]
pub(crate) fn chat_save_pasted_attachment(
    name: String,
    data_base64: String,
) -> Result<serde_json::Value, String> {
    match save_pasted_attachment(&name, &data_base64)? {
        PastedAttachmentSave::Saved { path, name } => Ok(serde_json::json!({
            "success": true,
            "path": path.to_string_lossy(),
            "name": name,
        })),
        PastedAttachmentSave::Failed { error } => Ok(serde_json::json!({
            "success": false,
            "error": error,
        })),
    }
}

/// 读取系统剪贴板中的文件路径（Finder / 资源管理器复制文件）。
#[tauri::command]
pub(crate) fn chat_read_clipboard_files() -> Result<serde_json::Value, String> {
    use arboard::Clipboard;

    let mut clipboard = Clipboard::new().map_err(|e| format!("读取剪贴板失败: {e}"))?;
    let paths = match clipboard.get().file_list() {
        Ok(paths) => paths,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": true,
                "files": [],
            }));
        }
    };

    let files: Vec<Value> = paths
        .into_iter()
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().to_string();
            if !is_attachable_file_name(&name) {
                return None;
            }
            Some(serde_json::json!({
                "path": path.to_string_lossy(),
                "name": name,
            }))
        })
        .collect();

    Ok(serde_json::json!({
        "success": true,
        "files": files,
    }))
}

async fn complete_assistant_reply(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    title_from_first_user: Option<&str>,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
    active_skill_id: Option<&str>,
    entry: crate::chat::agent::AgentRunEntry,
) -> Result<(), String> {
    complete_assistant_reply_inner(
        app,
        state,
        conversation,
        title_from_first_user,
        last_user_api_content,
        last_user_image_paths,
        active_skill_id,
        entry,
        None,
        false,
    )
    .await
    .map(|_| ())
}

/// 共享实现：`arm = None` 为单模型现状（直接落盘，返回 `Ok(())` 语义不变）；
/// `arm = Some(..)` 为多模型臂（用臂的 provider/model、自动批准工具、**不落盘**，
/// 把产出的 assistant 消息通过 `ArmReplyOutcome.message` 返回给协调者）。
async fn complete_assistant_reply_inner(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    title_from_first_user: Option<&str>,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
    active_skill_id: Option<&str>,
    entry: crate::chat::agent::AgentRunEntry,
    arm: Option<&ReplyArm>,
    probe: bool,
) -> Result<ArmReplyOutcome, String> {
    if conversation.agent_runtime.is_external() {
        // 外部 CLI 路径在 run.rs 内自带 generation；这里登记一条 per-run 回复槽位，
        // 让 `conversation_has_active_reply` 在外部回复期间也能拒绝并发新发送（防回归）。
        let ext_generation = state.next_chat_generation(&conversation.id);
        let ext_run_id = format!("chat-run-ext-{}-{}", ext_generation, Uuid::new_v4());
        let _ext_reply_guard =
            ChatReplyGuard::try_new(state.inner(), &conversation.id, &ext_run_id, ext_generation);
        let latest_user = conversation
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        return crate::external_agents::run_external_cli_reply(
            app,
            state,
            conversation,
            title_from_first_user,
            &latest_user,
            active_skill_id,
            entry,
        )
        .await
        .map(|_| ArmReplyOutcome { message: None });
    }

    let settings = state.settings_read().clone();
    // 多模型臂用自己的 provider/model；单模型用会话级（行为不变）。
    // 提前转成 owned，避免对 `conversation` 的长期不可变借用挡住后续的 `&mut conversation`。
    let resolved_provider_id = arm
        .map(|a| a.provider_id.clone())
        .unwrap_or_else(|| conversation.provider_id.clone());
    let resolved_model = arm
        .map(|a| a.model.clone())
        .unwrap_or_else(|| conversation.model.clone());
    let provider = settings
        .get_provider(&resolved_provider_id)
        .ok_or_else(|| "Chat provider not found".to_string())?
        .clone();
    if provider.api_keys.is_empty() {
        return Err(format_chat_missing_api_key_error(&provider.name));
    }
    if resolved_model.trim().is_empty() {
        return Err(chat_missing_model_error());
    }

    let last_user_idx = conversation.messages.iter().rposition(|m| m.role == "user");
    let language = crate::settings::resolve_chat_language(&settings);
    let stream_enabled = settings.chat.stream_enabled;
    // 思考：每对话等级覆盖全局开关。None=跟随全局（现状）；"off"=强制关；low/medium/high=按家族注入。
    let (thinking_enabled, thinking_level) =
        resolve_thinking(conversation.thinking_level.as_deref(), settings.chat.thinking_enabled);
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    let run_generation = state.next_chat_generation(&conversation.id);
    let run_id = format!("chat-run-{}-{}", run_generation, Uuid::new_v4());
    let assistant_message_id = format!("msg_{}", Uuid::new_v4());
    // per-run 回复槽位 + 活跃 generation 守卫：本函数任意退出路径（含早返回的直接生图 /
    // 辅助视觉分支）都会 drop 它，释放该 run 的槽位并退役其 generation。同会话多模型并发时
    // 每条 run 各持一个守卫，互不影响。`next_chat_generation` 已登记 generation，这里仅补登
    // run_id 槽位；run_id 由 generation + uuid 拼成，必不重复，try_new 不会返回 None。
    let _reply_guard =
        ChatReplyGuard::try_new(state.inner(), &conversation.id, &run_id, run_generation);
    let plan_mode = crate::chat::plan::is_plan_mode(&conversation.agent_plan_state);
    let orchestrate_mode = crate::chat::plan::is_orchestrate_mode(&conversation.agent_plan_state);
    if !plan_mode && model_can_generate_images_directly(&provider, &resolved_model) {
        if arm.is_some() {
            // 多答 fan-out MVP 不支持「直接生图模型」作为并行臂（生图路径自行落盘，
            // 与多臂统一合并落盘冲突）。该臂直接报错，其它臂不受影响。
            return Err(
                "多模型并行回答暂不支持直接生图模型，请在多答选择中移除该模型。".to_string(),
            );
        }
        return complete_direct_image_generation_reply(
            app,
            state,
            &settings,
            &provider,
            conversation,
            title_from_first_user,
            last_user_api_content,
            last_user_image_paths,
            active_skill_id,
            &run_id,
            assistant_message_id,
            run_generation,
            retry_attempts,
            entry,
        )
        .await
        .map(|_| ArmReplyOutcome { message: None });
    }
    let session = session_model_for_conversation(conversation);
    let auxiliary_vision_model = auxiliary_vision_model_for_images(
        &settings,
        Some(&provider),
        &resolved_model,
        last_user_image_paths,
        Some(session),
    );
    let mut auxiliary_tool_records = Vec::new();
    let auxiliary_vision_result = if let Some(auxiliary_vision_model) = auxiliary_vision_model {
        let mut record = auxiliary_vision_tool_record(
            &settings,
            &auxiliary_vision_model,
            last_user_image_paths.len(),
        );
        let started = Instant::now();
        emit_chat_stream_delta(
            app,
            &conversation.id,
            &run_id,
            &assistant_message_id,
            "",
            None,
            Some(&tool_segment_for_record(&record, 100, None)),
        );
        emit_chat_tool_record(
            app,
            &conversation.id,
            &run_id,
            &assistant_message_id,
            &record,
        );
        let analysis = tokio::select! {
            result = analyze_chat_images_with_auxiliary_model(
                state,
                &settings,
                &auxiliary_vision_model,
                &conversation.id,
                &assistant_message_id,
                last_user_api_content,
                last_user_image_paths,
                retry_attempts,
                &language,
            ) => result,
            _ = wait_for_chat_cancel(state.inner(), &conversation.id, run_generation) => {
                finish_auxiliary_vision_tool_record(
                    &mut record,
                    ToolCallStatus::Cancelled,
                    started,
                    None,
                    Some("Mixer vision analysis cancelled".to_string()),
                );
                emit_chat_tool_record(app, &conversation.id, &run_id, &assistant_message_id, &record);
                auxiliary_tool_records.push(record);
                emit_chat_stream_done(
                    app,
                    &conversation.id,
                    &run_id,
                    &assistant_message_id,
                    "cancelled",
                    "",
                );
                return Err("cancelled".to_string());
            }
        };
        match analysis {
            Ok(result) => {
                finish_auxiliary_vision_tool_record(
                    &mut record,
                    ToolCallStatus::Success,
                    started,
                    Some(truncate_chars(result.content.trim(), 1000)),
                    None,
                );
                emit_chat_tool_record(
                    app,
                    &conversation.id,
                    &run_id,
                    &assistant_message_id,
                    &record,
                );
                auxiliary_tool_records.push(record);
                Some(result)
            }
            Err(err) => {
                finish_auxiliary_vision_tool_record(
                    &mut record,
                    ToolCallStatus::Error,
                    started,
                    None,
                    Some(err.clone()),
                );
                emit_chat_tool_record(
                    app,
                    &conversation.id,
                    &run_id,
                    &assistant_message_id,
                    &record,
                );
                auxiliary_tool_records.push(record);
                return Err(err);
            }
        }
    } else {
        None
    };
    let empty_image_paths: &[PathBuf] = &[];
    let main_image_paths = if auxiliary_vision_result.is_some() {
        empty_image_paths
    } else {
        last_user_image_paths
    };
    let augmented_last_user_content = auxiliary_vision_result.as_ref().map(|result| {
        user_content_with_auxiliary_vision_result(last_user_api_content, result, &language)
    });
    let last_user_content_for_main = augmented_last_user_content
        .as_deref()
        .or(last_user_api_content);
    let skill_registry =
        skills::build_registry(app, &settings.chat_tools.skill_scan_paths).unwrap_or_default();
    let requested_skill_id = active_skill_id.or(conversation.active_skill_id.as_deref());
    let skill_id = resolve_forced_skill_id(
        &settings.chat_tools,
        conversation.assistant_snapshot.as_ref(),
        &skill_registry,
        requested_skill_id,
        &settings.email_accounts,
        crate::settings::obsidian_connector_configured(&settings.obsidian_vault_path),
    );
    if skill_id.is_none() && conversation.active_skill_id.is_some() {
        conversation.active_skill_id = None;
    }
    let active_skill_record = skill_id
        .as_deref()
        .and_then(|id| skill_registry.find(id))
        .cloned();
    let active_skill_detail = skill_id.as_deref().and_then(|id| {
        skills::read_skill_detail(app, &settings.chat_tools.skill_scan_paths, id).ok()
    });
    let mut effective_chat_tools = settings.chat_tools.clone();
    if arm.is_some() || probe {
        // 多答 fan-out（决策 D1 注）：N 条并行 run 若各自弹工具审批会产生 N 倍弹窗、
        // 且无法对应到具体列。多模型臂内一律自动批准（静默执行）。单模型保持原审批策略。
        // probe（无头测试通道）同理：无 GUI 可应答审批，必须自动放行，否则挂起。
        effective_chat_tools.approval_policy = "auto".to_string();
    }
    let (memory_prompt, memory_warning) = chat_memory_prompt_for_request(app, &settings);
    if let Some(warning) = memory_warning.as_ref() {
        conversation.context_state.warning = Some(warning.clone());
    }
    let tools_capable = agent_prepare::chat_tools_capable(
        &effective_chat_tools,
        settings.chat_memory.enabled,
        crate::settings::chat_image_generation_enabled_for_session(
            &settings,
            Some(session_model_for_conversation(conversation)),
        ),
    );
    let mut tools = list_tools_for_chat(
        app,
        state.inner(),
        &settings,
        Some(session_model_for_conversation(conversation)),
    )
    .await;
    agent_prepare::apply_assistant_mcp_restrictions(
        &mut tools,
        conversation.assistant_snapshot.as_ref(),
    );
    let builder_mode = is_builder_conversation(conversation);
    if builder_mode {
        // 搭建会话只暴露 save_assistant,屏蔽文件/命令/MCP/技能等,保持聚焦。
        tools.clear();
        tools.push(crate::mcp::types::native_save_assistant_tool());
    }
    if let Some(skill) = active_skill_record.as_ref() {
        agent_prepare::apply_active_skill_tool_filter(&mut tools, skill);
    }
    apply_inline_code_request_tool_filter(&mut tools, last_user_api_content);
    let blocked_tool_calls = apply_agent_plan_tool_filter(&mut tools, plan_mode);
    let user_tools_available = tools_capable && !tools.is_empty();
    agent_prepare::apply_skill_fallback_when_tools_unavailable(
        &mut effective_chat_tools,
        skill_id.as_deref(),
        user_tools_available,
    );
    let ask_user_tools_available = append_agent_ask_user_tools(&mut tools);
    let todo_tools_available = append_agent_todo_tools(&mut tools);
    // Multi-agent spawn tool (P3): exposure is mode-controlled. Act and
    // Orchestrate both expose the `agent` tool; Plan mode excludes it (spawn is a
    // side-effecting, non-read-only capability).
    if !plan_mode && !builder_mode {
        crate::chat::sub_agent::append_tool_definitions(&mut tools, true);
    }
    // Orchestrate mode raises the autonomy budget: a single user message may
    // need more tool rounds to plan, fan out sub-agents, and aggregate. We lift
    // max_tool_rounds to max(configured, ORCHESTRATE_MIN_TOOL_ROUNDS) but keep
    // unlimited (None) as-is rather than forcing a cap.
    if orchestrate_mode {
        effective_chat_tools.max_tool_rounds = effective_chat_tools
            .max_tool_rounds
            .map(|rounds| rounds.max(crate::settings::ORCHESTRATE_MIN_TOOL_ROUNDS));
    }
    let runtime_tools_available = !tools.is_empty();
    let available_builtin_tools = agent_prepare::available_builtin_tool_names(&tools);
    let agent_todo_prompt = crate::chat::todo::format_prompt(
        &conversation.agent_todo_state,
        todo_tools_available,
    );
    let agent_ask_user_prompt =
        crate::chat::ask_user::format_prompt(ask_user_tools_available);
    let agent_plan_prompt =
        crate::chat::plan::format_prompt(&conversation.agent_plan_state);
    let project_prompt_context = project_prompt_context_for(app, conversation);
    // Persistent per-conversation delivery directory surfaced to the model so it
    // can write deliverable files there (which auto-render as downloadable cards).
    let delivery_dir = crate::native_tools::delivery_dir(&conversation.id)
        .ok()
        .map(|path| path.display().to_string());
    // 集的系统提示词：按对话 set_id 实时取（不冻结），随集编辑立即对集内对话生效。
    let set_system_prompt = conversation
        .set_id
        .as_deref()
        .and_then(|id| find_set_by_id(app, id).ok())
        .map(|set| set.system_prompt)
        .filter(|prompt| !prompt.trim().is_empty());
    let obsidian_vault_path = (!settings.obsidian_vault_path.trim().is_empty())
        .then_some(settings.obsidian_vault_path.as_str());
    let himalaya_binary = crate::connectors::himalaya::resolve_himalaya_binary_when_active(
        &settings.email_accounts,
    )
    .map(|path| path.display().to_string());
    let email_accounts_prompt = crate::settings::email_accounts_system_prompt(
        &settings.email_accounts,
        himalaya_binary.as_deref(),
    );
    let system_prompt = agent_prepare::build_chat_system_prompt(
        &language,
        !main_image_paths.is_empty(),
        thinking_enabled,
        &skill_registry,
        &effective_chat_tools,
        runtime_tools_available,
        &available_builtin_tools,
        skill_id.as_deref(),
        active_skill_detail.as_ref(),
        conversation.assistant_snapshot.as_ref(),
        set_system_prompt.as_deref(),
        settings.chat.system_prompt.as_str(),
        memory_prompt.as_deref(),
        Some(&agent_plan_prompt),
        Some(&agent_ask_user_prompt),
        Some(&agent_todo_prompt),
        project_prompt_context.as_ref(),
        delivery_dir.as_deref(),
        obsidian_vault_path,
        &settings.email_accounts,
        email_accounts_prompt.as_deref(),
    );

    let runtime_messages = build_chat_api_messages(
        &system_prompt,
        conversation,
        last_user_idx,
        last_user_content_for_main,
        main_image_paths,
    )?;
    let mut fallback_chat_tools = effective_chat_tools.clone();
    if skill_id.is_some() && fallback_chat_tools.skill_fallback_mode == "progressive" {
        fallback_chat_tools.skill_fallback_mode = "skill_md_only".to_string();
    }
    let provider_tools_fallback_system_prompt = agent_prepare::build_chat_system_prompt(
        &language,
        !main_image_paths.is_empty(),
        thinking_enabled,
        &skill_registry,
        &fallback_chat_tools,
        false,
        &[],
        skill_id.as_deref(),
        active_skill_detail.as_ref(),
        conversation.assistant_snapshot.as_ref(),
        set_system_prompt.as_deref(),
        settings.chat.system_prompt.as_str(),
        memory_prompt.as_deref(),
        Some(&agent_plan_prompt),
        Some(&crate::chat::ask_user::format_prompt(false)),
        Some(&crate::chat::todo::format_prompt(
            &conversation.agent_todo_state,
            false,
        )),
        project_prompt_context.as_ref(),
        delivery_dir.as_deref(),
        obsidian_vault_path,
        &settings.email_accounts,
        email_accounts_prompt.as_deref(),
    );

    let chat_host = ChatAgentHost {
        app: app.clone(),
        state: state.inner(),
        // 多模型臂不直接落盘（最终由协调者统一 upsert + save），因此抑制 loop 的
        // mid-run 部分快照写盘，避免 N 条并发 run 同写 conversations/{id}.json 的竞态。
        suppress_partial_persist: arm.is_some(),
    };
    // probe（无头测试通道，仅 debug）：换用自动放行审批/consent/ask_user 的 host，
    // 否则模型调用敏感工具或 ask_user 会 await GUI 应答而永久挂起。
    #[cfg(debug_assertions)]
    let probe_host = ProbeAgentHost { state: state.inner() };
    let host: &dyn crate::chat::agent::AgentHost = {
        #[cfg(debug_assertions)]
        {
            if probe {
                &probe_host
            } else {
                &chat_host
            }
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = probe;
            &chat_host
        }
    };
    let executor = RegistryToolExecutor {
        app: app.clone(),
        state: state.inner(),
    };
    let max_output_tokens = chat_max_output_tokens_for_model(
        Some(&provider),
        &resolved_model,
        settings.chat.max_output_tokens,
    );
    // 真实用量锚点：run 首次压缩检查前，用上一轮落盘 usage 把上下文占用锚定到 provider 实报值
    // （对齐 pi/opencode 的 ground-truth 口径，避免字符估算低估导致压缩过晚/超窗）。
    let (initial_anchor_total_tokens, initial_anchor_trailing_estimate) =
        resolve_usage_anchor(conversation, Some(&provider));
    let result = crate::chat::agent::run_agent_loop(
        crate::chat::agent::AgentRunConfig {
            state: state.inner(),
            conversation_id: conversation.id.clone(),
            tool_conversation_id: conversation.id.clone(),
            depth: 0,
            run_id: run_id.clone(),
            message_id: assistant_message_id.clone(),
            generation: run_generation,
            provider,
            model: resolved_model.clone(),
            runtime_messages,
            tools,
            blocked_tool_calls,
            settings: settings.clone(),
            effective_chat_tools,
            language,
            thinking_enabled,
            thinking_level,
            stream_enabled,
            max_output_tokens,
            retry_attempts,
            assistant_snapshot: conversation.assistant_snapshot.clone(),
            provider_tools_fallback_system_prompt,
            initial_anchor_total_tokens,
            initial_anchor_trailing_estimate,
        },
        host,
        &executor,
    )
    .await;
    let result = result?;

    merge_latest_agent_todo_state(app, conversation);
    merge_latest_agent_plan_state(app, conversation);
    let message_plan = capture_agent_plan_draft_if_needed(
        app,
        conversation,
        plan_mode,
        &result.content,
        result.stream_outcome.as_str(),
    );
    let mut segments = auxiliary_tool_segments(&auxiliary_tool_records);
    segments.extend(result.segments);
    let mut tool_records = auxiliary_tool_records;
    tool_records.extend(result.tool_records);
    let run_entry = agent_run_entry_label(entry);
    if let Some(arm) = arm {
        // 多模型臂：构造 assistant 消息但**不落盘**，交协调者统一合并 + 一次性 save。
        let message = build_assistant_message(
            assistant_message_id,
            result.content,
            result.reasoning,
            Vec::new(),
            tool_records,
            result.api_messages,
            segments,
            skill_id.as_deref(),
            Some(run_entry),
            Some(result.stream_outcome.as_str()),
            result.usage,
            result.last_step_usage,
            message_plan,
            Some((
                arm.group_id.clone(),
                resolved_provider_id.clone(),
                resolved_model.clone(),
            )),
        );
        return Ok(ArmReplyOutcome {
            message: Some(message),
        });
    }
    if let Some(boundary) = result.compaction_boundary.clone() {
        conversation
            .context_state
            .compaction_boundaries
            .push(boundary);
    }
    // L2 压缩对齐落盘路径：run 结束时把 L2 产出的 summary 写回 context_state.summary +
    // compression_count（不再只 push boundary）。质量兜底已在 compaction 核心拦截，此处直接采用。
    if let Some(mut summary) = result.compaction_summary.clone() {
        // L2 产出的 summary.source_message_ids 为空（运行时侧拿不到完整 UI id 列表）——
        // 在此按 source_until_message_id 从 conversation 累积（含旧 summary 覆盖范围），
        // 与落盘路径 compact_conversation_inner 口径一致。必须在替换 summary **之前**读旧 S1。
        summary.source_message_ids = crate::chat::agent::compaction::accumulate_source_ids(
            conversation,
            &summary.source_until_message_id,
        );
        conversation.context_state.last_compressed_at = Some(summary.created_at);
        conversation.context_state.compressed_message_count = summary.source_message_ids.len();
        conversation.context_state.compression_count = conversation
            .context_state
            .compression_count
            .saturating_add(1);
        conversation.context_state.summary = Some(summary);
        // R-4：多次链式压缩后提示准确性下降（与 compact_conversation 口径一致）。
        conversation.context_state.warning = crate::chat::agent::compaction::decay_warning_for(
            conversation.context_state.compression_count,
        );
    }
    push_assistant_message(
        app,
        state,
        &settings,
        conversation,
        assistant_message_id,
        result.content,
        result.reasoning,
        Vec::new(),
        tool_records,
        result.api_messages,
        segments,
        skill_id.as_deref(),
        title_from_first_user,
        Some(run_entry),
        Some(result.stream_outcome.as_str()),
        result.usage,
        result.last_step_usage,
        message_plan,
    )
    .await?;
    Ok(ArmReplyOutcome { message: None })
}

/// 多模型一问多答（任务 06-30 步骤 3）的协调者。
///
/// 对每个臂 `(provider_id, model)`：在会话的**独立克隆**上并发跑一次 agent loop
/// （`complete_assistant_reply_inner` 的 arm 模式），各臂自带 message_id/run_id/generation +
/// 共享 `group_id`，工具自动批准、**不直接落盘**。全部臂结束后，把各臂产出的 assistant
/// 消息按 id `upsert` 进真正的 `conversation`、统一计算一次上下文、一次性 `save_conversation`，
/// 从根本上避开 N 条并发 run 同写 `conversations/{id}.json` 的竞态。
///
/// 返回：
/// - 至少一列产出（成功**或**报错）→ `Ok(())`。报错臂也会合成一条 `stream_outcome="error"`
///   的列消息落库，避免整列被吞（只剩能正常回答的模型）。
/// - 全部臂被取消 → `Err("cancelled")`。
/// - 无任何产出（理论兜底）→ `Err(首个错误信息)`。
#[allow(clippy::too_many_arguments)]
async fn run_reply_fan_out(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    arms: &[(String, String)],
    group_id: &str,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
    active_skill_id: Option<&str>,
) -> Result<(), String> {
    // 各臂独立克隆，互不写盘。arm 模式不走 push_assistant_message 的标题生成路径，
    // 故各臂统一传 title=None：多答首条回复的标题留给后续单模型轮或手动重命名
    // （避免 N 个克隆各自异步生成标题再丢弃）。
    let run_entry = agent_run_entry_label(crate::chat::agent::AgentRunEntry::Send);
    let arm_futures = arms.iter().map(|(provider_id, model)| {
        let mut arm_conversation = conversation.clone();
        let provider_id = provider_id.clone();
        let model = model.clone();
        let arm = ReplyArm {
            group_id: group_id.to_string(),
            provider_id: provider_id.clone(),
            model: model.clone(),
        };
        async move {
            let outcome = complete_assistant_reply_inner(
                app,
                state,
                &mut arm_conversation,
                None,
                last_user_api_content,
                last_user_image_paths,
                active_skill_id,
                crate::chat::agent::AgentRunEntry::Send,
                Some(&arm),
                false,
            )
            .await;
            (outcome, provider_id, model)
        }
    });

    let results = futures::future::join_all(arm_futures).await;

    let mut produced = 0usize;
    let mut cancelled = 0usize;
    let mut first_error: Option<String> = None;
    for (outcome, provider_id, model) in results {
        match outcome {
            Ok(ArmReplyOutcome {
                message: Some(message),
            }) => {
                upsert_assistant_message(conversation, message);
                produced += 1;
            }
            Ok(ArmReplyOutcome { message: None }) => {
                // 不应发生（arm 模式必返回消息），保守计为无产出。
            }
            Err(err) if err == "cancelled" => {
                cancelled += 1;
            }
            Err(err) => {
                // 报错臂也保留为一列：否则整列被吞、只剩能正常回答的模型。合成一条
                // content=错误信息、stream_outcome="error" 的 assistant 列消息落库。
                let message =
                    build_error_arm_message(group_id, provider_id, model, err.clone(), run_entry, active_skill_id);
                upsert_assistant_message(conversation, message);
                produced += 1;
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }

    if produced > 0 {
        // 至少一列产出（成功或报错）：合并后统一计算一次上下文并落盘。
        match compute_context_state(app, state, conversation, None, &[]).await {
            Ok(context_state) => {
                conversation.context_state = context_state.clone();
                emit_chat_context_state(app, &conversation.id, &context_state);
            }
            Err(err) => {
                eprintln!("Context usage estimate failed after multi-model fan-out: {err}");
            }
        }
        conversation.updated_at = chrono::Local::now().timestamp();
        save_conversation(app, conversation)?;
        return Ok(());
    }

    if cancelled > 0 && first_error.is_none() {
        return Err("cancelled".to_string());
    }
    Err(first_error.unwrap_or_else(|| "全部模型回答均失败".to_string()))
}

async fn complete_direct_image_generation_reply(
    app: &AppHandle,
    state: &State<'_, AppState>,
    settings: &Settings,
    provider: &ModelProvider,
    conversation: &mut Conversation,
    title_from_first_user: Option<&str>,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
    active_skill_id: Option<&str>,
    run_id: &str,
    assistant_message_id: String,
    run_generation: u64,
    retry_attempts: usize,
    entry: crate::chat::agent::AgentRunEntry,
) -> Result<(), String> {
    if !last_user_image_paths.is_empty() {
        return Err(
            "当前直接选择的生图模型只支持文字生图；图生图/图片编辑请先使用文字提示，或之后单独配置支持图片编辑的流程。"
                .to_string(),
        );
    }

    let prompt = direct_image_generation_prompt(conversation, last_user_api_content)?;
    let arguments = serde_json::json!({
        "prompt": prompt,
        "size": "auto",
        "quality": "auto",
        "n": 1,
    });
    let started = Instant::now();
    emit_chat_stream_delta(
        app,
        &conversation.id,
        run_id,
        &assistant_message_id,
        DIRECT_IMAGE_GENERATION_PENDING,
        None,
        Some(&plain_text_segment(1000, DIRECT_IMAGE_GENERATION_PENDING)),
    );

    let model = conversation.model.clone();
    let result = tokio::select! {
        result = crate::chat::image_generation::generate_image_with_provider(
            state.inner(),
            provider,
            &model,
            &arguments,
            retry_attempts,
            "Chat image generation",
        ) => result,
        _ = wait_for_chat_cancel(state.inner(), &conversation.id, run_generation) => {
            emit_chat_stream_done(
                app,
                &conversation.id,
                run_id,
                &assistant_message_id,
                "cancelled",
                "",
            );
            return Err("cancelled".to_string());
        }
    };

    match result {
        Ok(output) if !output.is_error => {
            let content = direct_image_generation_content(&output.artifacts);
            emit_chat_stream_done(
                app,
                &conversation.id,
                run_id,
                &assistant_message_id,
                "done",
                &content,
            );
            let active_skill = active_skill_id
                .map(str::to_string)
                .or_else(|| conversation.active_skill_id.clone());
            push_assistant_message(
                app,
                state,
                settings,
                conversation,
                assistant_message_id,
                content.clone(),
                None,
                output.artifacts,
                Vec::new(),
                Vec::new(),
                vec![plain_text_segment(1000, content.as_str())],
                active_skill.as_deref(),
                title_from_first_user,
                Some(agent_run_entry_label(entry)),
                Some("completed"),
                None,
                None,
                None,
            )
            .await?;
            Ok(())
        }
        Ok(output) => {
            let err = output.content;
            eprintln!(
                "Direct image generation failed after {}ms: {err}",
                started.elapsed().as_millis()
            );
            Err(err)
        }
        Err(err) => {
            eprintln!(
                "Direct image generation failed after {}ms: {err}",
                started.elapsed().as_millis()
            );
            Err(err)
        }
    }
}

fn agent_run_entry_label(entry: crate::chat::agent::AgentRunEntry) -> &'static str {
    match entry {
        crate::chat::agent::AgentRunEntry::Send => "send",
        crate::chat::agent::AgentRunEntry::Regenerate => "regenerate",
    }
}

fn direct_image_generation_content(artifacts: &[ChatToolArtifact]) -> String {
    artifacts
        .iter()
        .map(|artifact| format!("![{}]({})", artifact.name, artifact.name))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn direct_image_generation_prompt(
    conversation: &Conversation,
    last_user_api_content: Option<&str>,
) -> Result<String, String> {
    let prompt = conversation
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.content.trim())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            last_user_api_content
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| "请输入要生成的图片描述。".to_string())?;
    Ok(truncate_chars(prompt, 8000))
}

/// 多答组的列标识：(group_id, provider_id, model)。单模型为 None（字段写 None）。
type AssistantGroupMeta = (String, String, String);

/// 反向对账:给「有工具分段但无对应记录」的孤立 `tool_call_id` 合成一条中断态
/// (`Cancelled`)占位记录,追加进 `tool_calls`。
///
/// 场景:工具分段在 planning 阶段(解析出调用即)创建并流式推送,记录在 execution
/// 阶段(工具执行时)创建。若一轮在两者之间被中断(网关掐流/400/取消/超时),落库消息
/// 就有分段无记录 → 前端渲染「工具记录缺失」。`normalize_assistant_segments` 只补
/// 「有记录没分段」的正向;此函数补反向,消除困惑呈现,并保留「模型确实发起过该工具」
/// 的痕迹。能从 `api_messages`(OpenAI 线格式 assistant `tool_calls[]`)按 id 回捞
/// name/arguments 就用真值,捞不到留空(前端兜底显示「工具调用」)。对无孤立分段的
/// 消息零副作用(空转)。
fn reconcile_orphan_tool_segments(
    tool_calls: &mut Vec<ToolCallRecord>,
    segments: &[ChatMessageSegment],
    api_messages: &[Value],
) {
    use std::collections::HashSet;
    let record_ids: HashSet<&str> = tool_calls.iter().map(|record| record.id.as_str()).collect();

    // 孤立工具分段的 (id, round),去重保序。
    let mut orphan_ids: Vec<(String, u32)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for segment in segments {
        if segment.kind != ChatMessageSegmentKind::Tool {
            continue;
        }
        let Some(id) = segment.tool_call_id.as_deref() else {
            continue;
        };
        if id.is_empty() || record_ids.contains(id) || !seen.insert(id.to_string()) {
            continue;
        }
        orphan_ids.push((id.to_string(), segment.round.unwrap_or(0)));
    }
    if orphan_ids.is_empty() {
        return;
    }

    let now = chrono::Local::now().timestamp();
    for (id, round) in orphan_ids {
        let (name, arguments) = tool_call_meta_from_api_messages(api_messages, &id);
        tool_calls.push(ToolCallRecord {
            id,
            name,
            source: String::new(),
            server_id: None,
            arguments,
            status: ToolCallStatus::Cancelled,
            result_preview: None,
            error: Some("工具调用未完成（会话中断）".to_string()),
            duration_ms: Some(0),
            started_at: Some(now),
            completed_at: Some(now),
            round,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        });
    }
}

/// 从 `api_messages`(OpenAI 线格式)里按 `tool_call_id` 回捞工具调用的
/// `(name, arguments)`。扫每条消息的 `tool_calls[]`,匹配 `id` 命中即返回;未命中
/// 返回 `(空, 空)`。
fn tool_call_meta_from_api_messages(api_messages: &[Value], id: &str) -> (String, String) {
    for message in api_messages {
        let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
            continue;
        };
        for call in calls {
            if call.get("id").and_then(Value::as_str) == Some(id) {
                let function = call.get("function");
                let name = function
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = function
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                return (name, arguments);
            }
        }
    }
    (String::new(), String::new())
}

/// 构造一条 assistant `ChatMessage`（含 segment 归一、model_messages 计算）。
/// `push_assistant_message`（落盘路径）与多模型臂（返回消息交协调者落盘）共用此函数，
/// 保证两条路径生成的消息形态一致。`group_meta = Some(..)` 时写入 group_id/provider_id/model。
#[allow(clippy::too_many_arguments)]
fn build_assistant_message(
    message_id: String,
    content: String,
    reasoning: Option<String>,
    artifacts: Vec<ChatToolArtifact>,
    mut tool_calls: Vec<ToolCallRecord>,
    api_messages: Vec<Value>,
    segments: Vec<ChatMessageSegment>,
    active_skill_id: Option<&str>,
    run_entry: Option<&str>,
    stream_outcome: Option<&str>,
    usage: Option<crate::chat::model::ModelUsage>,
    anchor_usage: Option<crate::chat::model::ModelUsage>,
    agent_plan: Option<AgentPlanState>,
    group_meta: Option<AssistantGroupMeta>,
) -> ChatMessage {
    // 反向对账:补齐「有工具分段无记录」的孤立调用为中断态记录，避免前端显示
    // 「工具记录缺失」。在 normalize（正向补段）之前跑，使新记录与既有分段自然对上。
    reconcile_orphan_tool_segments(&mut tool_calls, &segments, &api_messages);
    let segments =
        normalize_assistant_segments(&content, reasoning.as_deref(), &tool_calls, segments);
    let stored_content = content_from_segments(&segments).unwrap_or_else(|| content.clone());
    let stored_reasoning = reasoning_from_segments(&segments).or(reasoning);

    // model_messages 是规范回放源（build_chat_api_messages 优先用它）。算好后，若它
    // 非空就丢弃冗余的 api_messages（OpenAI 线格式）——回放/编辑路径仅在 model_messages
    // 为空时才回落 api_messages，前端更是从不读它。省 RAM/磁盘/IPC。为空兜底（罕见：
    // 转换产出空）才保留 api_messages，避免丢工具上下文。中断草稿走另一条路
    // (persist_partial_assistant_snapshot)，那里仍保留 api_messages 以保「继续」可回放。
    let model_messages = assistant_model_messages_for_storage(
        &stored_content,
        stored_reasoning.as_deref(),
        &api_messages,
        &tool_calls,
    );
    let api_messages = if model_messages.is_empty() {
        api_messages
    } else {
        Vec::new()
    };

    let (group_id, provider_id, model) = match group_meta {
        Some((g, p, m)) => (Some(g), Some(p), Some(m)),
        None => (None, None, None),
    };

    ChatMessage {
        id: message_id,
        role: "assistant".to_string(),
        content: stored_content,
        attachments: vec![],
        reasoning: stored_reasoning,
        artifacts,
        model_messages,
        tool_calls,
        segments,
        agent_plan,
        api_messages,
        active_skill_id: active_skill_id.map(|id| id.to_string()),
        run_entry: run_entry.map(str::to_string),
        stream_outcome: stream_outcome.map(str::to_string),
        usage,
        anchor_usage,
        group_id,
        provider_id,
        model,
        timestamp: chrono::Local::now().timestamp(),
    }
}

/// 多答 fan-out 中某个臂报错时，合成一条「错误列」assistant 消息（不落盘由调用者 upsert）。
/// 复用 build_assistant_message 保证列形态一致：带 group_id/provider/model，content 为错误信息，
/// stream_outcome 标 "error"。这样报错的模型仍保留为一列，而不是被整列吞掉。
fn build_error_arm_message(
    group_id: &str,
    provider_id: String,
    model: String,
    error: String,
    run_entry: &str,
    active_skill_id: Option<&str>,
) -> ChatMessage {
    build_assistant_message(
        format!("msg_{}", Uuid::new_v4()),
        error,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        active_skill_id,
        Some(run_entry),
        Some("error"),
        None,
        None,
        None,
        Some((group_id.to_string(), provider_id, model)),
    )
}

pub(crate) async fn push_assistant_message(
    app: &AppHandle,
    state: &State<'_, AppState>,
    settings: &Settings,
    conversation: &mut Conversation,
    message_id: String,
    content: String,
    reasoning: Option<String>,
    artifacts: Vec<ChatToolArtifact>,
    tool_calls: Vec<ToolCallRecord>,
    api_messages: Vec<Value>,
    segments: Vec<ChatMessageSegment>,
    active_skill_id: Option<&str>,
    title_from_first_user: Option<&str>,
    run_entry: Option<&str>,
    stream_outcome: Option<&str>,
    usage: Option<crate::chat::model::ModelUsage>,
    anchor_usage: Option<crate::chat::model::ModelUsage>,
    agent_plan: Option<AgentPlanState>,
) -> Result<(), String> {
    let message = build_assistant_message(
        message_id,
        content.clone(),
        reasoning,
        artifacts,
        tool_calls,
        api_messages,
        segments,
        active_skill_id,
        run_entry,
        stream_outcome,
        usage,
        anchor_usage,
        agent_plan,
        // 单模型落盘路径不带 group 信息（行为不变）。
        None,
    );
    let stored_content = message.content.clone();
    let generated_title = if let Some(user_content) = title_from_first_user {
        if conversation.messages.len() == 1 && conversation.title == "新对话" {
            // 被取消的首条回复不值得花一次模型调用生成标题（标题生成是一次
            // 带 8s 超时的 LLM 请求，会显著拖慢"停止"后 invoke 的返回 / 输入框解锁）。
            // 用本地启发式标题兜底；下一条正常回复或重命名仍可得到更好的标题。
            if stream_outcome == Some("cancelled") {
                Some(generate_title(user_content))
            } else {
                Some(
                    resolve_conversation_title(
                        settings,
                        state,
                        conversation,
                        user_content,
                        &stored_content,
                    )
                    .await,
                )
            }
        } else {
            None
        }
    } else {
        None
    };

    upsert_assistant_message(conversation, message);

    if let Some(title) = generated_title {
        conversation.title = title;
    }

    match compute_context_state(app, state, conversation, None, &[]).await {
        Ok(context_state) => {
            conversation.context_state = context_state.clone();
            try_auto_compress_context_after_update(app, state, conversation, None, &[]).await;
            emit_chat_context_state(app, &conversation.id, &conversation.context_state);
        }
        Err(err) => {
            eprintln!("Context usage estimate failed after assistant reply: {err}");
        }
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(app, conversation)?;
    Ok(())
}

/// Insert an assistant message, replacing any existing message that already
/// carries the same id. The agent loop's per-round crash-safety checkpoint
/// writes a draft assistant message under the run's `message_id`; both that
/// draft path and the final write go through here so a completed run cleanly
/// overwrites its own draft instead of appending a duplicate.
fn upsert_assistant_message(conversation: &mut Conversation, message: ChatMessage) {
    if let Some(pos) = conversation
        .messages
        .iter()
        .position(|existing| existing.id == message.id)
    {
        conversation.messages[pos] = message;
    } else {
        conversation.messages.push(message);
    }
}

/// Write a best-effort snapshot of the in-progress assistant turn to disk so a
/// mid-run crash / forced exit doesn't discard the whole reply. Reloads the
/// conversation (to pick up todo/plan/user state already persisted by other
/// paths), upserts a draft assistant message keyed by `message_id`, and saves.
/// The draft is marked `interrupted`; the loop's final write replaces it with
/// the completed message. `api_messages` carries the loop's accumulated
/// provider messages (assistant tool_calls + tool results) so the draft stays
/// replayable on a later "continue" — `model_messages` are derived from them
/// exactly as the final write does, keeping the storage shape consistent. No-op
/// when nothing has been produced yet.
fn persist_partial_assistant_snapshot(
    app: &AppHandle,
    conversation_id: &str,
    message_id: &str,
    tool_records: &[ToolCallRecord],
    segments: &[ChatMessageSegment],
    api_messages: &[Value],
) -> Result<(), String> {
    if tool_records.is_empty() && segments.is_empty() {
        return Ok(());
    }
    let mut conversation = load_conversation(app, conversation_id)?;
    let segments = segments.to_vec();
    // 中断草稿是「永不完成」run 的最终存档，最易出孤立工具分段——同样反向对账补齐。
    let mut tool_records = tool_records.to_vec();
    reconcile_orphan_tool_segments(&mut tool_records, &segments, api_messages);
    let content = content_from_segments(&segments).unwrap_or_default();
    let reasoning = reasoning_from_segments(&segments);
    let model_messages = assistant_model_messages_for_storage(
        &content,
        reasoning.as_deref(),
        api_messages,
        &tool_records,
    );
    let draft = ChatMessage {
        id: message_id.to_string(),
        role: "assistant".to_string(),
        content,
        attachments: Vec::new(),
        reasoning,
        artifacts: Vec::new(),
        tool_calls: tool_records,
        segments,
        agent_plan: None,
        api_messages: api_messages.to_vec(),
        model_messages,
        active_skill_id: None,
        run_entry: None,
        stream_outcome: Some("interrupted".to_string()),
        usage: None,
        anchor_usage: None,
        group_id: None,
        provider_id: None,
        model: None,
        timestamp: chrono::Local::now().timestamp(),
    };
    upsert_assistant_message(&mut conversation, draft);
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(app, &conversation)
}

fn normalize_assistant_segments(
    content: &str,
    reasoning: Option<&str>,
    tool_calls: &[ToolCallRecord],
    mut segments: Vec<ChatMessageSegment>,
) -> Vec<ChatMessageSegment> {
    if segments.is_empty() {
        segments = synthesize_assistant_segments(content, reasoning, tool_calls);
    }

    let mut next_order = next_segment_order(&segments);
    if !content.trim().is_empty() && content_from_segments(&segments).is_none() {
        segments.push(ChatMessageSegment {
            id: format!("seg_{}_synthesis_text", next_order),
            kind: ChatMessageSegmentKind::Text,
            phase: if tool_calls.is_empty() {
                ChatMessageSegmentPhase::Plain
            } else {
                ChatMessageSegmentPhase::Synthesis
            },
            order: next_order,
            step_number: None,
            round: None,
            text: Some(content.to_string()),
            tool_call_id: None,
        });
        next_order = next_order.saturating_add(1);
    }

    if reasoning_from_segments(&segments).is_none() {
        if let Some(reasoning) = reasoning.map(str::trim).filter(|value| !value.is_empty()) {
            segments.push(ChatMessageSegment {
                id: format!("seg_{}_reasoning", next_order),
                kind: ChatMessageSegmentKind::Reasoning,
                phase: ChatMessageSegmentPhase::Synthesis,
                order: next_order,
                step_number: None,
                round: None,
                text: Some(reasoning.to_string()),
                tool_call_id: None,
            });
        }
    }

    let existing_tool_segment_ids = segments
        .iter()
        .filter_map(|segment| {
            if segment.kind == ChatMessageSegmentKind::Tool {
                segment.tool_call_id.clone()
            } else {
                None
            }
        })
        .collect::<std::collections::HashSet<_>>();
    let mut missing_records: Vec<&ToolCallRecord> = tool_calls
        .iter()
        .filter(|record| !existing_tool_segment_ids.contains(&record.id))
        .collect();
    missing_records.sort_by_key(|record| record.started_at.unwrap_or(0));
    if !missing_records.is_empty() {
        let synthesis_start = segments
            .iter()
            .filter(|segment| segment.phase == ChatMessageSegmentPhase::Synthesis)
            .map(|segment| segment.order)
            .min();
        for record in missing_records {
            let insert_at = segments
                .iter()
                .filter(|segment| synthesis_start.is_none_or(|start| segment.order < start))
                .map(|segment| segment.order)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            for segment in segments.iter_mut() {
                if segment.order >= insert_at {
                    segment.order = segment.order.saturating_add(1);
                }
            }
            segments.push(tool_segment_for_record(record, insert_at, None));
        }
    }

    segments.sort_by_key(|segment| segment.order);
    segments
}

fn synthesize_assistant_segments(
    content: &str,
    reasoning: Option<&str>,
    tool_calls: &[ToolCallRecord],
) -> Vec<ChatMessageSegment> {
    let mut segments = Vec::new();
    let mut order = 1000u32;
    for record in tool_calls {
        segments.push(tool_segment_for_record(record, order, None));
        order = order.saturating_add(1);
    }
    if let Some(reasoning) = reasoning.map(str::trim).filter(|value| !value.is_empty()) {
        segments.push(ChatMessageSegment {
            id: format!("seg_{}_reasoning", order),
            kind: ChatMessageSegmentKind::Reasoning,
            phase: if tool_calls.is_empty() {
                ChatMessageSegmentPhase::Plain
            } else {
                ChatMessageSegmentPhase::Synthesis
            },
            order,
            step_number: None,
            round: None,
            text: Some(reasoning.to_string()),
            tool_call_id: None,
        });
        order = order.saturating_add(1);
    }
    if !content.trim().is_empty() {
        segments.push(ChatMessageSegment {
            id: format!("seg_{}_text", order),
            kind: ChatMessageSegmentKind::Text,
            phase: if tool_calls.is_empty() {
                ChatMessageSegmentPhase::Plain
            } else {
                ChatMessageSegmentPhase::Synthesis
            },
            order,
            step_number: None,
            round: None,
            text: Some(content.to_string()),
            tool_call_id: None,
        });
    }
    segments
}

fn auxiliary_tool_segments(records: &[ToolCallRecord]) -> Vec<ChatMessageSegment> {
    records
        .iter()
        .enumerate()
        .map(|(index, record)| tool_segment_for_record(record, 100 + index as u32, None))
        .collect()
}

fn tool_segment_for_record(
    record: &ToolCallRecord,
    order: u32,
    step_number: Option<u8>,
) -> ChatMessageSegment {
    ChatMessageSegment {
        id: format!("seg_{}_tool_{}", order, record.id),
        kind: ChatMessageSegmentKind::Tool,
        phase: if record.round == 0 || record.source == "mixer" {
            ChatMessageSegmentPhase::Auxiliary
        } else {
            ChatMessageSegmentPhase::ToolLoop
        },
        order,
        step_number,
        round: Some(record.round),
        text: None,
        tool_call_id: Some(record.id.clone()),
    }
}

fn plain_text_segment(order: u32, text: &str) -> ChatMessageSegment {
    ChatMessageSegment {
        id: format!("seg_{}_plain_text", order),
        kind: ChatMessageSegmentKind::Text,
        phase: ChatMessageSegmentPhase::Plain,
        order,
        step_number: None,
        round: None,
        text: Some(text.to_string()),
        tool_call_id: None,
    }
}

fn content_from_segments(segments: &[ChatMessageSegment]) -> Option<String> {
    let content = joined_segment_text(segments, |segment| {
        segment.kind == ChatMessageSegmentKind::Text
            && matches!(
                segment.phase,
                ChatMessageSegmentPhase::Plain | ChatMessageSegmentPhase::Synthesis
            )
    });
    if content.trim().is_empty() {
        None
    } else {
        Some(content)
    }
}

fn reasoning_from_segments(segments: &[ChatMessageSegment]) -> Option<String> {
    let reasoning = joined_segment_text(segments, |segment| {
        segment.kind == ChatMessageSegmentKind::Reasoning
    });
    if reasoning.trim().is_empty() {
        None
    } else {
        Some(reasoning)
    }
}

fn joined_segment_text(
    segments: &[ChatMessageSegment],
    predicate: impl Fn(&ChatMessageSegment) -> bool,
) -> String {
    let mut parts = segments
        .iter()
        .filter(|segment| predicate(segment))
        .filter_map(|segment| {
            let text = segment.text.as_deref()?.trim();
            if text.is_empty() {
                None
            } else {
                Some((segment.order, text.to_string()))
            }
        })
        .collect::<Vec<_>>();
    parts.sort_by_key(|(order, _)| *order);
    parts
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn next_segment_order(segments: &[ChatMessageSegment]) -> u32 {
    segments
        .iter()
        .map(|segment| segment.order)
        .max()
        .unwrap_or(999)
        .saturating_add(1)
}

fn replace_final_text_segments_for_edit(message: &mut ChatMessage, content: &str) {
    let mut segments = if message.segments.is_empty() {
        synthesize_assistant_segments(
            &message.content,
            message.reasoning.as_deref(),
            &message.tool_calls,
        )
    } else {
        std::mem::take(&mut message.segments)
    };
    segments.retain(|segment| {
        !(segment.kind == ChatMessageSegmentKind::Text
            && matches!(
                segment.phase,
                ChatMessageSegmentPhase::Plain | ChatMessageSegmentPhase::Synthesis
            ))
    });
    let order = next_segment_order(&segments);
    segments.push(ChatMessageSegment {
        id: format!("seg_{}_edited_synthesis", order),
        kind: ChatMessageSegmentKind::Text,
        phase: if message.tool_calls.is_empty() {
            ChatMessageSegmentPhase::Plain
        } else {
            ChatMessageSegmentPhase::Synthesis
        },
        order,
        step_number: None,
        round: None,
        text: Some(content.to_string()),
        tool_call_id: None,
    });
    segments.sort_by_key(|segment| segment.order);
    message.segments = segments;
    message.content =
        content_from_segments(&message.segments).unwrap_or_else(|| content.to_string());
    message.reasoning = reasoning_from_segments(&message.segments);
    message.model_messages = edited_assistant_model_messages(message);
    message.api_messages = Vec::new();
}

fn edited_assistant_model_messages(message: &ChatMessage) -> Vec<ModelMessage> {
    let mut replay = message.model_messages.clone();
    if replay.is_empty() && !message.api_messages.is_empty() {
        replay = model_messages_from_openai_messages(message.api_messages.clone());
    }

    let edited_answer = assistant_model_messages_for_storage(
        &message.content,
        message.reasoning.as_deref(),
        &[],
        &[],
    );
    if edited_answer.is_empty() {
        return Vec::new();
    }

    if let Some(final_answer_idx) = replay.iter().rposition(|model_message| {
        model_message.role == ModelRole::Assistant
            && !model_message
                .content
                .iter()
                .any(|part| matches!(part, MessagePart::ToolCall { .. }))
    }) {
        replay.truncate(final_answer_idx);
        replay.extend(edited_answer);
        replay
    } else if replay.is_empty() {
        edited_answer
    } else {
        replay.extend(edited_answer);
        replay
    }
}

fn merge_latest_agent_todo_state(app: &AppHandle, conversation: &mut Conversation) {
    match load_conversation(app, &conversation.id) {
        Ok(latest) => {
            conversation.agent_todo_state = latest.agent_todo_state;
        }
        Err(err) => {
            eprintln!("Failed to reload latest agent todo state before saving reply: {err}");
        }
    }
}

fn merge_latest_agent_plan_state(app: &AppHandle, conversation: &mut Conversation) {
    match load_conversation(app, &conversation.id) {
        Ok(latest) => {
            conversation.agent_plan_state = latest.agent_plan_state;
        }
        Err(err) => {
            eprintln!("Failed to reload latest agent plan state before saving reply: {err}");
        }
    }
}

fn capture_agent_plan_draft_if_needed(
    app: &AppHandle,
    conversation: &mut Conversation,
    original_plan_mode: bool,
    content: &str,
    stream_outcome: &str,
) -> Option<AgentPlanState> {
    if stream_outcome != "completed"
        || !original_plan_mode
        || !crate::chat::plan::is_plan_mode(&conversation.agent_plan_state)
    {
        return None;
    }
    let next_state =
        crate::chat::plan::capture_draft_from_reply(&conversation.agent_plan_state, content);
    if next_state == conversation.agent_plan_state {
        return if crate::chat::plan::executable_plan_text(&next_state)
            .is_some_and(|plan| plan == content.trim())
        {
            Some(next_state)
        } else {
            None
        };
    }
    conversation.agent_plan_state = next_state.clone();
    emit_chat_plan_state(app, &conversation.id, &next_state);
    Some(next_state)
}

fn assistant_model_messages_for_storage(
    content: &str,
    reasoning: Option<&str>,
    api_messages: &[Value],
    tool_calls: &[ToolCallRecord],
) -> Vec<ModelMessage> {
    if !api_messages.is_empty() {
        let mut canonical = model_messages_from_openai_messages(api_messages.to_vec());
        mark_tool_result_errors(&mut canonical, tool_calls);
        if !canonical.is_empty() {
            return canonical;
        }
    }

    let mut parts = Vec::new();
    if !content.trim().is_empty() {
        parts.push(MessagePart::Text {
            text: content.to_string(),
        });
    }
    if let Some(reasoning) = reasoning.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(MessagePart::Reasoning {
            text: reasoning.to_string(),
        });
    }

    if parts.is_empty() {
        Vec::new()
    } else {
        vec![ModelMessage {
            role: ModelRole::Assistant,
            content: parts,
        }]
    }
}

fn mark_tool_result_errors(messages: &mut [ModelMessage], tool_calls: &[ToolCallRecord]) {
    let error_by_id: HashMap<&str, bool> = tool_calls
        .iter()
        .map(|record| {
            (
                record.id.as_str(),
                matches!(record.status, ToolCallStatus::Error),
            )
        })
        .collect();
    if error_by_id.is_empty() {
        return;
    }

    for message in messages {
        for part in &mut message.content {
            if let MessagePart::ToolResult {
                tool_call_id,
                is_error,
                ..
            } = part
            {
                if let Some(failed) = error_by_id.get(tool_call_id.as_str()) {
                    *is_error = *failed;
                }
            }
        }
    }
}

async fn resolve_conversation_title(
    settings: &Settings,
    state: &State<'_, AppState>,
    conversation: &Conversation,
    user_content: &str,
    assistant_content: &str,
) -> String {
    let session = SessionModel {
        provider_id: conversation.provider_id.as_str(),
        model: conversation.model.as_str(),
    };
    match timeout(
        Duration::from_secs(8),
        generate_title_with_model(
            settings,
            state,
            &conversation.id,
            Some(session),
            user_content,
            assistant_content,
        ),
    )
    .await
    {
        Ok(Some(title)) => title,
        Ok(None) => generate_title(user_content),
        Err(_) => generate_title(user_content),
    }
}

async fn generate_title_with_model(
    settings: &Settings,
    state: &State<'_, AppState>,
    conversation_id: &str,
    session: Option<SessionModel<'_>>,
    user_content: &str,
    assistant_content: &str,
) -> Option<String> {
    let (provider_id, model) = settings.effective_title_summary_model_for_session(session);
    let provider = settings.get_provider(&provider_id)?.clone();
    if provider.api_keys.is_empty() || model.trim().is_empty() {
        return None;
    }
    if model_can_generate_images_directly(&provider, &model) {
        return None;
    }

    let language = crate::settings::resolve_chat_language(settings);
    let prompt = build_title_summary_prompt(user_content, assistant_content, &language);
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    let messages = vec![
        serde_json::json!({
            "role": "system",
            "content": title_summary_system_prompt(&language),
        }),
        serde_json::json!({
            "role": "user",
            "content": prompt,
        }),
    ];
    let message = call_chat_completion_message(
        state,
        &provider,
        &model,
        messages,
        None,
        retry_attempts,
        false,
        Some(conversation_id),
        None,
        "Chat title summary",
    )
    .await
    .ok()?;
    let raw = agent_stop::assistant_content_from_api_message(&message);

    sanitize_generated_title(&raw)
}

fn title_summary_system_prompt(language: &str) -> &'static str {
    if language.starts_with("zh") {
        "你只负责为对话生成简洁标题。只输出标题本身，不要解释。"
    } else {
        "You only generate concise conversation titles. Output only the title, with no explanation."
    }
}

fn build_title_summary_prompt(
    user_content: &str,
    assistant_content: &str,
    language: &str,
) -> String {
    let user = truncate_chars(user_content.trim(), 1200);
    let assistant = truncate_chars(assistant_content.trim(), 1200);
    if language.starts_with("zh") {
        format!(
            "请根据下面的首轮对话生成一个简洁中文标题。\n要求：只输出标题本身；不要引号；不要句号；不超过 14 个汉字，最多 20 个字符。\n\n用户：{user}\n\n助手：{assistant}"
        )
    } else {
        format!(
            "Create a concise English title for this first chat turn.\nRules: output only the title; no quotes; no period; 3-6 words.\n\nUser: {user}\n\nAssistant: {assistant}"
        )
    }
}

fn sanitize_generated_title(raw: &str) -> Option<String> {
    let mut title = raw
        .trim()
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_string();

    title = title
        .trim_start_matches(['-', '*', '•', ' '])
        .trim_matches(['"', '\'', '`', '“', '”', '‘', '’', '。', '.', ' '])
        .to_string();
    for prefix in ["标题：", "标题:", "Title:", "Title：", "title:", "title："] {
        if let Some(rest) = title.strip_prefix(prefix) {
            title = rest.trim().to_string();
        }
    }
    title = title
        .trim_matches(['"', '\'', '`', '“', '”', '‘', '’', '。', '.', ' '])
        .to_string();
    if title.is_empty() {
        return None;
    }
    Some(generate_title(&title))
}

/// Detect a leading `/skill <args>` slash trigger in a user message and, when it
/// matches an enabled skill, rewrite the message body to pin that skill.
///
/// Returns `(skill_id, rewritten_content)` on a match. The rewrite is
/// `"[Skill: name]\n\n{body}"` where `body` is the skill body with `$ARGUMENTS`
/// / `$ARG_NAME` substituted from the trailing words. The resolved id then flows
/// through the existing pin chain (resolve_forced_skill_id → active_skill_record
/// → apply_active_skill_tool_filter + catalog/pin injection).
///
/// `disable_model_invocation` only gates *model* auto-invocation, so it is
/// intentionally ignored here — an explicit user slash command may still trigger
/// such a skill. Availability is gated by `skill_allowed_for_conversation`
/// (Settings enable list, connector prerequisites, assistant allow-list).
fn try_apply_skill_slash_trigger(
    registry: &skills::SkillRegistry,
    chat_tools: &crate::settings::ChatToolsConfig,
    assistant_snapshot: Option<&crate::chat::types::ChatAssistantSnapshot>,
    content: &str,
    email_accounts: &[crate::settings::EmailAccountConfig],
    obsidian_vault_configured: bool,
) -> Option<(String, String)> {
    let trimmed = content.trim_start();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first_word = parts.next().unwrap_or_default();
    if !first_word.starts_with('/') {
        return None;
    }
    let args_raw = parts.next().unwrap_or_default();

    let record = registry.find_by_trigger(first_word)?;
    if !agent_prepare::skill_allowed_for_conversation(
        chat_tools,
        assistant_snapshot,
        &record.meta.id,
        email_accounts,
        obsidian_vault_configured,
    )
    {
        // A disabled or out-of-allow-list skill's slash command is left as ordinary text.
        return None;
    }
    if crate::mcp::native_registry::find_entry(first_word.trim_start_matches('/')).is_some() {
        // A skill id colliding with a built-in tool name would shadow it on the
        // backend trigger path. The front-end intercepts built-in slash commands
        // before send, so this is low risk — just note it.
        eprintln!(
            "[skill-slash] trigger {first_word} matches a built-in tool name; pinning skill {}",
            record.meta.id
        );
    }

    let rendered = skills::substitute_arguments(&record.body, args_raw, &record.meta.arguments);
    let rewritten = format!("[Skill: {}]\n\n{}", record.meta.name, rendered);
    Some((record.meta.id.clone(), rewritten))
}

fn resolve_forced_skill_id(
    chat_tools: &crate::settings::ChatToolsConfig,
    assistant_snapshot: Option<&crate::chat::types::ChatAssistantSnapshot>,
    registry: &skills::SkillRegistry,
    requested: Option<&str>,
    email_accounts: &[crate::settings::EmailAccountConfig],
    obsidian_vault_configured: bool,
) -> Option<String> {
    let requested = requested.map(str::trim).filter(|id| !id.is_empty())?;
    let enabled = registry
        .records
        .iter()
        .filter(|record| {
            agent_prepare::skill_allowed_for_conversation(
                chat_tools,
                assistant_snapshot,
                &record.meta.id,
                email_accounts,
                obsidian_vault_configured,
            )
        })
        .any(|record| {
            record.meta.id == requested
                || record.meta.name == requested
                || skills::slugify(requested) == record.meta.id
        });
    if enabled {
        Some(requested.to_string())
    } else {
        None
    }
}

fn emit_chat_plan_state(app: &AppHandle, conversation_id: &str, plan_state: &AgentPlanState) {
    let _ = app.emit(
        "chat-plan",
        serde_json::json!({
            "conversationId": conversation_id,
            "planState": plan_state,
        }),
    );
}

async fn list_tools_for_chat(
    app: &AppHandle,
    state: &AppState,
    settings: &Settings,
    session: Option<SessionModel<'_>>,
) -> Vec<ChatToolDefinition> {
    if !(settings.chat_tools.enabled
        || crate::settings::chat_native_tools_enabled(&settings.chat_tools)
        || crate::settings::chat_memory_tools_enabled(settings)
        || crate::settings::chat_image_generation_enabled_for_session(settings, session)
        || settings.advisor_model().is_some())
    {
        return Vec::new();
    }
    let mut tools = mcp::registry::list_enabled_tool_defs(app, state)
        .await
        .unwrap_or_default();
    if let Some((provider_id, model)) =
        crate::chat::model_metadata::image_generation_model_for_session(settings, session)
    {
        if !tools
            .iter()
            .any(|tool| tool.name == "mixer_generate_image")
        {
            let mut tool = mcp::types::mixer_generate_image_tool();
            let provider_name = settings
                .get_provider(&provider_id)
                .map(|provider| {
                    if provider.name.trim().is_empty() {
                        provider.id.clone()
                    } else {
                        provider.name.clone()
                    }
                })
                .unwrap_or(provider_id);
            tool.server_id = Some(format!("{provider_name} / {model}"));
            tools.push(tool);
        }
    }
    tools
}

fn append_agent_todo_tools(tools: &mut Vec<ChatToolDefinition>) -> bool {
    crate::chat::todo::append_tool_definitions(tools);
    true
}

fn append_agent_ask_user_tools(tools: &mut Vec<ChatToolDefinition>) -> bool {
    crate::chat::ask_user::append_tool_definitions(tools);
    true
}

fn apply_agent_plan_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    plan_mode: bool,
) -> Vec<ChatToolDefinition> {
    if !plan_mode {
        return Vec::new();
    }
    let mut blocked = Vec::new();
    tools.retain(|tool| {
        let allowed = agent_plan_allows_tool(tool);
        if !allowed {
            blocked.push(tool.clone());
        }
        allowed
    });
    blocked
}

fn agent_plan_allows_tool(tool: &ChatToolDefinition) -> bool {
    if tool.source == "native" && crate::chat::ask_user::is_ask_user_tool_name(&tool.name) {
        return true;
    }
    if tool.source == "native" && crate::chat::todo::is_agent_todo_tool_name(&tool.name) {
        return true;
    }
    if tool.source == "native" {
        return tool.is_read_only_tool();
    }
    if tool.source == "mcp" {
        return tool.is_read_only_tool();
    }
    tool.source == "skill" && tool.name == "skill"
}

fn apply_inline_code_request_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    last_user_api_content: Option<&str>,
) {
    if !should_answer_inline_without_file_write(last_user_api_content) {
        return;
    }
    tools.retain(|tool| !(tool.source == "native" && tool.name == "write"));
}

fn should_answer_inline_without_file_write(last_user_api_content: Option<&str>) -> bool {
    let Some(content) = last_user_api_content else {
        return false;
    };
    let user_text = content
        .split("[已添加附件]")
        .next()
        .unwrap_or(content)
        .trim();
    if user_text.is_empty() {
        return false;
    }
    let normalized = user_text.to_ascii_lowercase();
    if has_explicit_file_write_intent(user_text, &normalized) {
        return false;
    }
    has_inline_code_request_intent(user_text, &normalized)
}

fn has_explicit_file_write_intent(text: &str, normalized: &str) -> bool {
    const ZH_MARKERS: &[&str] = &[
        "保存",
        "写入",
        "写到",
        "写进",
        "输出到",
        "导出",
        "创建文件",
        "生成文件",
        "另存为",
        "存成",
        "落盘",
    ];
    const EN_MARKERS: &[&str] = &[
        "save",
        "create file",
        "output file",
        "output to",
        "export",
        "save as",
        "write to",
        "file named",
    ];
    ZH_MARKERS.iter().any(|marker| text.contains(marker))
        || EN_MARKERS.iter().any(|marker| normalized.contains(marker))
}

fn has_inline_code_request_intent(text: &str, normalized: &str) -> bool {
    const ZH_MARKERS: &[&str] = &["```", "代码块", "代码框", "围栏代码"];
    const EN_MARKERS: &[&str] = &["```", "code block", "fenced code"];
    ZH_MARKERS.iter().any(|marker| text.contains(marker))
        || EN_MARKERS.iter().any(|marker| normalized.contains(marker))
}

// 历史拼装的唯一入口：send 与 regenerate 都最终走这里。
/// 失败/无视觉模型时逐级降级，始终返回一个可读的文本结果。
pub(crate) async fn read_image_as_tool_result(
    app: &AppHandle,
    settings: &Settings,
    conversation_id: &str,
    message_id: &str,
    path: &Path,
) -> Result<mcp::types::McpToolCallResult, String> {
    super::vision::read_image_as_tool_result(app, settings, conversation_id, message_id, path)
        .await
}

/// R1：MCP 工具结果里的图片 artifact「直达模型」。通用于所有 MCP server（非
/// officecli 专属），复用 `read_image_as_tool_result` 已验证的两级策略：
/// ① 主模型支持视觉 → 把图片作为 follow-up user 消息直喂（`data_url_image_part`，
/// 不落盘）；② 纯文本主模型 → 落临时文件 `kivio-mcpimg-<uuid>.<ext>` 走辅助视觉
/// 模型做审查向分析（R2），把分析文字追加进 tool 结果的 content，随后删除临时
/// 文件。全程尽力而为：拿不到会话上下文、无可用视觉模型、分析失败等任何一步
/// 出错都原样保留 `[image: <mime>]` 占位符，不影响 MCP 工具调用本身的成败。
/// 仅对当前这一轮工具结果生效，不回填历史轮（调用方每轮都会重新执行）。
pub(crate) async fn attach_image_artifacts_for_model(
    app: &AppHandle,
    settings: &Settings,
    conversation_id: &str,
    message_id: &str,
    result: &mut mcp::types::McpToolCallResult,
) {
    super::vision::attach_image_artifacts_for_model(
        app,
        settings,
        conversation_id,
        message_id,
        result,
    )
    .await;
}

struct ChatAgentHost<'a> {
    app: AppHandle,
    state: &'a AppState,
    /// 多模型臂置 true：抑制 mid-run 部分快照落盘（协调者统一落盘）。默认 false（现状）。
    suppress_partial_persist: bool,
}

impl crate::chat::agent::AgentHost for ChatAgentHost<'_> {
    fn emit_stream_delta(
        &self,
        conversation_id: &str,
        run_id: &str,
        message_id: &str,
        delta: &str,
        reasoning_delta: Option<&str>,
        segment: Option<&ChatMessageSegment>,
    ) {
        emit_chat_stream_delta(
            &self.app,
            conversation_id,
            run_id,
            message_id,
            delta,
            reasoning_delta,
            segment,
        );
    }

    fn emit_stream_done(
        &self,
        conversation_id: &str,
        run_id: &str,
        message_id: &str,
        reason: &str,
        full: &str,
    ) {
        emit_chat_stream_done(&self.app, conversation_id, run_id, message_id, reason, full);
    }

    fn emit_tool_record(
        &self,
        conversation_id: &str,
        run_id: &str,
        message_id: &str,
        record: &ToolCallRecord,
    ) {
        emit_chat_tool_record(&self.app, conversation_id, run_id, message_id, record);
    }

    fn emit_compaction_status(
        &self,
        conversation_id: &str,
        phase: &str,
        trigger: Option<&str>,
        boundary: Option<&CompactionBoundaryRecord>,
    ) {
        emit_chat_compaction_state(&self.app, conversation_id, phase, trigger, boundary);
    }

    fn persist_partial_assistant(
        &self,
        conversation_id: &str,
        message_id: &str,
        tool_records: &[ToolCallRecord],
        segments: &[ChatMessageSegment],
        api_messages: &[Value],
    ) {
        if self.suppress_partial_persist {
            // 多模型臂不直接写盘（避免 N 条并发 run 同写 conversations/{id}.json）。
            return;
        }
        if let Err(err) = persist_partial_assistant_snapshot(
            &self.app,
            conversation_id,
            message_id,
            tool_records,
            segments,
            api_messages,
        ) {
            eprintln!("persist partial assistant snapshot failed: {err}");
        }
    }

    fn request_tool_approval<'a>(
        &'a self,
        ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
        record: &'a ToolCallRecord,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async move {
            request_tool_approval(
                &self.app,
                self.state,
                ctx.conversation_id,
                ctx.run_id,
                ctx.message_id,
                ctx.generation,
                record,
            )
            .await
        })
    }

    fn request_session_consent<'a>(
        &'a self,
        ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async move {
            request_session_consent(
                &self.app,
                self.state,
                ctx.tool_conversation_id,
                ctx.run_id,
                ctx.message_id,
                ctx.generation,
            )
            .await
        })
    }

    fn request_user_response<'a>(
        &'a self,
        ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
        record: &'a ToolCallRecord,
        prompt: crate::chat::ask_user::AskUserPromptPayload,
    ) -> crate::chat::agent::AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult> {
        Box::pin(async move {
            request_user_response(
                &self.app,
                self.state,
                ctx.conversation_id,
                ctx.run_id,
                ctx.message_id,
                ctx.generation,
                record,
                prompt,
            )
            .await
        })
    }

    fn is_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
        self.state
            .is_chat_generation_active(conversation_id, generation)
    }

    fn wait_for_generation_inactive<'a>(
        &'a self,
        conversation_id: &'a str,
        generation: u64,
    ) -> crate::chat::agent::AgentHostFuture<'a, ()> {
        Box::pin(async move {
            wait_for_chat_cancel(self.state, conversation_id, generation).await;
        })
    }
}

/// 无头测试通道（probe）的 AgentHost，仅 debug 构建。跑的是与 GUI 完全相同的生成核心
/// （`complete_assistant_reply_inner`），但所有需要 GUI 应答的交互门一律自动放行：审批 /
/// 会话 consent → 允许，`ask_user` → 取消态（不阻塞）。事件发射 no-op（结果从落盘的 assistant
/// 消息内联读取，不靠事件）。generation 相关沿用标准机制，保证超时/取消能生效。
#[cfg(debug_assertions)]
struct ProbeAgentHost<'a> {
    state: &'a AppState,
}

#[cfg(debug_assertions)]
impl crate::chat::agent::AgentHost for ProbeAgentHost<'_> {
    fn emit_stream_delta(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        _delta: &str,
        _reasoning_delta: Option<&str>,
        _segment: Option<&ChatMessageSegment>,
    ) {
    }

    fn emit_stream_done(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        _reason: &str,
        _full: &str,
    ) {
    }

    fn emit_tool_record(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        _record: &ToolCallRecord,
    ) {
    }

    fn request_tool_approval<'a>(
        &'a self,
        _ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
        _record: &'a ToolCallRecord,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async { true })
    }

    fn request_session_consent<'a>(
        &'a self,
        _ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async { true })
    }

    fn request_user_response<'a>(
        &'a self,
        _ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
        _record: &'a ToolCallRecord,
        _prompt: crate::chat::ask_user::AskUserPromptPayload,
    ) -> crate::chat::agent::AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult> {
        // 无头：不能向用户提问，直接返回取消态让 loop 继续（不阻塞）。
        Box::pin(async { crate::chat::ask_user::cancelled_response() })
    }

    fn is_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
        self.state
            .is_chat_generation_active(conversation_id, generation)
    }

    fn wait_for_generation_inactive<'a>(
        &'a self,
        conversation_id: &'a str,
        generation: u64,
    ) -> crate::chat::agent::AgentHostFuture<'a, ()> {
        Box::pin(async move {
            wait_for_chat_cancel(self.state, conversation_id, generation).await;
        })
    }
}

/// 无头测试通道的一次生成编排（仅 debug）：把 scratch 会话绑到一个**固定复用**的
/// 「Chat Probe」项目（根为请求的 cwd，使文件工具相对路径可解析）→ 推入 user 消息 →
/// 走与 GUI 完全相同的生成核心（`complete_assistant_reply_inner`，probe=true 自动放行）→
/// 取回生成的 assistant 消息。**会话与项目都保留**（不删除），以便在会话列表里观察调试。
/// 返回 assistant 消息（含 content + tool_calls + stream_outcome + usage）。
#[cfg(debug_assertions)]
pub(crate) async fn run_chat_probe(
    app: &AppHandle,
    state: &State<'_, AppState>,
    prompt: String,
    provider: Option<String>,
    model: Option<String>,
    skill_id: Option<String>,
    mode: Option<String>,
    cwd: Option<String>,
) -> Result<ChatMessage, String> {
    const PROBE_PROJECT_ID: &str = "proj_kivio_probe";
    // cwd → 固定复用的「Chat Probe」项目：根设为 cwd，使文件工具（read/glob/grep）相对路径
    // 从此解析（非项目会话是 global workspace 无根，与真实 GUI 一致）。复用同一项目避免污染
    // 列表；不删除，方便在会话列表里点开观察每次 probe 的完整轨迹。
    let project_id = if let Some(cwd) = cwd.as_deref().filter(|c| !c.trim().is_empty()) {
        let now = chrono::Local::now().timestamp();
        let exists = get_projects(app)?
            .into_iter()
            .any(|p| p.id == PROBE_PROJECT_ID);
        if exists {
            // 更新根到本次 cwd（其余字段不动）。
            let _ = update_project(
                app,
                PROBE_PROJECT_ID,
                None,
                None,
                false,
                None,
                false,
                Some(cwd.to_string()),
                true,
            );
        } else {
            create_project(
                app,
                crate::chat::types::ChatProject {
                    id: PROBE_PROJECT_ID.to_string(),
                    name: "Chat Probe".to_string(),
                    description: Some("无头测试通道（debug）的会话都在这里，可点开观察".to_string()),
                    color: None,
                    root_path: Some(cwd.to_string()),
                    created_at: now,
                    updated_at: now,
                },
            )?;
        }
        Some(PROBE_PROJECT_ID.to_string())
    } else {
        None
    };

    let mut conversation = create_chat_conversation_internal(
        app,
        state.inner(),
        provider,
        model,
        None,
        project_id,
        None,
        None,
    )?;
    // 会话标题取自 prompt（截断），便于在列表里识别。
    conversation.title = {
        let head: String = prompt.chars().take(60).collect();
        format!("🔬 {head}")
    };
    // 可选运行模式（act/plan/orchestrate）：验证模式提示词用。非法值报错而非静默回落。
    if let Some(mode) = mode.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        let mode = crate::chat::plan::mode_from_str(mode)?;
        conversation.agent_plan_state =
            crate::chat::plan::with_mode(&conversation.agent_plan_state, mode);
    }
    let user_message = ChatMessage {
        id: format!("msg_{}", Uuid::new_v4()),
        role: "user".to_string(),
        content: prompt.clone(),
        attachments: Vec::new(),
        reasoning: None,
        artifacts: Vec::new(),
        tool_calls: Vec::new(),
        segments: Vec::new(),
        agent_plan: None,
        api_messages: Vec::new(),
        model_messages: Vec::new(),
        active_skill_id: None,
        run_entry: None,
        stream_outcome: None,
        usage: None,
        anchor_usage: None,
        group_id: None,
        provider_id: None,
        model: None,
        timestamp: chrono::Local::now().timestamp(),
    };
    conversation.messages.push(user_message);
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(app, &conversation)?;

    let gen_result = complete_assistant_reply_inner(
        app,
        state,
        &mut conversation,
        None,
        Some(prompt.as_str()),
        &[],
        skill_id.as_deref(),
        crate::chat::agent::AgentRunEntry::Send,
        None,
        /* probe */ true,
    )
    .await;

    // 拿到最后一条 assistant 消息（complete_assistant_reply_inner 已 push+save 到会话）。
    // 会话与项目都保留在列表里，供观察调试——不删除。
    let assistant = conversation
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .cloned();

    gen_result?;
    assistant.ok_or_else(|| "probe: no assistant message produced".to_string())
}

struct RegistryToolExecutor<'a> {
    app: AppHandle,
    state: &'a AppState,
}
impl crate::chat::agent::ToolExecutor for RegistryToolExecutor<'_> {
    fn call<'a>(
        &'a self,
        ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
        skill_cache: Option<&'a mut skills::SkillRunCache>,
    ) -> crate::chat::agent::ToolExecutorFuture<'a> {
        Box::pin(async move {
            let native_ctx = mcp::registry::NativeToolContext {
                // Conversation-scoped tools (todo / native workspace) target the
                // tool conversation, which equals the run conversation for a
                // top-level run and the PARENT conversation for a sub-agent run.
                conversation_id: ctx.tool_conversation_id.to_string(),
                message_id: ctx.message_id.to_string(),
                tool_call_id: Some(ctx.tool_call_id.to_string()),
                run_id: ctx.run_id.to_string(),
                generation: ctx.generation,
                depth: ctx.depth,
            };
            mcp::registry::call_tool(
                &self.app,
                self.state,
                tool,
                arguments,
                skill_cache,
                Some(native_ctx),
            )
            .await
        })
    }
}

fn sanitize_api_message_for_model(message: &Value) -> Value {
    let mut sanitized = message.clone();
    if let Some(content) = sanitized.get_mut("content") {
        sanitize_api_content_for_model(content);
    }
    sanitized
}

fn sanitize_api_content_for_model(content: &mut Value) {
    match content {
        Value::String(text) => {
            *text = sanitize_image_payloads_for_model(text);
        }
        Value::Array(parts) => {
            for part in parts {
                if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                    let sanitized = sanitize_image_payloads_for_model(text);
                    if let Some(text_value) = part.get_mut("text") {
                        *text_value = Value::String(sanitized);
                    }
                }
            }
        }
        _ => {}
    }
}

fn sanitize_image_payloads_for_model(content: &str) -> String {
    let without_data_urls = strip_image_data_urls_for_model(content);
    without_data_urls
        .lines()
        .map(|line| {
            if looks_like_inline_image_base64(line.trim()) {
                "[image base64 omitted; image is available as a tool artifact]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_image_data_urls_for_model(content: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("data:image/") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let Some(base64_marker) = after_start.find(";base64,") else {
            output.push_str("data:image/");
            rest = &after_start["data:image/".len()..];
            continue;
        };
        let payload_start = start + base64_marker + ";base64,".len();
        let mut payload_end = payload_start;
        for (offset, ch) in rest[payload_start..].char_indices() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=') {
                payload_end = payload_start + offset + ch.len_utf8();
            } else {
                break;
            }
        }
        output.push_str("[image data URL omitted; image is available as a tool artifact]");
        rest = &rest[payload_end..];
    }
    output.push_str(rest);
    output
}

fn looks_like_inline_image_base64(value: &str) -> bool {
    if value.len() < 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return false;
    }
    value.starts_with("iVBORw0KGgo")
        || value.starts_with("/9j/")
        || value.starts_with("R0lGOD")
        || value.starts_with("UklGR")
        || value.starts_with("PHN2Zy")
        || value.starts_with("PD94bWwg")
}

async fn request_session_consent(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
) -> bool {
    // Already granted for this conversation — no prompt.
    if state.has_chat_consent(conversation_id) {
        return true;
    }
    // Serialize prompts so concurrent first-round tools (read/grep/find/ls run
    // in parallel) don't each insert a pending sender and clobber one another.
    // Whoever wins the lock prompts once; the rest re-check consent and reuse
    // the grant without a second dialog.
    let _prompt_guard = state.chat_consent_prompt_lock.lock().await;
    if state.has_chat_consent(conversation_id) {
        return true;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut pending = state
            .pending_chat_session_consents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Only one outstanding consent prompt per conversation.
        pending.insert(conversation_id.to_string(), tx);
    }
    let _ = app.emit(
        "chat-session-consent",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
        }),
    );
    let result = tokio::select! {
        result = timeout(Duration::from_secs(60), rx) => result,
        _ = wait_for_chat_cancel(state, conversation_id, generation) => {
            state
                .pending_chat_session_consents
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(conversation_id);
            return false;
        }
    };
    match result {
        Ok(Ok(true)) => {
            state.grant_chat_consent(conversation_id);
            true
        }
        _ => {
            state
                .pending_chat_session_consents
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(conversation_id);
            false
        }
    }
}

async fn request_tool_approval(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    record: &ToolCallRecord,
) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut pending = state
            .pending_chat_tool_approvals
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.insert(record.id.clone(), tx);
    }
    let _ = app.emit(
        "chat-tool-confirm",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "toolCallId": record.id,
            "name": record.name,
            "source": record.source,
            "serverId": record.server_id,
            "argumentsPreview": format_tool_approval_summary(record),
            "sensitivity": "sensitive",
        }),
    );
    let result = tokio::select! {
        result = timeout(Duration::from_secs(60), rx) => result,
        _ = wait_for_chat_cancel(state, conversation_id, generation) => {
            let mut pending = state
                .pending_chat_tool_approvals
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            return false;
        }
    };
    match result {
        Ok(Ok(value)) => value,
        _ => {
            let mut pending = state
                .pending_chat_tool_approvals
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            false
        }
    }
}

async fn request_user_response(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    record: &ToolCallRecord,
    prompt: crate::chat::ask_user::AskUserPromptPayload,
) -> crate::chat::ask_user::AskUserResponseResult {
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut pending = state
            .pending_chat_user_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.insert(
            record.id.clone(),
            crate::chat::ask_user::PendingAskUserPrompt {
                prompt: prompt.clone(),
                sender: tx,
            },
        );
    }

    let empty_answers = HashMap::new();
    let structured_content = crate::chat::ask_user::structured_content(
        &prompt,
        crate::chat::ask_user::ASK_USER_PHASE_AWAITING,
        &empty_answers,
    );
    let _ = app.emit(
        "chat-user-prompt",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "toolCallId": record.id,
            "id": record.id,
            "name": record.name,
            "source": record.source,
            "prompt": prompt,
            "structuredContent": structured_content,
        }),
    );

    let result = tokio::select! {
        result = timeout(Duration::from_secs(600), rx) => result,
        _ = wait_for_chat_cancel(state, conversation_id, generation) => {
            let mut pending = state
                .pending_chat_user_prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            return crate::chat::ask_user::cancelled_response();
        }
    };
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => {
            let mut pending = state
                .pending_chat_user_prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            crate::chat::ask_user::cancelled_response()
        }
        Err(_) => {
            let mut pending = state
                .pending_chat_user_prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            crate::chat::ask_user::timeout_response()
        }
    }
}

async fn wait_for_chat_cancel(state: &AppState, conversation_id: &str, generation: u64) {
    while state.is_chat_generation_active(conversation_id, generation) {
        sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) fn emit_chat_tool_record(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    record: &ToolCallRecord,
) {
    let _ = app.emit(
        "chat-tool",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "toolCallId": record.id,
            "id": record.id,
            "name": record.name,
            "source": record.source,
            "serverId": record.server_id,
            "status": record.status,
            "argumentsPreview": truncate_chars(&record.arguments, 800),
            "resultPreview": record.result_preview,
            "error": record.error,
            "startedAt": record.started_at,
            "completedAt": record.completed_at,
            "durationMs": record.duration_ms,
            "round": record.round,
            "sensitive": record.sensitive,
            "artifacts": record.artifacts,
            "traceId": record.trace_id,
            "spanId": record.span_id,
            "structuredContent": record.structured_content,
        }),
    );
}

pub(crate) fn emit_chat_stream_delta(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    delta: &str,
    reasoning_delta: Option<&str>,
    segment: Option<&ChatMessageSegment>,
) {
    let _ = app.emit(
        "chat-stream",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "imageId": "",
            "kind": "answer",
            "delta": delta,
            "reasoningDelta": reasoning_delta,
            "segmentId": segment.map(|segment| segment.id.as_str()),
            "segmentKind": segment.map(|segment| &segment.kind),
            "phase": segment.map(|segment| &segment.phase),
            "order": segment.map(|segment| segment.order),
            "stepNumber": segment.and_then(|segment| segment.step_number),
            "round": segment.and_then(|segment| segment.round),
            "toolCallId": segment.and_then(|segment| segment.tool_call_id.as_deref()),
            "segment": segment,
        }),
    );
}

pub(crate) fn emit_chat_stream_done(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    reason: &str,
    full: &str,
) {
    let _ = app.emit(
        "chat-stream",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "imageId": "",
            "kind": "answer",
            "delta": "",
            "done": true,
            "reason": reason,
            "full": full,
        }),
    );
}

use crate::chat::agent::execute::truncate_chars;





fn format_tool_approval_summary(record: &ToolCallRecord) -> String {
    let parsed = serde_json::from_str::<Value>(&record.arguments).ok();
    let mut lines = Vec::new();
    match record.name.as_str() {
        "bash" => {
            if let Some(command) = parsed
                .as_ref()
                .and_then(|value| value.get("command"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Command: {command}"));
            }
            if let Some(cwd) = parsed
                .as_ref()
                .and_then(|value| value.get("cwd"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Working directory: {cwd}"));
            }
        }
        "write" | "edit" | "read" => {
            if let Some(path) = parsed
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Path: {path}"));
            }
            if record.name == "edit" {
                // Current shape: edits: [{old_string, new_string}, ...]. Preview the
                // first edit's old_string; fall back to the legacy single-edit field.
                let first_old = parsed
                    .as_ref()
                    .and_then(|value| value.get("edits"))
                    .and_then(|value| value.as_array())
                    .and_then(|edits| edits.first())
                    .and_then(|edit| edit.get("old_string"))
                    .and_then(|value| value.as_str())
                    .or_else(|| {
                        parsed
                            .as_ref()
                            .and_then(|value| value.get("old_string").or_else(|| value.get("old")))
                            .and_then(|value| value.as_str())
                    })
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(old) = first_old {
                    lines.push(format!("Replace: {}", truncate_chars(old, 180)));
                }
            }
        }
        _ => {}
    }

    if lines.is_empty() {
        truncate_chars(&record.arguments, 800)
    } else {
        let mut summary = lines.join("\n");
        summary.push_str("\n\nRaw arguments:\n");
        summary.push_str(&truncate_chars(&record.arguments, 800));
        summary
    }
}

fn find_message_index(conversation: &Conversation, message_id: &str) -> Result<usize, String> {
    conversation
        .messages
        .iter()
        .position(|m| m.id == message_id)
        .ok_or_else(|| "消息不存在".to_string())
}

/// 更新单条消息（仅助手回复）
#[tauri::command]
pub(crate) async fn chat_update_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
    content: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("消息内容不能为空".to_string());
    }

    let idx = find_message_index(&conversation, &message_id)?;
    if conversation.messages[idx].role != "assistant" {
        return Err("仅支持编辑助手回复".to_string());
    }

    mark_summary_stale_if_needed(&mut conversation, idx);
    replace_final_text_segments_for_edit(&mut conversation.messages[idx], trimmed);
    conversation.messages[idx].timestamp = chrono::Local::now().timestamp();
    let context_state = compute_context_state(&app, &state, &conversation, None, &[]).await?;
    conversation.context_state = context_state.clone();
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_context_state(&app, &conversation.id, &context_state);

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// `chat_regenerate_message` 的截断/编辑核心（纯函数，便于单测）：
/// - assistant：截到它之前（`new_content` 无意义 → 报错）。
/// - user + `new_content`：trim 校验非空 → 替换内容（编辑提问；附件保留）→ 保留该条截掉其后。
///   摘要失效用 `idx`（内容变了，覆盖到该条的摘要即失效）。
/// - user 无 `new_content`：孤儿重试，摘要失效用 `idx + 1`，保留该条截掉其后。
fn apply_regenerate_truncation(
    conversation: &mut Conversation,
    idx: usize,
    new_content: Option<String>,
) -> Result<(), String> {
    match conversation.messages[idx].role.as_str() {
        "assistant" => {
            if new_content.is_some() {
                return Err("编辑内容仅支持用户消息".to_string());
            }
            mark_summary_stale_if_needed(conversation, idx);
            conversation.messages.truncate(idx);
        }
        "user" => {
            if let Some(content) = new_content {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Err("消息内容不能为空".to_string());
                }
                mark_summary_stale_if_needed(conversation, idx);
                conversation.messages[idx].content = trimmed.to_string();
                conversation.messages[idx].timestamp = chrono::Local::now().timestamp();
            } else {
                mark_summary_stale_if_needed(conversation, idx + 1);
            }
            conversation.messages.truncate(idx + 1);
        }
        _ => return Err("仅支持重新生成助手回复或重试用户消息".to_string()),
    }
    Ok(())
}

/// 重新生成助手回复（移除该条及之后的消息，再基于此前上下文请求新回复）。
/// `new_content`：编辑用户提问并重新生成——仅当目标是 user 消息时有效，先替换其内容
/// 再走截断+重生成（附件保留；一个原子命令，避免"改了历史但不重生成"的不一致状态）。
#[tauri::command]
pub(crate) async fn chat_regenerate_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
    new_content: Option<String>,
) -> Result<serde_json::Value, String> {
    // Busy 拒绝：该会话仍有任意一条 run 在跑时不允许再触发重新生成。
    // 原子哨兵预留关闭 TOCTOU 窗口；per-run 槽位 / generation 在 `complete_assistant_reply` 内注册。
    let Some(_send_reservation) = ChatSendReservation::try_acquire(state.inner(), &conversation_id)
    else {
        return Ok(serde_json::json!({
            "success": false,
            "error": CHAT_REPLY_BUSY_ERROR,
        }));
    };

    let mut conversation = load_conversation(&app, &conversation_id)?;
    let idx = find_message_index(&conversation, &message_id)?;
    apply_regenerate_truncation(&mut conversation, idx, new_content)?;
    if conversation.messages.last().map(|m| m.role.as_str()) != Some("user") {
        return Err("缺少对应的用户消息，无法重新生成".to_string());
    }

    // 多答组（任务 06-30 / D5 / AC4）：truncate 可能删掉某组的显式「选中条」（或整组），
    // 留下指向已删消息的 group_selections，会让 group_answer_excluded_from_context 把残余
    // 答案全排除出上下文。清掉任何指向已不存在消息的选中记录，回退到「组内第一条」默认。
    if !conversation.group_selections.is_empty() {
        let existing_ids: std::collections::HashSet<&str> =
            conversation.messages.iter().map(|m| m.id.as_str()).collect();
        conversation
            .group_selections
            .retain(|_, msg_id| existing_ids.contains(msg_id.as_str()));
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

    let last_user_api_content = conversation
        .messages
        .last()
        .filter(|message| message.role == "user")
        .map(|message| {
            let attachment_dir = if message.attachments.is_empty() {
                None
            } else {
                conversation_attachments_dir(&app, &conversation_id).ok()
            };
            compose_user_content_for_api(
                &message.content,
                &message.attachments,
                attachment_dir.as_deref(),
            )
        });
    let last_user_image_paths = conversation
        .messages
        .last()
        .filter(|message| message.role == "user")
        .map(|message| {
            stored_image_paths_for_attachments(&app, &conversation_id, &message.attachments)
        })
        .transpose()?
        .unwrap_or_default();
    match compute_context_state(
        &app,
        &state,
        &conversation,
        last_user_api_content.as_deref(),
        &last_user_image_paths,
    )
    .await
    {
        Ok(context_state) => {
            conversation.context_state = context_state.clone();
            save_conversation(&app, &conversation)?;
            emit_chat_context_state(&app, &conversation.id, &context_state);
        }
        Err(err) => eprintln!("Context usage estimate failed before regenerate: {err}"),
    }
    let reply_outcome = complete_assistant_reply(
        &app,
        &state,
        &mut conversation,
        None,
        last_user_api_content.as_deref(),
        &last_user_image_paths,
        None,
        crate::chat::agent::AgentRunEntry::Regenerate,
    )
    .await;
    strip_transcripts_for_frontend(&mut conversation);
    match reply_outcome {
        Ok(()) => Ok(serde_json::json!({
            "success": true,
            "conversation": conversation,
        })),
        Err(err) if err == "cancelled" => Ok(serde_json::json!({
            "success": true,
            "conversation": conversation,
        })),
        Err(err) => Ok(serde_json::json!({
            "success": false,
            "error": err,
        })),
    }
}

/// 删除单条消息
#[tauri::command]
pub(crate) async fn chat_delete_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let idx = find_message_index(&conversation, &message_id)?;
    if conversation.messages[idx].role != "assistant" {
        return Err("仅支持删除助手回复".to_string());
    }

    mark_summary_stale_if_needed(&mut conversation, idx);
    let removed = conversation.messages.remove(idx);
    // 多答组（任务 06-30 / D5 / AC4）：删除某条答案时，若它正是某组的显式「选中条」，
    // 清掉该 group 的 group_selections 记录，让选中条回退到「该组顺序第一条」。否则
    // group_selections 会指向已删除的 message_id，导致 group_answer_excluded_from_context
    // 把整组答案都排除出下一轮上下文（无任何答案进历史）。
    if let Some(group_id) = removed.group_id.as_deref() {
        if conversation.group_selections.get(group_id).map(String::as_str) == Some(removed.id.as_str()) {
            conversation.group_selections.remove(group_id);
        }
    }
    let context_state = compute_context_state(&app, &state, &conversation, None, &[]).await?;
    conversation.context_state = context_state.clone();
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_context_state(&app, &conversation.id, &context_state);

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 组装分叉分支的消息前缀（纯函数，便于单测）：
/// - 取 `messages[0..=anchor_idx]`（含锚点）。
/// - 若锚点属于某多模型多答组（`group_id = Some(g)`，决策 Q2）：只保留锚点那一列，
///   移除前缀内其余同组兄弟列（组内更晚的列已被切片排除），并把锚点的 `group_id` 置 None
///   转为普通单答，使新对话该轮成为干净的线性单答。
/// - 保留原 message id（跨对话无需唯一，`group_selections` 引用因此仍有效）。
fn build_fork_messages(messages: &[ChatMessage], anchor_idx: usize) -> Vec<ChatMessage> {
    let anchor_id = messages[anchor_idx].id.clone();
    let anchor_group = messages[anchor_idx].group_id.clone();
    let mut out: Vec<ChatMessage> = messages[..=anchor_idx].to_vec();

    if let Some(group) = anchor_group {
        // 丢弃同组的其它兄弟列，仅留锚点那条。
        out.retain(|m| m.group_id.as_deref() != Some(group.as_str()) || m.id == anchor_id);
        // 锚点转普通单答。
        if let Some(anchor) = out.iter_mut().find(|m| m.id == anchor_id) {
            anchor.group_id = None;
        }
    }

    out
}

/// 分叉成新对话（方案 B）：把源对话某消息及其之前的消息复制进一个全新对话，之后独立继续。
/// 纯复制 + 打开，不自动发送（决策 Q1）。新对话继承源的会话级配置、深拷被引用的附件/图片
/// artifact，并记录 `forked_from` 供 UI 面包屑回跳。源对话完全只读、不受影响。
#[tauri::command]
pub(crate) fn chat_fork_conversation(
    app: AppHandle,
    conversation_id: String,
    message_id: String,
) -> Result<serde_json::Value, String> {
    let source = load_conversation(&app, &conversation_id)?;
    let anchor_idx = find_message_index(&source, &message_id)?;

    let now = chrono::Local::now().timestamp();
    let new_id = format!("conv_{}", Uuid::new_v4());
    let messages = build_fork_messages(&source.messages, anchor_idx);

    // 深拷被复制消息引用的附件 / 图片 artifact 文件到新对话目录（裸文件名同名拷贝，路径保持有效）。
    // 缺文件容错跳过（记 warning，不阻断分叉）。沙箱导出的生成文件（~/Kivio/outputs/<id>/）不拷——见 design 限制。
    copy_forked_conversation_files(&app, &conversation_id, &new_id, &messages);

    // 清理 group_selections：仅保留其选中 message_id 仍存在于新 messages、且该消息仍带 group_id 的条目。
    let existing_groups: std::collections::HashMap<&str, &str> = messages
        .iter()
        .filter_map(|m| m.group_id.as_deref().map(|g| (m.id.as_str(), g)))
        .collect();
    let mut group_selections = source.group_selections.clone();
    group_selections.retain(|group_id, sel_msg_id| {
        existing_groups.get(sel_msg_id.as_str()) == Some(&group_id.as_str())
    });

    // 标题加「（分支）」后缀（R7）。先把源标题截到留出后缀空间，保证后缀在 40 字上限内始终可见。
    const FORK_SUFFIX: &str = "（分支）";
    let base = truncate_chars(&source.title, 40 - FORK_SUFFIX.chars().count());
    let title = format!("{base}{FORK_SUFFIX}");

    let conversation = Conversation {
        id: new_id,
        title,
        provider_id: source.provider_id.clone(),
        model: source.model.clone(),
        messages,
        agent_runtime: source.agent_runtime.clone(),
        active_skill_id: source.active_skill_id.clone(),
        assistant_id: source.assistant_id.clone(),
        assistant_snapshot: source.assistant_snapshot.clone(),
        created_at: now,
        updated_at: now,
        pinned: false,
        folder: source.folder.clone(),
        project_id: source.project_id.clone(),
        set_id: source.set_id.clone(),
        context_state: ConversationContextState::default(),
        agent_todo_state: AgentTodoState::default(),
        agent_plan_state: AgentPlanState::default(),
        knowledge_base_ids: source.knowledge_base_ids.clone(),
        force_knowledge_search: source.force_knowledge_search,
        thinking_level: source.thinking_level.clone(),
        reply_models: source.reply_models.clone(),
        group_selections,
        forked_from: Some(ForkOrigin {
            conversation_id: source.id.clone(),
            message_id: source.messages[anchor_idx].id.clone(),
            title: source.title.clone(),
        }),
    };

    save_conversation(&app, &conversation)?;

    let mut conversation = conversation;
    // 与 chat_get_conversation 一致：返回前做读时孤立工具分段对账（必须在 strip 之前——
    // strip 会清掉已完成消息的 api_messages，而回捞工具名依赖它）。源存量文件若有孤立分段，
    // 分叉后的即时展示也能正常显示工具卡（tool-segment-record-reconcile 契约）。
    reconcile_conversation_orphan_tool_segments(&mut conversation);
    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 深拷分叉消息引用的对话目录文件（附件 + 图片 artifact）到新对话附件目录。
/// path 为裸文件名（相对源对话附件目录），同名拷贝到新目录后引用保持有效。
fn copy_forked_conversation_files(
    app: &AppHandle,
    source_id: &str,
    new_id: &str,
    messages: &[ChatMessage],
) {
    // 收集所有被引用的裸文件名（附件 + 消息级 artifact + 各 tool_call 内 artifact）。
    let mut names: Vec<&str> = Vec::new();
    for message in messages {
        for att in &message.attachments {
            if !att.path.is_empty() {
                names.push(att.path.as_str());
            }
        }
        for artifact in &message.artifacts {
            if let Some(p) = artifact.path.as_deref().filter(|p| !p.is_empty()) {
                names.push(p);
            }
        }
        for tool_call in &message.tool_calls {
            for artifact in &tool_call.artifacts {
                if let Some(p) = artifact.path.as_deref().filter(|p| !p.is_empty()) {
                    names.push(p);
                }
            }
        }
    }
    if names.is_empty() {
        return;
    }
    names.sort_unstable();
    names.dedup();

    let (Ok(src_dir), Ok(dst_dir)) = (
        conversation_attachments_dir(app, source_id),
        conversation_attachments_dir(app, new_id),
    ) else {
        eprintln!("fork: 无法解析附件目录，跳过附件深拷（source={source_id} new={new_id}）");
        return;
    };

    for name in names {
        // 只接受裸文件名，拒绝任何路径分隔符（防越目录）。
        if name.contains('/') || name.contains('\\') {
            eprintln!("fork: 跳过非法附件路径 {name}");
            continue;
        }
        let src = src_dir.join(name);
        let dst = dst_dir.join(name);
        if !src.is_file() {
            eprintln!("fork: 源附件不存在，跳过 {name}");
            continue;
        }
        if let Err(e) = std::fs::copy(&src, &dst) {
            eprintln!("fork: 拷贝附件失败 {name}: {e}");
        }
    }
}

/// 删除对话
#[tauri::command]
pub(crate) fn chat_delete_conversation(
    app: AppHandle,
    state: tauri::State<crate::state::AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    // 删对话即终止其持久外部 CLI 会话（actor 关闭子进程）并清掉跨重启 resume 句柄。
    state.remove_external_live_session(&conversation_id);
    crate::external_agents::session::clear_live_handle(&app, &conversation_id);
    // 顺手清掉该对话在内存里按 conversation_id 累积的运行态小 map（stream 代际计数 /
    // 会话级工具同意），它们只插不删、严格无界——对话删了便永远不会再被引用。
    state.forget_chat_conversation_runtime(&conversation_id);
    delete_conv(&app, &conversation_id)?;
    Ok(serde_json::json!({
        "success": true,
    }))
}

/// 更新对话（标题、置顶、文件夹等）
#[tauri::command]
pub(crate) fn chat_update_conversation(
    app: AppHandle,
    conversation_id: String,
    title: Option<String>,
    pinned: Option<bool>,
    folder: Option<String>,
    project_id: Option<String>,
    set_id: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    active_skill_id: Option<String>,
    assistant_id: Option<String>,
    knowledge_base_ids: Option<Vec<String>>,
    force_knowledge_search: Option<bool>,
    thinking_level: Option<String>,
    reply_models: Option<Vec<crate::chat::ModelRef>>,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;

    if let Some(t) = title {
        conversation.title = t;
    }
    if let Some(p) = pinned {
        conversation.pinned = p;
    }
    if let Some(folder) = folder {
        let trimmed = folder.trim();
        conversation.folder = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        conversation.project_id = match conversation.folder.as_deref() {
            Some(folder) => find_project_by_name(&app, folder)?.map(|project| project.id),
            None => None,
        };
    }
    if let Some(project_id) = project_id {
        let trimmed = project_id.trim();
        if trimmed.is_empty() {
            conversation.project_id = None;
            conversation.folder = None;
        } else {
            let project = find_project_by_id(&app, trimmed)?;
            conversation.project_id = Some(project.id);
            conversation.folder = Some(project.name);
            conversation.set_id = None; // 集与项目互斥
        }
    }
    if let Some(set_id) = set_id {
        let trimmed = set_id.trim();
        if trimmed.is_empty() {
            conversation.set_id = None;
        } else {
            let set = find_set_by_id(&app, trimmed)?;
            conversation.set_id = Some(set.id);
            // 集与项目互斥：归入集即移出项目/文件夹
            conversation.project_id = None;
            conversation.folder = None;
        }
    }
    if let Some(provider_id) = provider_id {
        conversation.provider_id = provider_id;
    }
    if let Some(model) = model {
        conversation.model = model;
    }
    if let Some(skill_id) = active_skill_id {
        let trimmed = skill_id.trim();
        conversation.active_skill_id = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(assistant_id) = assistant_id {
        let trimmed = assistant_id.trim();
        if trimmed.is_empty() {
            conversation.assistant_id = None;
            conversation.assistant_snapshot = None;
            conversation.active_skill_id = None;
        } else {
            let snapshot = assistant_snapshot(&app, trimmed)?;
            // 切换助手不再强制激活默认技能;skill_ids 仅作白名单。
            conversation.active_skill_id = None;
            conversation.assistant_id = Some(snapshot.id.clone());
            conversation.assistant_snapshot = Some(snapshot);
        }
    }
    if let Some(ids) = knowledge_base_ids {
        // Drop blanks/dups; order preserved.
        let mut seen = std::collections::HashSet::new();
        conversation.knowledge_base_ids = ids
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && seen.insert(s.clone()))
            .collect();
    }
    if let Some(force) = force_knowledge_search {
        conversation.force_knowledge_search = force;
    }
    if let Some(level) = thinking_level {
        // 仅接受已知值；空串/未知 → 清除（回到「跟随全局」）。
        conversation.thinking_level = match level.trim() {
            "off" | "low" | "medium" | "high" | "xhigh" | "max" => Some(level.trim().to_string()),
            _ => None,
        };
    }
    if let Some(reply_models) = reply_models {
        // 多模型一问多答（决策 D2/D4）：持久化会话级多答模型集。去重（provider+model）、
        // 丢空、保序、上限 MAX_REPLY_MODELS（超出报错，前端应已禁选）。
        if reply_models.len() > MAX_REPLY_MODELS {
            return Err(format!(
                "多模型并行回答最多同时选择 {MAX_REPLY_MODELS} 个模型。"
            ));
        }
        let mut seen = std::collections::HashSet::new();
        conversation.reply_models = reply_models
            .into_iter()
            .filter_map(|m| {
                let provider_id = m.provider_id.trim().to_string();
                let model = m.model.trim().to_string();
                if provider_id.is_empty() || model.is_empty() {
                    return None;
                }
                let key = format!("{provider_id}\u{0}{model}");
                if seen.insert(key) {
                    Some(crate::chat::ModelRef { provider_id, model })
                } else {
                    None
                }
            })
            .collect();
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 设置某个多答组（task 06-30）的「选中条」（决策 D5）：用户点选某一列后续聊以它为准。
/// `message_id` 必须是属于 `group_id` 这组的某条 assistant 消息；写入
/// `conversation.group_selections[group_id] = message_id`，下一轮历史拼装据此只保留该条。
#[tauri::command]
pub(crate) fn chat_set_group_selection(
    app: AppHandle,
    conversation_id: String,
    group_id: String,
    message_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let group_id = group_id.trim();
    let message_id = message_id.trim();
    if group_id.is_empty() || message_id.is_empty() {
        return Err("group_id 与 message_id 不能为空".to_string());
    }
    // 校验：该消息必须存在、是 assistant、且属于这个 group。
    let valid = conversation.messages.iter().any(|m| {
        m.id == message_id
            && m.role == "assistant"
            && m.group_id.as_deref() == Some(group_id)
    });
    if !valid {
        return Err("选中的回答不属于该多答组".to_string());
    }
    conversation
        .group_selections
        .insert(group_id.to_string(), message_id.to_string());
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 生成对话标题（本地兜底截断）
fn generate_title(content: &str) -> String {
    let trimmed = content.trim();
    let title = trimmed.chars().take(30).collect::<String>();
    if trimmed.chars().count() > 30 {
        format!("{title}...")
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::Attachment;
    use crate::chat::ModelRef;
    use std::collections::HashMap;

    #[test]
    fn resolve_thinking_maps_levels_and_defaults_to_high() {
        // 未设置 → 默认档 high，不再跟随全局（全局只服务 lens / 翻译）。
        assert_eq!(resolve_thinking(None, true), (true, Some("high".to_string())));
        assert_eq!(resolve_thinking(None, false), (true, Some("high".to_string())));
        // off → 强制关。
        assert_eq!(resolve_thinking(Some("off"), true), (false, None));
        // 具体等级 → 开 + 带等级。
        assert_eq!(
            resolve_thinking(Some("low"), false),
            (true, Some("low".to_string()))
        );
        assert_eq!(
            resolve_thinking(Some("high"), false),
            (true, Some("high".to_string()))
        );
        // xhigh / max 也放行（是否被模型接受由前端按模型门控）。
        assert_eq!(
            resolve_thinking(Some("xhigh"), false),
            (true, Some("xhigh".to_string()))
        );
        assert_eq!(
            resolve_thinking(Some("max"), false),
            (true, Some("max".to_string()))
        );
        // 未知值 → 当作未设置，落默认档 high。
        assert_eq!(resolve_thinking(Some("ultra"), true), (true, Some("high".to_string())));
    }

    #[test]
    fn builder_args_produce_valid_assistant() {
        let args = serde_json::json!({
            "name": "  写作助手 ",
            "system_prompt": "你是写作助手。",
            "description": "写文案",
            "mcp_server_ids": ["mcp-1", "  ", "mcp-2"],
            "skill_ids": ["doc"]
        });
        let a = assistant_from_builder_args(&args).expect("should parse");
        assert!(a.id.starts_with("asst_"));
        assert_eq!(a.name, "写作助手");
        assert_eq!(a.system_prompt, "你是写作助手。");
        assert_eq!(a.source, "user");
        assert!(!a.built_in);
        assert_eq!(a.mcp_server_ids, vec!["mcp-1", "mcp-2"]); // 空串被过滤
        assert_eq!(a.skill_ids, vec!["doc"]);
    }

    #[test]
    fn builder_args_reject_missing_required() {
        assert!(assistant_from_builder_args(&serde_json::json!({ "system_prompt": "x" })).is_err());
        assert!(assistant_from_builder_args(&serde_json::json!({ "name": "x" })).is_err());
        assert!(
            assistant_from_builder_args(&serde_json::json!({ "name": "x", "system_prompt": "  " }))
                .is_err()
        );
    }
    fn slash_skill_record(id: &str, name: &str, triggers: Vec<&str>) -> skills::SkillRecord {
        skills::SkillRecord {
            meta: skills::SkillMeta {
                id: id.to_string(),
                name: name.to_string(),
                description: "desc".to_string(),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation: false,
                files: vec![],
                triggers: triggers.into_iter().map(str::to_string).collect(),
                argument_hint: Some("<message>".to_string()),
                arguments: vec!["message".to_string()],
            },
            location: std::path::PathBuf::from(format!("/skills/{id}/SKILL.md")),
            base_dir: std::path::PathBuf::from(format!("/skills/{id}")),
            body: "Write a commit for: $ARGUMENTS (subject $MESSAGE)".to_string(),
            allowed_tools: vec![],
        }
    }

    fn slash_skill_registry(record: skills::SkillRecord) -> skills::SkillRegistry {
        skills::SkillRegistry {
            records: vec![record],
            warnings: vec![],
        }
    }

    #[test]
    fn slash_trigger_rewrites_body_and_pins_skill() {
        let registry = slash_skill_registry(slash_skill_record("commit", "Commit", vec!["/commit"]));
        let chat_tools = crate::settings::ChatToolsConfig::default();

        let (skill_id, rewritten) =
            try_apply_skill_slash_trigger(&registry, &chat_tools, None, "/commit fix login", &[], false)
                .expect("slash trigger should match");

        assert_eq!(skill_id, "commit");
        assert!(rewritten.starts_with("[Skill: Commit]\n\n"));
        assert!(rewritten.contains("Write a commit for: fix login"));
        // first positional arg ($MESSAGE) → "fix"
        assert!(rewritten.contains("subject fix"));
    }

    #[test]
    fn slash_trigger_ignores_non_slash_and_unknown() {
        let registry = slash_skill_registry(slash_skill_record("commit", "Commit", vec!["/commit"]));
        let chat_tools = crate::settings::ChatToolsConfig::default();

        assert!(try_apply_skill_slash_trigger(&registry, &chat_tools, None, "commit fix", &[], false).is_none());
        assert!(try_apply_skill_slash_trigger(&registry, &chat_tools, None, "/unknown x", &[], false).is_none());
    }

    #[test]
    fn slash_trigger_skips_disabled_skill() {
        let registry = slash_skill_registry(slash_skill_record("commit", "Commit", vec!["/commit"]));
        let mut chat_tools = crate::settings::ChatToolsConfig::default();
        chat_tools.disabled_skill_ids = vec!["commit".to_string()];

        assert!(try_apply_skill_slash_trigger(&registry, &chat_tools, None, "/commit fix", &[], false).is_none());
    }

    fn test_provider(id: &str, name: &str, enabled_models: Vec<&str>) -> ModelProvider {
        ModelProvider {
            id: id.to_string(),
            name: name.to_string(),
            api_keys: vec!["sk-test".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: Vec::new(),
            enabled_models: enabled_models.into_iter().map(str::to_string).collect(),
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: HashMap::new(),
            compress_request_body: false,
        }
    }

    #[test]
    fn inline_code_request_filter_removes_file_creation_tools_for_fenced_code() {
        let mut tools = vec![
            crate::mcp::types::native_read_file_tool(),
            crate::mcp::types::native_write_file_tool(),
            crate::mcp::types::native_edit_file_tool(),
        ];

        apply_inline_code_request_tool_filter(
            &mut tools,
            Some("生成一个完整的 HTML demo，用 ```html 代码块包起来。"),
        );

        assert!(tools.iter().any(|tool| tool.name == "read"));
        assert!(!tools.iter().any(|tool| tool.name == "write"));
        assert!(tools.iter().any(|tool| tool.name == "edit"));
    }

    #[test]
    fn inline_code_request_filter_does_not_hide_file_tools_for_generic_demo_words() {
        let mut tools = vec![
            crate::mcp::types::native_read_file_tool(),
            crate::mcp::types::native_write_file_tool(),
        ];

        apply_inline_code_request_tool_filter(&mut tools, Some("生成一个完整的 HTML demo"));

        assert!(tools.iter().any(|tool| tool.name == "write"));
    }

    #[test]
    fn inline_code_request_filter_treats_put_into_code_block_as_inline() {
        let mut tools = vec![
            crate::mcp::types::native_read_file_tool(),
            crate::mcp::types::native_write_file_tool(),
        ];

        apply_inline_code_request_tool_filter(&mut tools, Some("把完整 HTML 放到代码块里给我"));

        assert!(!tools.iter().any(|tool| tool.name == "write"));
    }

    #[test]
    fn inline_code_request_filter_keeps_write_tools_for_save_intent() {
        let mut tools = vec![
            crate::mcp::types::native_read_file_tool(),
            crate::mcp::types::native_write_file_tool(),
            crate::mcp::types::native_edit_file_tool(),
        ];

        apply_inline_code_request_tool_filter(
            &mut tools,
            Some("生成一个完整的 HTML demo，保存为 ~/news-demo.html。"),
        );

        assert!(tools.iter().any(|tool| tool.name == "write"));
        assert!(tools.iter().any(|tool| tool.name == "edit"));
    }

    #[test]
    fn agent_plan_tool_filter_keeps_only_read_only_and_agent_state_tools() {
        let readonly_mcp_tool = ChatToolDefinition {
            id: "mcp__docs__search".to_string(),
            name: "search".to_string(),
            description: "Search docs".to_string(),
            source: "mcp".to_string(),
            server_id: Some("docs".to_string()),
            server_name: Some("Docs".to_string()),
            input_schema: serde_json::json!({"type": "object"}),
            sensitive: false,
            annotations: Some(serde_json::json!({ "readOnlyHint": true })),
            output_schema: None,
        };
        let write_mcp_tool = ChatToolDefinition {
            id: "mcp__fs__write".to_string(),
            name: "write".to_string(),
            description: "Write file".to_string(),
            source: "mcp".to_string(),
            server_id: Some("fs".to_string()),
            server_name: Some("FS".to_string()),
            input_schema: serde_json::json!({"type": "object"}),
            sensitive: true,
            annotations: Some(serde_json::json!({ "readOnlyHint": false })),
            output_schema: None,
        };
        let mut tools = vec![
            crate::mcp::types::native_read_file_tool(),
            crate::mcp::types::native_write_file_tool(),
            crate::mcp::types::native_run_command_tool(),
            crate::mcp::types::native_run_python_tool(),
            crate::mcp::types::native_memory_read_tool(),
            crate::mcp::types::native_memory_modify_tool(),
            crate::mcp::types::mixer_generate_image_tool(),
            crate::mcp::types::native_skill_activate_tool(),
            crate::chat::ask_user::ask_user_tool(),
            crate::chat::todo::todo_write_tool(),
            readonly_mcp_tool,
            write_mcp_tool,
        ];

        let blocked = apply_agent_plan_tool_filter(&mut tools, true);

        let names = tools
            .iter()
            .map(|tool| tool.openai_tool_name())
            .collect::<Vec<_>>();
        let blocked_names = blocked
            .iter()
            .map(|tool| tool.openai_tool_name())
            .collect::<Vec<_>>();
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"memory_read".to_string()));
        assert!(names.contains(&"skill".to_string()));
        assert!(names.contains(&"ask_user".to_string()));
        assert!(names.contains(&"todo_write".to_string()));
        assert!(names.contains(&"mcp__docs__search".to_string()));
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"bash".to_string()));
        assert!(!names.contains(&"run_python".to_string()));
        assert!(!names.contains(&"memory_modify".to_string()));
        assert!(!names.contains(&"mixer_generate_image".to_string()));
        assert!(!names.contains(&"mcp__fs__write".to_string()));
        assert!(blocked_names.contains(&"write".to_string()));
        assert!(blocked_names.contains(&"bash".to_string()));
        assert!(blocked_names.contains(&"run_python".to_string()));
        assert!(blocked_names.contains(&"memory_modify".to_string()));
        assert!(blocked_names.contains(&"mixer_generate_image".to_string()));
        assert!(blocked_names.contains(&"mcp__fs__write".to_string()));
    }

    #[test]
    fn agent_plan_tool_filter_is_noop_outside_plan_mode() {
        let mut tools = vec![
            crate::mcp::types::native_read_file_tool(),
            crate::mcp::types::native_write_file_tool(),
            crate::mcp::types::native_run_command_tool(),
        ];

        let blocked = apply_agent_plan_tool_filter(&mut tools, false);

        assert!(tools.iter().any(|tool| tool.name == "read"));
        assert!(tools.iter().any(|tool| tool.name == "write"));
        assert!(tools.iter().any(|tool| tool.name == "bash"));
        assert!(blocked.is_empty());
    }

    #[test]
    fn orchestrate_budget_bump_raises_rounds_but_keeps_unlimited() {
        use crate::settings::ORCHESTRATE_MIN_TOOL_ROUNDS;
        let bump = |configured: Option<u32>| {
            configured.map(|rounds| rounds.max(ORCHESTRATE_MIN_TOOL_ROUNDS))
        };
        // Configured below the floor -> raised to the floor.
        assert_eq!(bump(Some(20)), Some(ORCHESTRATE_MIN_TOOL_ROUNDS));
        // Configured above the floor -> preserved.
        assert_eq!(bump(Some(80)), Some(80));
        // Unlimited (None) stays unlimited.
        assert_eq!(bump(None), None);
    }

    #[test]
    fn inline_code_request_ignores_attachment_safe_copy_paths() {
        let content = compose_user_content_for_api(
            "用 ```html 包起来给我",
            &[Attachment {
                id: "att_1".to_string(),
                attachment_type: "file".to_string(),
                name: "report.pdf".to_string(),
                path: "att_1-report.pdf".to_string(),
            }],
            Some(Path::new("/Users/test/Library/Application Support/com.zmair.kivio/conversations/conv_1_attachments")),
        );

        assert!(should_answer_inline_without_file_write(Some(&content)));
    }

    #[test]
    fn generate_title_truncates_unicode_safely() {
        let title = generate_title("附件: 这是一张非常非常非常非常非常非常非常长的图片文件名.png");

        assert!(title.ends_with("..."));
        assert!(title.chars().count() <= 33);
    }

    #[test]
    fn agent_run_entry_label_distinguishes_regenerate() {
        assert_eq!(
            agent_run_entry_label(crate::chat::agent::AgentRunEntry::Send),
            "send"
        );
        assert_eq!(
            agent_run_entry_label(crate::chat::agent::AgentRunEntry::Regenerate),
            "regenerate"
        );
    }

    #[test]
    fn build_title_summary_prompt_uses_first_turn_context() {
        let prompt = build_title_summary_prompt(
            "今天下雨吗，吉林市。天气怎么样？",
            "吉林市今天有小雨，建议带伞。",
            "zh-CN",
        );

        assert!(prompt.contains("首轮对话"));
        assert!(prompt.contains("用户：今天下雨吗"));
        assert!(prompt.contains("助手：吉林市今天有小雨"));
        assert!(prompt.contains("只输出标题本身"));
    }

    #[test]
    fn sanitize_generated_title_removes_model_formatting() {
        assert_eq!(
            sanitize_generated_title("- 标题：\"吉林天气查询。\""),
            Some("吉林天气查询".to_string())
        );
        assert_eq!(
            sanitize_generated_title("Title: `Jilin Weather Forecast.`"),
            Some("Jilin Weather Forecast".to_string())
        );
    }

    #[test]
    fn sanitize_generated_title_rejects_empty_output() {
        assert_eq!(sanitize_generated_title("\n\n  "), None);
        assert_eq!(sanitize_generated_title("标题：..."), None);
    }

    #[test]
    fn format_tool_approval_summary_highlights_run_command() {
        let record = ToolCallRecord {
            id: "call_1".to_string(),
            name: "bash".to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: r#"{"command":"npm test","cwd":"/tmp/project"}"#.to_string(),
            status: ToolCallStatus::Pending,
            result_preview: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 1,
            sensitive: true,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        };

        let summary = format_tool_approval_summary(&record);
        assert!(summary.contains("Command: npm test"));
        assert!(summary.contains("Working directory: /tmp/project"));
        assert!(summary.contains("Raw arguments"));
    }

    #[test]
    fn format_tool_approval_summary_highlights_file_path() {
        let record = ToolCallRecord {
            id: "call_1".to_string(),
            name: "write".to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: r#"{"path":"/tmp/project/out.txt","content":"hello"}"#.to_string(),
            status: ToolCallStatus::Pending,
            result_preview: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 1,
            sensitive: true,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        };

        let summary = format_tool_approval_summary(&record);
        assert!(summary.contains("Path: /tmp/project/out.txt"));
        assert!(summary.contains("Raw arguments"));
    }

    #[test]
    fn assistant_model_messages_marks_failed_tool_results_as_error() {
        let api_messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_error",
                    "type": "function",
                    "function": {
                        "name": "run_python",
                        "arguments": "{\"code\":\"print(1/0)\"}"
                    }
                }]
            }),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "call_error",
                "content": "Python 执行失败：ZeroDivisionError: division by zero"
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "ZeroDivisionError"
            }),
        ];
        let tool_calls = vec![ToolCallRecord {
            id: "call_error".to_string(),
            name: "run_python".to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{\"code\":\"print(1/0)\"}".to_string(),
            status: ToolCallStatus::Error,
            result_preview: None,
            error: Some("Python 执行失败：ZeroDivisionError: division by zero".to_string()),
            duration_ms: Some(31),
            started_at: Some(1),
            completed_at: Some(2),
            round: 1,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        }];

        let model_messages = assistant_model_messages_for_storage(
            "ZeroDivisionError",
            None,
            &api_messages,
            &tool_calls,
        );
        let tool_result_is_error = model_messages
            .iter()
            .flat_map(|message| message.content.iter())
            .find_map(|part| match part {
                MessagePart::ToolResult {
                    tool_call_id,
                    is_error,
                    ..
                } if tool_call_id == "call_error" => Some(*is_error),
                _ => None,
            });

        assert_eq!(tool_result_is_error, Some(true));
    }

    fn test_tool_record(
        id: &str,
        source: &str,
        round: u32,
        status: ToolCallStatus,
    ) -> ToolCallRecord {
        ToolCallRecord {
            id: id.to_string(),
            name: if source == "mixer" {
                "mixer_vision".to_string()
            } else {
                "run_python".to_string()
            },
            source: source.to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status,
            result_preview: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        }
    }

    fn tool_segment(order: u32, tool_call_id: &str, round: u32) -> ChatMessageSegment {
        ChatMessageSegment {
            id: format!("seg_{order}_tool_{tool_call_id}"),
            kind: ChatMessageSegmentKind::Tool,
            phase: ChatMessageSegmentPhase::ToolLoop,
            order,
            step_number: Some(1),
            round: Some(round),
            text: None,
            tool_call_id: Some(tool_call_id.to_string()),
        }
    }

    #[test]
    fn reconcile_orphan_tool_segments_synthesizes_cancelled_record_with_recovered_meta() {
        let mut tool_calls = vec![test_tool_record("call_ok", "native", 1, ToolCallStatus::Success)];
        let segments = vec![
            tool_segment(1, "call_ok", 1),
            tool_segment(2, "fc_call_function_4agzr50pp9go_1", 2),
        ];
        let api_messages = vec![serde_json::json!({
            "role": "assistant",
            "tool_calls": [{
                "id": "fc_call_function_4agzr50pp9go_1",
                "type": "function",
                "function": { "name": "run_python", "arguments": "{\"code\":\"1\"}" }
            }]
        })];

        reconcile_orphan_tool_segments(&mut tool_calls, &segments, &api_messages);

        assert_eq!(tool_calls.len(), 2, "orphan segment should get a synthesized record");
        let synthesized = tool_calls
            .iter()
            .find(|r| r.id == "fc_call_function_4agzr50pp9go_1")
            .expect("synthesized record present");
        assert!(matches!(synthesized.status, ToolCallStatus::Cancelled));
        assert_eq!(synthesized.name, "run_python", "name recovered from api_messages");
        assert_eq!(synthesized.arguments, "{\"code\":\"1\"}");
        assert_eq!(synthesized.round, 2);
        assert!(synthesized.error.is_some());
    }

    #[test]
    fn reconcile_orphan_tool_segments_falls_back_to_empty_name_without_api_meta() {
        let mut tool_calls: Vec<ToolCallRecord> = Vec::new();
        let segments = vec![tool_segment(1, "orphan_no_meta", 1)];

        reconcile_orphan_tool_segments(&mut tool_calls, &segments, &[]);

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "orphan_no_meta");
        assert!(tool_calls[0].name.is_empty(), "no api meta → empty name fallback");
        assert!(matches!(tool_calls[0].status, ToolCallStatus::Cancelled));
    }

    #[test]
    fn reconcile_orphan_tool_segments_noop_when_all_segments_have_records() {
        let mut tool_calls = vec![test_tool_record("call_ok", "native", 1, ToolCallStatus::Success)];
        let segments = vec![tool_segment(1, "call_ok", 1)];

        reconcile_orphan_tool_segments(&mut tool_calls, &segments, &[]);

        assert_eq!(tool_calls.len(), 1, "no orphan → tool_calls unchanged");
    }

    #[test]
    fn old_assistant_message_without_segments_deserializes() {
        let message: ChatMessage = serde_json::from_value(serde_json::json!({
            "id": "msg_legacy",
            "role": "assistant",
            "content": "legacy answer",
            "timestamp": 42
        }))
        .expect("legacy message should deserialize");

        assert_eq!(message.content, "legacy answer");
        assert!(message.segments.is_empty());
        assert!(message.tool_calls.is_empty());
    }

    #[test]
    fn segment_legacy_fields_join_only_their_owned_segment_kinds() {
        let segments = vec![
            ChatMessageSegment {
                id: "seg_tool_loop_text".to_string(),
                kind: ChatMessageSegmentKind::Text,
                phase: ChatMessageSegmentPhase::ToolLoop,
                order: 20,
                step_number: Some(1),
                round: Some(1),
                text: Some("planning text".to_string()),
                tool_call_id: None,
            },
            ChatMessageSegment {
                id: "seg_plain".to_string(),
                kind: ChatMessageSegmentKind::Text,
                phase: ChatMessageSegmentPhase::Plain,
                order: 10,
                step_number: None,
                round: None,
                text: Some("plain answer".to_string()),
                tool_call_id: None,
            },
            ChatMessageSegment {
                id: "seg_reasoning".to_string(),
                kind: ChatMessageSegmentKind::Reasoning,
                phase: ChatMessageSegmentPhase::ToolLoop,
                order: 30,
                step_number: Some(1),
                round: Some(1),
                text: Some("reasoning block".to_string()),
                tool_call_id: None,
            },
            ChatMessageSegment {
                id: "seg_synthesis".to_string(),
                kind: ChatMessageSegmentKind::Text,
                phase: ChatMessageSegmentPhase::Synthesis,
                order: 40,
                step_number: Some(2),
                round: None,
                text: Some("final answer".to_string()),
                tool_call_id: None,
            },
        ];

        assert_eq!(
            content_from_segments(&segments).as_deref(),
            Some("plain answer\n\nfinal answer")
        );
        assert_eq!(
            reasoning_from_segments(&segments).as_deref(),
            Some("reasoning block")
        );
    }

    #[test]
    fn normalize_segments_adds_auxiliary_and_skipped_tool_segments() {
        let tool_calls = vec![
            test_tool_record("call_aux", "mixer", 0, ToolCallStatus::Success),
            test_tool_record("call_blocked", "native", 1, ToolCallStatus::Skipped),
        ];
        let segments = normalize_assistant_segments(
            "final",
            None,
            &tool_calls,
            vec![ChatMessageSegment {
                id: "seg_final".to_string(),
                kind: ChatMessageSegmentKind::Text,
                phase: ChatMessageSegmentPhase::Synthesis,
                order: 1000,
                step_number: Some(2),
                round: None,
                text: Some("final".to_string()),
                tool_call_id: None,
            }],
        );

        let auxiliary = segments
            .iter()
            .find(|segment| segment.tool_call_id.as_deref() == Some("call_aux"))
            .expect("auxiliary tool should have a segment");
        let skipped = segments
            .iter()
            .find(|segment| segment.tool_call_id.as_deref() == Some("call_blocked"))
            .expect("skipped tool should have a segment");

        assert_eq!(auxiliary.kind, ChatMessageSegmentKind::Tool);
        assert_eq!(auxiliary.phase, ChatMessageSegmentPhase::Auxiliary);
        assert_eq!(skipped.kind, ChatMessageSegmentKind::Tool);
        assert_eq!(skipped.phase, ChatMessageSegmentPhase::ToolLoop);
    }

    #[test]
    fn normalize_segments_inserts_tool_segments_before_synthesis_text() {
        let tool_calls = vec![test_tool_record(
            "call_read",
            "external_cli",
            1,
            ToolCallStatus::Success,
        )];
        let segments = normalize_assistant_segments(
            "final answer",
            Some("reasoning"),
            &tool_calls,
            vec![
                ChatMessageSegment {
                    id: "seg_reasoning".to_string(),
                    kind: ChatMessageSegmentKind::Reasoning,
                    phase: ChatMessageSegmentPhase::Plain,
                    order: 1,
                    step_number: None,
                    round: None,
                    text: Some("reasoning".to_string()),
                    tool_call_id: None,
                },
                ChatMessageSegment {
                    id: "seg_before".to_string(),
                    kind: ChatMessageSegmentKind::Text,
                    phase: ChatMessageSegmentPhase::ToolLoop,
                    order: 2,
                    step_number: None,
                    round: Some(1),
                    text: Some("working".to_string()),
                    tool_call_id: None,
                },
                ChatMessageSegment {
                    id: "seg_final".to_string(),
                    kind: ChatMessageSegmentKind::Text,
                    phase: ChatMessageSegmentPhase::Synthesis,
                    order: 3,
                    step_number: None,
                    round: None,
                    text: Some("final answer".to_string()),
                    tool_call_id: None,
                },
            ],
        );

        let tool_segment = segments
            .iter()
            .find(|segment| segment.tool_call_id.as_deref() == Some("call_read"))
            .expect("tool segment should exist");
        let final_segment = segments
            .iter()
            .find(|segment| segment.id == "seg_final")
            .expect("final segment should exist");
        assert_eq!(tool_segment.kind, ChatMessageSegmentKind::Tool);
        assert!(tool_segment.order < final_segment.order);
    }

    #[test]
    fn editing_assistant_reply_replaces_final_text_segments_only() {
        let tool_call = test_tool_record("call_blocked", "native", 1, ToolCallStatus::Skipped);
        let mut message = ChatMessage {
            id: "msg_assistant".to_string(),
            role: "assistant".to_string(),
            content: "old final".to_string(),
            attachments: Vec::new(),
            reasoning: Some("reasoning block".to_string()),
            artifacts: Vec::new(),
            tool_calls: vec![tool_call],
            segments: vec![
                ChatMessageSegment {
                    id: "seg_plan".to_string(),
                    kind: ChatMessageSegmentKind::Text,
                    phase: ChatMessageSegmentPhase::ToolLoop,
                    order: 1000,
                    step_number: Some(1),
                    round: Some(1),
                    text: Some("planning text".to_string()),
                    tool_call_id: None,
                },
                ChatMessageSegment {
                    id: "seg_tool".to_string(),
                    kind: ChatMessageSegmentKind::Tool,
                    phase: ChatMessageSegmentPhase::ToolLoop,
                    order: 1001,
                    step_number: Some(1),
                    round: Some(1),
                    text: None,
                    tool_call_id: Some("call_blocked".to_string()),
                },
                ChatMessageSegment {
                    id: "seg_reasoning".to_string(),
                    kind: ChatMessageSegmentKind::Reasoning,
                    phase: ChatMessageSegmentPhase::ToolLoop,
                    order: 1002,
                    step_number: Some(1),
                    round: Some(1),
                    text: Some("reasoning block".to_string()),
                    tool_call_id: None,
                },
                ChatMessageSegment {
                    id: "seg_old".to_string(),
                    kind: ChatMessageSegmentKind::Text,
                    phase: ChatMessageSegmentPhase::Synthesis,
                    order: 1003,
                    step_number: Some(2),
                    round: None,
                    text: Some("old final".to_string()),
                    tool_call_id: None,
                },
            ],
            agent_plan: None,
            api_messages: Vec::new(),
            model_messages: Vec::new(),
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            anchor_usage: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 1,
        };

        replace_final_text_segments_for_edit(&mut message, "new final");

        assert_eq!(message.content, "new final");
        assert_eq!(message.reasoning.as_deref(), Some("reasoning block"));
        assert!(message.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Tool
                && segment.tool_call_id.as_deref() == Some("call_blocked")
        }));
        assert!(message.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::ToolLoop
                && segment.text.as_deref() == Some("planning text")
        }));
        assert!(!message.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && matches!(
                    segment.phase,
                    ChatMessageSegmentPhase::Plain | ChatMessageSegmentPhase::Synthesis
                )
                && segment.text.as_deref() == Some("old final")
        }));
        assert!(message.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::Synthesis
                && segment.text.as_deref() == Some("new final")
        }));
    }

    #[test]
    fn editing_assistant_reply_rewrites_replay_to_edited_final_answer() {
        let mut message = ChatMessage {
            id: "msg_assistant".to_string(),
            role: "assistant".to_string(),
            content: "old final".to_string(),
            attachments: Vec::new(),
            reasoning: Some("old visible reasoning".to_string()),
            artifacts: Vec::new(),
            tool_calls: vec![test_tool_record(
                "call_1",
                "native",
                1,
                ToolCallStatus::Success,
            )],
            segments: vec![
                ChatMessageSegment {
                    id: "seg_reasoning".to_string(),
                    kind: ChatMessageSegmentKind::Reasoning,
                    phase: ChatMessageSegmentPhase::Synthesis,
                    order: 999,
                    step_number: Some(2),
                    round: None,
                    text: Some("old visible reasoning".to_string()),
                    tool_call_id: None,
                },
                ChatMessageSegment {
                    id: "seg_old".to_string(),
                    kind: ChatMessageSegmentKind::Text,
                    phase: ChatMessageSegmentPhase::Synthesis,
                    order: 1000,
                    step_number: Some(2),
                    round: None,
                    text: Some("old final".to_string()),
                    tool_call_id: None,
                },
            ],
            agent_plan: None,
            api_messages: vec![
                serde_json::json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"/tmp/old.txt\"}"
                        }
                    }]
                }),
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "content": "tool output"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "old final",
                    "reasoning_content": "old final reasoning"
                }),
            ],
            model_messages: Vec::new(),
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            anchor_usage: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 1,
        };

        replace_final_text_segments_for_edit(&mut message, "new final");

        assert!(message.api_messages.is_empty());
        let replay = openai_messages_from_model_messages(&message.model_messages);
        let serialized = serde_json::to_string(&replay).expect("replay serializes");
        assert!(serialized.contains("tool output"));
        assert!(serialized.contains("new final"));
        assert!(serialized.contains("old visible reasoning"));
        assert!(!serialized.contains("old final"));
        assert!(!serialized.contains("old final reasoning"));
    }

    fn test_chat_message(id: &str, role: &str, content: &str, timestamp: i64) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            reasoning: None,
            artifacts: Vec::new(),
            tool_calls: Vec::new(),
            segments: Vec::new(),
            agent_plan: None,
            api_messages: Vec::new(),
            model_messages: Vec::new(),
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            anchor_usage: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp,
        }
    }

    fn test_conversation_with_summary(stale: bool) -> Conversation {
        Conversation {
            id: "conv_test".to_string(),
            title: "test".to_string(),
            provider_id: "provider".to_string(),
            model: "model".to_string(),
            messages: vec![
                test_chat_message("msg_user_1", "user", "old user content", 1),
                test_chat_message("msg_assistant_1", "assistant", "old assistant content", 2),
                test_chat_message("msg_user_2", "user", "recent user content", 3),
                test_chat_message(
                    "msg_assistant_2",
                    "assistant",
                    "recent assistant content",
                    4,
                ),
            ],
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 1,
            updated_at: 4,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState {
                summary: Some(ConversationContextSummary {
                    id: "ctxsum_test".to_string(),
                    content: "summary of older messages".to_string(),
                    source_message_ids: vec![
                        "msg_user_1".to_string(),
                        "msg_assistant_1".to_string(),
                    ],
                    source_until_message_id: "msg_assistant_1".to_string(),
                    token_estimate_before: 100,
                    token_estimate_after: 10,
                    created_at: 5,
                    provider_id: "provider".to_string(),
                    model: "model".to_string(),
                    stale,
                }),
                ..ConversationContextState::default()
            },
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
            thinking_level: None,
            reply_models: Vec::new(),
            group_selections: std::collections::HashMap::new(),
            forked_from: None,
            agent_runtime: crate::chat::AgentRuntimeConfig::default(),
        }
    }

    #[test]
    fn approve_agent_plan_targets_selected_message_plan() {
        let mut conversation = test_conversation_with_summary(false);
        let old_plan = "1. Inspect current code\n2. Draft older fix";
        let new_plan = "1. Inspect plan mode\n2. Implement inline execution";
        let mut older = test_chat_message("msg_plan_old", "assistant", old_plan, 10);
        older.agent_plan = Some(AgentPlanState {
            mode: crate::chat::AgentPlanMode::Plan,
            status: crate::chat::AgentPlanStatus::Draft,
            plan: Some(old_plan.to_string()),
            updated_at: 10,
        });
        let mut newer = test_chat_message("msg_plan_new", "assistant", new_plan, 11);
        newer.agent_plan = Some(AgentPlanState {
            mode: crate::chat::AgentPlanMode::Plan,
            status: crate::chat::AgentPlanStatus::Draft,
            plan: Some(new_plan.to_string()),
            updated_at: 11,
        });
        conversation.agent_plan_state = older.agent_plan.clone().unwrap();
        conversation.messages.push(older);
        conversation.messages.push(newer);

        approve_agent_plan_for_execution(&mut conversation, Some("msg_plan_new")).unwrap();

        assert_eq!(
            conversation.agent_plan_state.plan.as_deref(),
            Some(new_plan)
        );
        assert_eq!(
            conversation.agent_plan_state.status,
            crate::chat::AgentPlanStatus::Approved
        );
        let older = conversation
            .messages
            .iter()
            .find(|message| message.id == "msg_plan_old")
            .unwrap();
        assert_eq!(
            older.agent_plan.as_ref().unwrap().status,
            crate::chat::AgentPlanStatus::Draft
        );
        let newer = conversation
            .messages
            .iter()
            .find(|message| message.id == "msg_plan_new")
            .unwrap();
        assert_eq!(
            newer.agent_plan.as_ref().unwrap().status,
            crate::chat::AgentPlanStatus::Approved
        );
    }

    #[test]
    fn approve_agent_plan_rejects_non_plan_message_target() {
        let mut conversation = test_conversation_with_summary(false);
        conversation
            .messages
            .push(test_chat_message("msg_plain", "assistant", "plain answer", 10));

        let error = approve_agent_plan_for_execution(&mut conversation, Some("msg_plain"))
            .unwrap_err();

        assert_eq!(error, "该消息不是可执行计划");
    }

    #[test]
    fn approve_agent_plan_rejects_empty_message_plan_target() {
        let mut conversation = test_conversation_with_summary(false);
        let mut message = test_chat_message("msg_empty_plan", "assistant", "plain answer", 10);
        message.agent_plan = Some(AgentPlanState {
            mode: crate::chat::AgentPlanMode::Plan,
            status: crate::chat::AgentPlanStatus::Draft,
            plan: Some("   ".to_string()),
            updated_at: 10,
        });
        conversation.messages.push(message);

        let error = approve_agent_plan_for_execution(&mut conversation, Some("msg_empty_plan"))
            .unwrap_err();

        assert_eq!(error, "该消息不是可执行计划");
    }

    #[test]
    fn approve_agent_plan_rejects_non_executable_fragment_target() {
        let mut conversation = test_conversation_with_summary(false);
        let mut message = test_chat_message("msg_fragment_plan", "assistant", "没问题！积萌,", 10);
        message.agent_plan = Some(AgentPlanState {
            mode: crate::chat::AgentPlanMode::Plan,
            status: crate::chat::AgentPlanStatus::Draft,
            plan: Some("没问题！积萌,".to_string()),
            updated_at: 10,
        });
        conversation.messages.push(message);

        let error = approve_agent_plan_for_execution(&mut conversation, Some("msg_fragment_plan"))
            .unwrap_err();

        assert_eq!(error, "该消息不是可执行计划");
    }

    #[test]
    fn strip_transcripts_for_frontend_keeps_interrupted_draft_drops_completed() {
        let mut completed = test_chat_message("msg_done", "assistant", "final answer", 2);
        completed.api_messages = vec![serde_json::json!({
            "role": "assistant",
            "content": "final answer"
        })];
        completed.model_messages =
            vec![ModelMessage::text(ModelRole::Assistant, "final answer")];
        completed.stream_outcome = Some("completed".to_string());

        let mut draft = test_chat_message("msg_draft", "assistant", "partial answer", 4);
        draft.api_messages = vec![serde_json::json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": { "name": "read_file", "arguments": "{}" }
            }]
        })];
        draft.model_messages =
            vec![ModelMessage::text(ModelRole::Assistant, "partial answer")];
        draft.stream_outcome = Some("interrupted".to_string());

        // 旧对话：完成但没有 model_messages，回放需回落 api_messages，DTO 不应剥。
        let mut legacy = test_chat_message("msg_legacy", "assistant", "legacy answer", 6);
        legacy.api_messages = vec![serde_json::json!({
            "role": "assistant",
            "content": "legacy answer"
        })];
        legacy.stream_outcome = Some("completed".to_string());

        let mut user = test_chat_message("msg_user", "user", "hi", 1);
        user.api_messages = vec![serde_json::json!({ "role": "user", "content": "hi" })];

        let mut conversation = test_conversation_with_summary(false);
        conversation.messages = vec![user, completed, draft, legacy];

        strip_transcripts_for_frontend(&mut conversation);

        // 已完成 + 有 model_messages：两份转录都剥光。
        assert!(conversation.messages[1].api_messages.is_empty());
        assert!(conversation.messages[1].model_messages.is_empty());
        // 中断草稿：两份都保住，「继续」要靠它恢复工具上下文。
        assert!(!conversation.messages[2].api_messages.is_empty());
        assert!(!conversation.messages[2].model_messages.is_empty());
        // legacy（无 model_messages）：api_messages 也剥——前端不读，后端回放读盘上完整副本。
        assert!(conversation.messages[3].api_messages.is_empty());
        // user 消息不动。
        assert!(!conversation.messages[0].api_messages.is_empty());
    }

    #[test]
    fn effective_side_models_auto_use_session_main_model() {
        let mut settings = Settings::default();
        settings.providers.push(test_provider(
            "global",
            "Global",
            vec!["gemini-3.1-flash-lite"],
        ));
        settings.providers.push(test_provider("session", "Session", vec!["gpt-4.1"]));
        settings.default_models.chat.provider_id = "global".to_string();
        settings.default_models.chat.model = "gemini-3.1-flash-lite".to_string();

        let session = SessionModel {
            provider_id: "session",
            model: "gpt-4.1",
        };

        assert_eq!(
            settings.effective_compression_model_for_session(Some(session)),
            ("session".to_string(), "gpt-4.1".to_string())
        );
        assert_eq!(
            settings.effective_title_summary_model_for_session(Some(session)),
            ("session".to_string(), "gpt-4.1".to_string())
        );
        assert_eq!(
            settings.effective_vision_model_for_session(Some(session)),
            ("session".to_string(), "gpt-4.1".to_string())
        );
    }

    #[test]
    fn effective_side_models_honor_explicit_mixer_selection() {
        let mut settings = Settings::default();
        settings.providers.push(test_provider(
            "global",
            "Global",
            vec!["gemini-3.1-flash-lite"],
        ));
        settings.providers.push(test_provider(
            "cheap",
            "Cheap",
            vec!["gemini-3.1-flash-lite"],
        ));
        settings.default_models.compression.provider_id = "cheap".to_string();
        settings.default_models.compression.model = "gemini-3.1-flash-lite".to_string();

        let session = SessionModel {
            provider_id: "global",
            model: "gpt-4.1",
        };

        assert_eq!(
            settings.effective_compression_model_for_session(Some(session)),
            (
                "cheap".to_string(),
                "gemini-3.1-flash-lite".to_string()
            )
        );
    }

    #[test]
    fn should_auto_compress_allows_recompression_when_summary_exists() {
        let mut conversation = test_conversation_with_summary(false);
        for i in 0..12 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            let content = format!("extra content {i} ").repeat(1_000);
            conversation.messages.push(test_chat_message(
                &format!("msg_extra_{i}"),
                role,
                &content,
                10 + i,
            ));
        }
        let context_state = ConversationContextState {
            usage_ratio: Some(0.9),
            ..ConversationContextState::default()
        };
        assert!(should_auto_compress_context(&context_state, &conversation));
    }

    #[test]
    fn should_auto_compress_false_when_no_new_compressible_range() {
        let mut conversation = test_conversation_with_summary(false);
        conversation
            .context_state
            .summary
            .as_mut()
            .expect("summary")
            .source_until_message_id = "msg_assistant_2".to_string();
        let context_state = ConversationContextState {
            usage_ratio: Some(0.9),
            ..ConversationContextState::default()
        };
        assert!(!should_auto_compress_context(&context_state, &conversation));
    }

    #[test]
    fn token_split_starts_after_existing_summary() {
        let mut conversation = test_conversation_with_summary(false);
        // summary source_until = msg_assistant_1（index 1）→ summary_start = 2。
        // 推 3 条大消息（每条 ~20000 tokens，ASCII 4 chars/token），recent 尾窗 20000 只够最后 1 条，
        // 其余进 old_segment；boundary 落在倒数第 2 条（index = len-2）。
        for i in 0..3 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            conversation.messages.push(test_chat_message(
                &format!("msg_extra_{i}"),
                role,
                &"a".repeat(80_000),
                10 + i as i64,
            ));
        }
        let summary_start = 2;
        let boundary = crate::chat::agent::compaction::token_split_chat_messages(
            &conversation.messages,
            summary_start,
            crate::chat::agent::compaction::RECENT_KEEP_TOKENS,
        )
        .expect("boundary");
        assert_eq!(boundary, conversation.messages.len() - 2);
        assert!(boundary > summary_start);
    }

    #[test]
    fn token_split_returns_none_when_recent_window_covers_all() {
        // 全是小消息，远不到 20k 尾窗 → 没有可摘要旧段。
        let mut conversation = test_conversation_with_summary(false);
        for i in 0..5 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            conversation.messages.push(test_chat_message(
                &format!("msg_small_{i}"),
                role,
                "x",
                10 + i as i64,
            ));
        }
        let split = crate::chat::agent::compaction::token_split_chat_messages(
            &conversation.messages,
            2,
            crate::chat::agent::compaction::RECENT_KEEP_TOKENS,
        );
        assert!(split.is_none());
    }

    #[test]
    fn build_chat_api_messages_injects_summary_and_skips_old_raw_messages() {
        let conversation = test_conversation_with_summary(false);
        let messages = build_chat_api_messages("system", &conversation, None, None, &[])
            .expect("messages should build");
        let serialized = serde_json::to_string(&messages).expect("messages serialize");

        assert_eq!(messages.len(), 4);
        assert!(serialized.contains("Previous conversation summary"));
        assert!(serialized.contains("summary of older messages"));
        assert!(!serialized.contains("old user content"));
        assert!(!serialized.contains("old assistant content"));
        assert!(serialized.contains("recent user content"));
        assert!(serialized.contains("recent assistant content"));
    }

    #[test]
    fn stale_summary_is_ignored_by_message_builder() {
        let conversation = test_conversation_with_summary(true);
        let messages = build_chat_api_messages("system", &conversation, None, None, &[])
            .expect("messages should build");
        let serialized = serde_json::to_string(&messages).expect("messages serialize");

        assert!(!serialized.contains("Previous conversation summary"));
        assert!(serialized.contains("old user content"));
        assert!(serialized.contains("recent assistant content"));
    }

    #[test]
    fn auxiliary_vision_result_becomes_text_for_main_chat_model() {
        let conversation = Conversation {
            id: "conv_test".to_string(),
            title: "test".to_string(),
            provider_id: "provider".to_string(),
            model: "text-model".to_string(),
            messages: vec![test_chat_message("msg_user_1", "user", "这是什么？", 1)],
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 1,
            updated_at: 1,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState::default(),
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
        thinking_level: None,
            reply_models: Vec::new(),
            group_selections: std::collections::HashMap::new(),
            forked_from: None,
            agent_runtime: crate::chat::AgentRuntimeConfig::default(),
        };
        let result = AuxiliaryVisionResult {
            provider_name: "Vision Provider".to_string(),
            model: "vision-model".to_string(),
            content: "图片里是一张 Kivio 设置页截图。".to_string(),
        };
        let augmented =
            user_content_with_auxiliary_vision_result(Some("这是什么？"), &result, "zh");

        let messages =
            build_chat_api_messages("system", &conversation, Some(0), Some(&augmented), &[])
                .expect("messages should build");
        let content = &messages[1]["content"];

        assert!(content.is_string());
        assert!(content.as_str().unwrap().contains("[混音器视觉副任务结果]"));
        assert!(content.as_str().unwrap().contains("Kivio 设置页截图"));
        assert!(!serde_json::to_string(&messages)
            .expect("messages serialize")
            .contains("image_url"));
    }

    #[test]
    fn mark_summary_stale_if_boundary_or_older_message_changes() {
        let mut after_boundary = test_conversation_with_summary(false);
        mark_summary_stale_if_needed(&mut after_boundary, 2);
        assert_eq!(
            after_boundary
                .context_state
                .summary
                .as_ref()
                .map(|summary| summary.stale),
            Some(false)
        );

        let mut at_boundary = test_conversation_with_summary(false);
        mark_summary_stale_if_needed(&mut at_boundary, 1);
        assert_eq!(
            at_boundary
                .context_state
                .summary
                .as_ref()
                .map(|summary| summary.stale),
            Some(true)
        );
    }

    #[test]
    fn regenerate_truncation_edits_user_content_and_truncates_after() {
        // 编辑 msg_user_2（index 2）：内容替换、其后 assistant 被截、摘要保持未过期
        // （msg_user_2 在摘要 boundary msg_assistant_1 之后，不触发 stale）。
        let mut conversation = test_conversation_with_summary(false);
        apply_regenerate_truncation(&mut conversation, 2, Some("edited question".to_string()))
            .unwrap();
        assert_eq!(conversation.messages.len(), 3);
        assert_eq!(conversation.messages[2].id, "msg_user_2");
        assert_eq!(conversation.messages[2].content, "edited question");
        assert_eq!(
            conversation.context_state.summary.as_ref().map(|s| s.stale),
            Some(false)
        );

        // 编辑被摘要覆盖的 msg_user_1（index 0）：摘要必须标 stale（内容变了摘要即过期）。
        let mut covered = test_conversation_with_summary(false);
        apply_regenerate_truncation(&mut covered, 0, Some("rewritten first question".to_string()))
            .unwrap();
        assert_eq!(covered.messages.len(), 1);
        assert_eq!(covered.messages[0].content, "rewritten first question");
        assert_eq!(
            covered.context_state.summary.as_ref().map(|s| s.stale),
            Some(true)
        );
    }

    #[test]
    fn regenerate_truncation_rejects_bad_edit_targets() {
        // 空内容 → 报错且对话未被改动。
        let mut conversation = test_conversation_with_summary(false);
        let err = apply_regenerate_truncation(&mut conversation, 2, Some("   ".to_string()))
            .unwrap_err();
        assert_eq!(err, "消息内容不能为空");
        assert_eq!(conversation.messages.len(), 4);

        // new_content 指向 assistant → 明确报错（不静默忽略）。
        let err = apply_regenerate_truncation(&mut conversation, 3, Some("nope".to_string()))
            .unwrap_err();
        assert_eq!(err, "编辑内容仅支持用户消息");
        assert_eq!(conversation.messages.len(), 4);

        // 无 new_content 的既有行为不回归：assistant 截到它之前；user 孤儿保留自身。
        let mut plain = test_conversation_with_summary(false);
        apply_regenerate_truncation(&mut plain, 3, None).unwrap();
        assert_eq!(plain.messages.len(), 3);
        assert_eq!(plain.messages.last().unwrap().id, "msg_user_2");
    }

    #[test]
    fn build_chat_api_messages_replays_hidden_tool_transcript() {
        let conversation = Conversation {
            id: "conv_test".to_string(),
            title: "test".to_string(),
            provider_id: "provider".to_string(),
            model: "model".to_string(),
            messages: vec![
                ChatMessage {
                    id: "msg_user_1".to_string(),
                    role: "user".to_string(),
                    content: "use a skill".to_string(),
                    attachments: Vec::new(),
                    reasoning: None,
                    artifacts: Vec::new(),
                    tool_calls: Vec::new(),
                    segments: Vec::new(),
                    agent_plan: None,
                    api_messages: Vec::new(),
                    model_messages: Vec::new(),
                    active_skill_id: None,
                    run_entry: None,
                    stream_outcome: None,
                    usage: None,
                    anchor_usage: None,
                    group_id: None,
                    provider_id: None,
                    model: None,
                    timestamp: 1,
                },
                ChatMessage {
                    id: "msg_assistant_1".to_string(),
                    role: "assistant".to_string(),
                    content: "visible answer".to_string(),
                    attachments: Vec::new(),
                    reasoning: Some("hidden thinking".to_string()),
                    artifacts: Vec::new(),
                    tool_calls: Vec::new(),
                    segments: Vec::new(),
                    agent_plan: None,
                    api_messages: vec![
                        serde_json::json!({
                            "role": "assistant",
                            "content": null,
                            "reasoning_content": "plan",
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "skill_activate",
                                    "arguments": "{\"name\":\"doc\"}"
                                }
                            }]
                        }),
                        serde_json::json!({
                            "role": "tool",
                            "tool_call_id": "call_1",
                            "content": "Skill body"
                        }),
                        serde_json::json!({
                            "role": "assistant",
                            "content": "visible answer",
                            "reasoning_content": "final"
                        }),
                    ],
                    model_messages: Vec::new(),
                    active_skill_id: Some("doc".to_string()),
                    run_entry: None,
                    stream_outcome: None,
                    usage: None,
                    anchor_usage: None,
                    group_id: None,
                    provider_id: None,
                    model: None,
                    timestamp: 2,
                },
            ],
            active_skill_id: Some("doc".to_string()),
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 1,
            updated_at: 2,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState::default(),
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
            thinking_level: None,
            reply_models: Vec::new(),
            group_selections: std::collections::HashMap::new(),
            forked_from: None,
            agent_runtime: crate::chat::AgentRuntimeConfig::default(),
        };

        let messages = build_chat_api_messages("system", &conversation, None, None, &[])
            .expect("messages should build");

        assert_eq!(messages.len(), 5);
        assert_eq!(
            messages[0].get("role").and_then(|value| value.as_str()),
            Some("system")
        );
        assert_eq!(
            messages[1].get("role").and_then(|value| value.as_str()),
            Some("user")
        );
        assert_eq!(
            messages[2]
                .get("tool_calls")
                .and_then(|value| value.as_array())
                .and_then(|calls| calls.first())
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(|value| value.as_str()),
            Some("skill_activate")
        );
        assert_eq!(
            messages[3].get("role").and_then(|value| value.as_str()),
            Some("tool")
        );
        assert_eq!(
            messages[4]
                .get("reasoning_content")
                .and_then(|value| value.as_str()),
            Some("final")
        );
    }

    #[test]
    fn sanitize_image_payloads_replaces_data_urls() {
        let content = "before ![img](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA) after";

        let sanitized = sanitize_image_payloads_for_model(content);

        assert!(
            sanitized.contains("[image data URL omitted; image is available as a tool artifact]")
        );
        assert!(!sanitized.contains("data:image/png;base64"));
        assert!(!sanitized.contains("iVBORw0KGgo"));
    }

    #[test]
    fn sanitize_image_payloads_replaces_raw_base64_lines() {
        let content = concat!(
            "stdout:\n",
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
            "done\n"
        );

        let sanitized = sanitize_image_payloads_for_model(content);

        assert!(sanitized.contains("[image base64 omitted; image is available as a tool artifact]"));
        assert!(!sanitized.contains("iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB"));
        assert!(sanitized.contains("done"));
    }

    #[test]
    fn build_chat_api_messages_sanitizes_image_payloads_in_replayed_history() {
        let conversation = Conversation {
            id: "conv_test".to_string(),
            title: "test".to_string(),
            provider_id: "provider".to_string(),
            model: "model".to_string(),
            messages: vec![
                test_chat_message("msg_user_1", "user", "make an image", 1),
                ChatMessage {
                    id: "msg_assistant_1".to_string(),
                    role: "assistant".to_string(),
                    content: "![img](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA)".to_string(),
                    attachments: Vec::new(),
                    reasoning: None,
                    artifacts: Vec::new(),
                    tool_calls: Vec::new(),
                    segments: Vec::new(),
                    agent_plan: None,
                    api_messages: vec![
                        serde_json::json!({
                            "role": "assistant",
                            "content": "![img](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA)"
                        }),
                        serde_json::json!({
                            "role": "tool",
                            "content": concat!(
                                "stdout:\n",
                                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n"
                            )
                        }),
                    ],
                    model_messages: Vec::new(),
                    active_skill_id: None,
                    run_entry: None,
                    stream_outcome: None,
                    usage: None,
                    anchor_usage: None,
                    group_id: None,
                    provider_id: None,
                    model: None,
                    timestamp: 2,
                },
            ],
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 1,
            updated_at: 2,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState::default(),
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
        thinking_level: None,
            reply_models: Vec::new(),
            group_selections: std::collections::HashMap::new(),
            forked_from: None,
            agent_runtime: crate::chat::AgentRuntimeConfig::default(),
        };

        let messages = build_chat_api_messages("system", &conversation, None, None, &[])
            .expect("messages should build");
        let serialized = serde_json::to_string(&messages).expect("messages serialize");

        assert!(
            serialized.contains("[image data URL omitted; image is available as a tool artifact]")
        );
        assert!(
            serialized.contains("[image base64 omitted; image is available as a tool artifact]")
        );
        assert!(!serialized.contains("data:image/png;base64"));
        assert!(!serialized.contains("iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB"));
    }

    #[test]
    fn context_token_count_ignores_image_data_url_payloads() {
        let image_part = serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": format!(
                    "data:image/png;base64,{}",
                    "A".repeat(200_000)
                )
            }
        });
        let text_part = serde_json::json!({
            "type": "text",
            "text": "describe this image"
        });

        assert_eq!(count_tokens_in_value(&image_part), 0);
        assert_eq!(
            count_tokens_in_value(&text_part),
            agent_prepare::estimate_tokens("describe this image")
        );
    }

    #[test]
    fn image_token_estimates_follow_provider_dimension_rules() {
        assert_eq!(
            estimate_image_tokens_for_dimensions(None, "gpt-4o", 1024, 1024),
            765
        );
        assert_eq!(
            estimate_image_tokens_for_dimensions(None, "gpt-4o", 2048, 4096),
            1105
        );
        assert_eq!(
            estimate_image_tokens_for_dimensions(None, "gpt-4.1-mini", 1024, 1024),
            1659
        );
        assert_eq!(
            estimate_image_tokens_for_dimensions(None, "claude-sonnet-4", 1000, 1000),
            1334
        );
        assert_eq!(
            estimate_image_tokens_for_dimensions(None, "gemini-2.0-flash", 384, 384),
            258
        );
        assert_eq!(
            estimate_image_tokens_for_dimensions(None, "gemini-2.0-flash", 1024, 1024),
            1032
        );
    }

    // ===== 任务 06-30 多模型一问多答（步骤 3 + 步骤 4）=====

    fn test_conversation_with_messages(messages: Vec<ChatMessage>) -> Conversation {
        Conversation {
            id: "conv_multi".to_string(),
            title: "test".to_string(),
            provider_id: "openai".to_string(),
            model: "gpt-4o".to_string(),
            messages,
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 1,
            updated_at: 1,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState::default(),
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
            thinking_level: None,
            reply_models: Vec::new(),
            group_selections: std::collections::HashMap::new(),
            forked_from: None,
            agent_runtime: crate::chat::AgentRuntimeConfig::default(),
        }
    }

    fn grouped_assistant(id: &str, content: &str, group_id: &str, ts: i64) -> ChatMessage {
        let mut m = test_chat_message(id, "assistant", content, ts);
        m.group_id = Some(group_id.to_string());
        m.provider_id = Some("openai".to_string());
        m.model = Some("gpt-4o".to_string());
        m
    }

    fn test_settings_with_providers(provider_ids: &[&str]) -> Settings {
        let mut settings = Settings::default();
        settings.providers = provider_ids
            .iter()
            .map(|id| {
                serde_json::from_value::<ModelProvider>(serde_json::json!({
                    "id": id,
                    "name": id,
                    "baseUrl": "https://example.com/v1",
                    "apiKeys": ["k"],
                }))
                .expect("provider deserialize")
            })
            .collect();
        settings
    }

    /// 带 anchor_usage 的 assistant（openai_chat 口径：anchor_prompt = input_tokens）。
    fn assistant_with_anchor(id: &str, ts: i64, input_tokens: u64) -> ChatMessage {
        let mut m = test_chat_message(id, "assistant", "reply", ts);
        m.provider_id = Some("openai".to_string());
        m.anchor_usage = Some(crate::chat::model::ModelUsage {
            input_tokens: Some(input_tokens),
            output_tokens: Some(100),
            ..Default::default()
        });
        m
    }

    fn boundary_at(created_at: i64) -> CompactionBoundaryRecord {
        CompactionBoundaryRecord {
            id: "ctxbd_test".to_string(),
            source_until_message_id: "u1".to_string(),
            display_after_message_id: None,
            token_estimate_before: 0,
            token_estimate_after: 0,
            summary_content: String::new(),
            trigger: "manual".to_string(),
            created_at,
        }
    }

    #[test]
    fn resolve_usage_anchor_reports_prompt_and_trailing() {
        let conv = test_conversation_with_messages(vec![
            test_chat_message("u1", "user", "hi", 1),
            assistant_with_anchor("a1", 2, 100_000),
            test_chat_message("u2", "user", "follow-up question here", 3),
        ]);
        let provider = test_provider("openai", "OpenAI", vec!["gpt-4o"]);
        let (total, trailing) = resolve_usage_anchor(&conv, Some(&provider));
        // openai 无 total_tokens → input(100000) + output(100)。
        assert_eq!(total, Some(100_100), "openai anchor = input + output");
        // trailing = 锚点 assistant **之后** 的消息（新 user），> 0；锚点响应本身不算进 trailing。
        assert!(trailing > 0);
    }

    #[test]
    fn resolve_usage_anchor_none_without_usage() {
        let conv = test_conversation_with_messages(vec![
            test_chat_message("u1", "user", "hi", 1),
            test_chat_message("a1", "assistant", "reply", 2),
        ]);
        let provider = test_provider("openai", "OpenAI", vec!["gpt-4o"]);
        assert_eq!(resolve_usage_anchor(&conv, Some(&provider)), (None, 0));
    }

    #[test]
    fn resolve_usage_anchor_invalidated_on_provider_switch() {
        let conv = test_conversation_with_messages(vec![
            test_chat_message("u1", "user", "hi", 1),
            assistant_with_anchor("a1", 2, 100_000),
        ]);
        // 会话切换到 anthropic：旧 openai 锚点计数口径不可比 → 作废。
        let provider = test_provider("anthropic", "Anthropic", vec!["claude"]);
        assert_eq!(resolve_usage_anchor(&conv, Some(&provider)), (None, 0));
    }

    #[test]
    fn resolve_usage_anchor_invalidated_after_compaction() {
        // 手动压缩发生在锚点消息之后（boundary.created_at=10 > anchor.ts=2）→ 锚点失真 → 作废（R4）。
        let mut conv = test_conversation_with_messages(vec![
            test_chat_message("u1", "user", "hi", 1),
            assistant_with_anchor("a1", 2, 100_000),
        ]);
        conv.context_state.compaction_boundaries = vec![boundary_at(10)];
        let provider = test_provider("openai", "OpenAI", vec!["gpt-4o"]);
        assert_eq!(resolve_usage_anchor(&conv, Some(&provider)), (None, 0));
    }

    #[test]
    fn resolve_usage_anchor_kept_when_compaction_precedes_anchor() {
        // run 内自动压缩：boundary.created_at=2 <= 压缩后生成的 assistant.ts=5 → 锚点仍有效。
        let mut conv = test_conversation_with_messages(vec![
            test_chat_message("u1", "user", "hi", 1),
            assistant_with_anchor("a1", 5, 100_000),
        ]);
        conv.context_state.compaction_boundaries = vec![boundary_at(2)];
        let provider = test_provider("openai", "OpenAI", vec!["gpt-4o"]);
        let (total, _) = resolve_usage_anchor(&conv, Some(&provider));
        assert_eq!(total, Some(100_100)); // input(100000) + output(100)
    }

    #[test]
    fn resolve_reply_arms_dedups_filters_and_caps() {
        let settings = test_settings_with_providers(&["openai", "anthropic"]);

        // 单模型 / 空 → ≤1（调用方走单模型路径）。
        assert!(resolve_reply_arms(&settings, &[]).unwrap().is_empty());
        let one = vec![ModelRef {
            provider_id: "openai".to_string(),
            model: "gpt-4o".to_string(),
        }];
        assert_eq!(resolve_reply_arms(&settings, &one).unwrap().len(), 1);

        // 去重（相同 provider+model）、保序、丢空、丢未知 provider。
        let many = vec![
            ModelRef { provider_id: "openai".to_string(), model: "gpt-4o".to_string() },
            ModelRef { provider_id: "openai".to_string(), model: "gpt-4o".to_string() }, // dup
            ModelRef { provider_id: "anthropic".to_string(), model: "claude-3".to_string() },
            ModelRef { provider_id: "ghost".to_string(), model: "y".to_string() }, // unknown provider
        ];
        let arms = resolve_reply_arms(&settings, &many).unwrap();
        assert_eq!(
            arms,
            vec![
                ("openai".to_string(), "gpt-4o".to_string()),
                ("anthropic".to_string(), "claude-3".to_string()),
            ]
        );

        // 空 provider 也被丢弃（单独验证，避免与上面的 4 条上限冲突）。
        let with_empty = vec![
            ModelRef { provider_id: "openai".to_string(), model: "gpt-4o".to_string() },
            ModelRef { provider_id: "".to_string(), model: "x".to_string() },
        ];
        assert_eq!(resolve_reply_arms(&settings, &with_empty).unwrap().len(), 1);

        // 超上限 → Err。
        let over: Vec<ModelRef> = (0..(MAX_REPLY_MODELS + 1))
            .map(|i| ModelRef {
                provider_id: "openai".to_string(),
                model: format!("m{i}"),
            })
            .collect();
        assert!(resolve_reply_arms(&settings, &over).is_err());
    }

    #[test]
    fn build_assistant_message_records_group_meta_only_when_provided() {
        let single = build_assistant_message(
            "msg_single".to_string(),
            "hi".to_string(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            Some("send"),
            Some("completed"),
            None,
            None,
            None,
            None,
        );
        assert!(single.group_id.is_none());
        assert!(single.provider_id.is_none());
        assert!(single.model.is_none());

        let arm = build_assistant_message(
            "msg_arm".to_string(),
            "hi".to_string(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            Some("send"),
            Some("completed"),
            None,
            None,
            None,
            Some((
                "grp_1".to_string(),
                "anthropic".to_string(),
                "claude-3".to_string(),
            )),
        );
        assert_eq!(arm.group_id.as_deref(), Some("grp_1"));
        assert_eq!(arm.provider_id.as_deref(), Some("anthropic"));
        assert_eq!(arm.model.as_deref(), Some("claude-3"));
    }

    #[test]
    fn build_error_arm_message_keeps_column_identity_and_marks_error() {
        // 报错臂合成的「错误列」：保留 group_id/provider/model，错误信息进 content，
        // stream_outcome 标 error —— 这样前端仍按 group_id 聚合出该列，不再被吞掉。
        let msg = build_error_arm_message(
            "grp_err",
            "provider-x".to_string(),
            "model-y".to_string(),
            "上游返回 429：额度不足".to_string(),
            "send",
            None,
        );
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.group_id.as_deref(), Some("grp_err"));
        assert_eq!(msg.provider_id.as_deref(), Some("provider-x"));
        assert_eq!(msg.model.as_deref(), Some("model-y"));
        assert_eq!(msg.stream_outcome.as_deref(), Some("error"));
        assert!(msg.content.contains("429"));
        assert!(msg.id.starts_with("msg_"));
    }

    #[test]
    fn build_chat_api_messages_keeps_only_selected_group_answer() {
        // user + 3 答（grp_1）。默认无 group_selections → 取顺序第一条 a1。
        let messages = vec![
            test_chat_message("msg_user", "user", "compare these", 1),
            grouped_assistant("msg_a1", "answer one", "grp_1", 2),
            grouped_assistant("msg_a2", "answer two", "grp_1", 3),
            grouped_assistant("msg_a3", "answer three", "grp_1", 4),
        ];
        let mut conversation = test_conversation_with_messages(messages);

        let built = build_chat_api_messages("system", &conversation, Some(0), None, &[])
            .expect("build");
        let serialized = serde_json::to_string(&built).unwrap();
        assert!(serialized.contains("answer one"));
        assert!(!serialized.contains("answer two"));
        assert!(!serialized.contains("answer three"));

        // 用户点选第二条 → 历史改为只含 a2。
        conversation
            .group_selections
            .insert("grp_1".to_string(), "msg_a2".to_string());
        let built = build_chat_api_messages("system", &conversation, Some(0), None, &[])
            .expect("build");
        let serialized = serde_json::to_string(&built).unwrap();
        assert!(!serialized.contains("answer one"));
        assert!(serialized.contains("answer two"));
        assert!(!serialized.contains("answer three"));
    }

    #[test]
    fn build_chat_api_messages_default_first_follows_deletion() {
        // 删除第一条后，默认「顺序第一条」自动变成原第二条。
        let messages = vec![
            test_chat_message("msg_user", "user", "compare these", 1),
            grouped_assistant("msg_a2", "answer two", "grp_1", 3),
            grouped_assistant("msg_a3", "answer three", "grp_1", 4),
        ];
        let conversation = test_conversation_with_messages(messages);
        let built = build_chat_api_messages("system", &conversation, Some(0), None, &[])
            .expect("build");
        let serialized = serde_json::to_string(&built).unwrap();
        assert!(serialized.contains("answer two"));
        assert!(!serialized.contains("answer three"));
    }

    #[test]
    fn build_chat_api_messages_default_skips_errored_arm() {
        // 首臂报错（stream_outcome=error）+ 次臂正常，且无显式 group_selections：
        // 默认应保留首个「非错误」臂、跳过错误臂文案，避免把错误回灌给模型（F2 修复）。
        let mut a1 = grouped_assistant("msg_a1", "arm one failed", "grp_1", 2);
        a1.stream_outcome = Some("error".to_string());
        let a2 = grouped_assistant("msg_a2", "arm two ok", "grp_1", 3);
        let messages = vec![
            test_chat_message("msg_user", "user", "compare these", 1),
            a1,
            a2,
        ];
        let conversation = test_conversation_with_messages(messages);
        let built = build_chat_api_messages("system", &conversation, Some(0), None, &[])
            .expect("build");
        let serialized = serde_json::to_string(&built).unwrap();
        assert!(
            !serialized.contains("arm one failed"),
            "errored arm must be excluded from context"
        );
        assert!(
            serialized.contains("arm two ok"),
            "first non-errored arm is retained"
        );
    }

    #[test]
    fn build_chat_api_messages_single_answer_unaffected() {
        // 无 group_id 的常规历史完全不受过滤影响（防回归 AC5/AC6）。
        let messages = vec![
            test_chat_message("msg_user", "user", "hello", 1),
            test_chat_message("msg_a", "assistant", "world", 2),
        ];
        let conversation = test_conversation_with_messages(messages);
        let built = build_chat_api_messages("system", &conversation, Some(0), None, &[])
            .expect("build");
        let serialized = serde_json::to_string(&built).unwrap();
        assert!(serialized.contains("hello"));
        assert!(serialized.contains("world"));
    }

    #[test]
    fn group_excludes_only_non_selected_assistants() {
        let messages = vec![
            test_chat_message("msg_user", "user", "q", 1),
            grouped_assistant("msg_a1", "a1", "grp_1", 2),
            grouped_assistant("msg_a2", "a2", "grp_1", 3),
        ];
        let conversation = test_conversation_with_messages(messages);
        // 默认选第一条：a1 保留、a2 排除。
        assert!(!group_answer_excluded_from_context(
            &conversation,
            &conversation.messages[1]
        ));
        assert!(group_answer_excluded_from_context(
            &conversation,
            &conversation.messages[2]
        ));
        // user 消息（即便带 group_id）永不被该过滤排除。
        let mut user_in_group = test_chat_message("msg_u2", "user", "uq", 4);
        user_in_group.group_id = Some("grp_1".to_string());
        assert!(!group_answer_excluded_from_context(&conversation, &user_in_group));
    }

    #[test]
    fn stale_group_selection_falls_back_to_first_remaining() {
        // D5/AC4：删除显式选中条后，清掉指向已删消息的 group_selections，选中条回退到组内
        // 顺序第一条（这里模拟 chat_delete_message / chat_regenerate_message 的清理后状态）。
        let messages = vec![
            test_chat_message("msg_user", "user", "q", 1),
            grouped_assistant("msg_a1", "answer one", "grp_1", 2),
            grouped_assistant("msg_a2", "answer two", "grp_1", 3),
        ];
        let mut conversation = test_conversation_with_messages(messages);
        // 用户显式选了第二条。
        conversation
            .group_selections
            .insert("grp_1".to_string(), "msg_a2".to_string());

        // 模拟删除被选中的 msg_a2：移除消息 + 删除命令对 group_selections 的清理。
        conversation.messages.retain(|m| m.id != "msg_a2");
        if conversation
            .group_selections
            .get("grp_1")
            .map(String::as_str)
            == Some("msg_a2")
        {
            conversation.group_selections.remove("grp_1");
        }

        // 残余的 msg_a1 必须仍进上下文（回退到组内第一条），而非被整组排除。
        assert!(!group_answer_excluded_from_context(
            &conversation,
            &conversation.messages[1]
        ));
        let built = build_chat_api_messages("system", &conversation, Some(0), None, &[])
            .expect("build");
        let serialized = serde_json::to_string(&built).unwrap();
        assert!(serialized.contains("answer one"));
    }

    // ===== 对话分支（方案 B）=====

    #[test]
    fn build_fork_messages_keeps_prefix_including_anchor() {
        let messages = vec![
            test_chat_message("m0", "user", "q1", 1),
            test_chat_message("m1", "assistant", "a1", 2),
            test_chat_message("m2", "user", "q2", 3),
            test_chat_message("m3", "assistant", "a2", 4),
        ];
        // 在 m2（user）建分支：保留 m0..=m2，丢弃其后。
        let forked = build_fork_messages(&messages, 2);
        let ids: Vec<&str> = forked.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["m0", "m1", "m2"]);
        // 源不变。
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn build_fork_messages_collapses_group_to_selected_column() {
        // 一轮多模型多答：m0 user，m1/m2/m3 同组三列答案。锚点选中 m2。
        let messages = vec![
            test_chat_message("m0", "user", "q", 1),
            grouped_assistant("m1", "col1", "grp", 2),
            grouped_assistant("m2", "col2", "grp", 3),
            grouped_assistant("m3", "col3", "grp", 4),
        ];
        let forked = build_fork_messages(&messages, 2);
        let ids: Vec<&str> = forked.iter().map(|m| m.id.as_str()).collect();
        // 只留 user + 选中列 m2，丢弃 m1（前序兄弟列）与 m3（切片外）。
        assert_eq!(ids, vec!["m0", "m2"]);
        // 锚点转普通单答（去 group_id）。
        assert_eq!(forked.last().unwrap().group_id, None);
    }

    #[test]
    fn build_fork_messages_non_group_anchor_leaves_group_id_untouched() {
        let messages = vec![
            test_chat_message("m0", "user", "q", 1),
            test_chat_message("m1", "assistant", "a", 2),
        ];
        let forked = build_fork_messages(&messages, 1);
        assert_eq!(forked.len(), 2);
        assert_eq!(forked[1].group_id, None);
    }

    #[test]
    fn fork_group_selection_cleanup_drops_dangling_and_collapsed() {
        // 模拟 chat_fork_conversation 内的 group_selections 清理逻辑。
        // 新前缀：g1 组完整保留（选中 s1）；g2 组被折叠成单答（锚点去 group_id）。
        let messages = vec![
            test_chat_message("u1", "user", "q1", 1),
            grouped_assistant("s1", "g1a", "g1", 2),
            grouped_assistant("s2", "g1b", "g1", 3),
            test_chat_message("u2", "user", "q2", 4),
            // g2 折叠后：这条已去 group_id（模拟 build_fork_messages 结果）。
            test_chat_message("s3", "assistant", "g2sel", 5),
        ];
        let existing_groups: std::collections::HashMap<&str, &str> = messages
            .iter()
            .filter_map(|m| m.group_id.as_deref().map(|g| (m.id.as_str(), g)))
            .collect();
        let mut selections: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        selections.insert("g1".to_string(), "s1".to_string()); // 有效：s1 仍在且仍属 g1
        selections.insert("g2".to_string(), "s3".to_string()); // 失效：s3 已去 group_id（组被折叠）
        selections.insert("g3".to_string(), "gone".to_string()); // 失效：消息已不存在
        selections.retain(|group_id, sel| existing_groups.get(sel.as_str()) == Some(&group_id.as_str()));

        assert_eq!(selections.len(), 1);
        assert_eq!(selections.get("g1").map(String::as_str), Some("s1"));
    }
}
