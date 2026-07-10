use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::chat::agent::prepare as agent_prepare;
use crate::chat::attachments::{
    compose_user_content_for_api, save_message_attachments, stored_image_paths_for_attachments,
    title_source_for_user_message,
};
#[cfg(test)]
use crate::chat::model::{
    openai_messages_from_model_messages, MessagePart, ModelMessage, ModelRole,
};
use crate::chat::model_metadata::{
    chat_max_output_tokens_for_model, model_can_generate_images_directly,
    reasoning_efforts_for_model,
};
use crate::mcp;
#[cfg(test)]
use crate::mcp::ChatToolDefinition;
use crate::settings::Settings;
#[cfg(test)]
use crate::settings::{ModelProvider, SessionModel};
use crate::skills;
use crate::state::AppState;

use super::model_call::{
    chat_missing_model_error, format_chat_missing_api_key_error,
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
    conversation_attachments_dir, find_set_by_id, load_conversation, save_conversation,
};
use super::{
    AgentPlanState, ChatMessage, ChatMessageSegment, ChatMessageSegmentKind,
    ChatMessageSegmentPhase, Conversation, ToolCallRecord,
    ToolCallStatus,
};
#[cfg(test)]
use super::{AgentTodoState, CompactionBoundaryRecord, ConversationContextState};

mod agent_host;
use agent_host::{ChatAgentHost, RegistryToolExecutor};
#[cfg(debug_assertions)]
use agent_host::ProbeAgentHost;

pub(crate) mod attachments;

pub(crate) mod catalog;

pub(crate) use catalog::create_assistant_via_builder;
use catalog::{
    chat_memory_prompt_for_request, is_builder_conversation,
    project_prompt_context_for, strip_transcripts_for_frontend,
};
#[cfg(test)]
use catalog::assistant_from_builder_args;

pub(crate) mod context;

pub(crate) mod interaction;

mod title;

mod tooling;

mod messages;

mod sanitization;

mod reply_runtime;
use reply_runtime::{
    resolve_reply_arms, ArmReplyOutcome, ChatReplyGuard, ChatSendReservation, ReplyArm,
    CHAT_REPLY_BUSY_ERROR, MAX_REPLY_MODELS,
};

mod fan_out;
use fan_out::run_reply_fan_out;

mod direct_image;
use direct_image::complete_direct_image_generation_reply;

#[cfg(debug_assertions)]
mod probe_runtime;
#[cfg(debug_assertions)]
pub(crate) use probe_runtime::run_chat_probe;

pub(crate) mod mutations;

pub(crate) use interaction::{
    emit_chat_stream_delta, emit_chat_stream_done, emit_chat_tool_record,
};
use interaction::wait_for_chat_cancel;
#[cfg(test)]
use title::generate_title;
pub(crate) use messages::push_assistant_message;
#[cfg(test)]
use mutations::{apply_regenerate_truncation, build_fork_messages};
#[cfg(test)]
use sanitization::sanitize_image_payloads_for_model;
use messages::{
    auxiliary_tool_segments, build_assistant_message,
    capture_agent_plan_draft_if_needed, merge_latest_agent_plan_state,
    merge_latest_agent_todo_state, tool_segment_for_record,
};
#[cfg(test)]
use messages::{
    assistant_model_messages_for_storage, build_error_arm_message, content_from_segments,
    normalize_assistant_segments,
    reasoning_from_segments, reconcile_orphan_tool_segments, replace_final_text_segments_for_edit,
};
use tooling::{
    append_agent_ask_user_tools, append_agent_todo_tools, apply_agent_plan_tool_filter,
    apply_inline_code_request_tool_filter, list_tools_for_chat, resolve_forced_skill_id,
    try_apply_skill_slash_trigger,
};
#[cfg(test)]
use tooling::should_answer_inline_without_file_write;
#[cfg(test)]
use title::{build_title_summary_prompt, sanitize_generated_title};

#[cfg(test)]
use interaction::{approve_agent_plan_for_execution, format_tool_approval_summary};

use context::{
    build_chat_api_messages, compress_conversation_context, compute_context_state,
    context_likely_over_limit, emit_chat_context_state,
    resolve_usage_anchor, rollback_user_message_after_failed_send, should_auto_compress_context,
};
#[cfg(test)]
use context::{
    count_tokens_in_value, estimate_image_tokens_for_dimensions,
    group_answer_excluded_from_context, mark_summary_stale_if_needed,
};
#[cfg(test)]
use super::ConversationContextSummary;




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



fn agent_run_entry_label(entry: crate::chat::agent::AgentRunEntry) -> &'static str {
    match entry {
        crate::chat::agent::AgentRunEntry::Send => "send",
        crate::chat::agent::AgentRunEntry::Regenerate => "regenerate",
    }
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

use crate::chat::agent::execute::truncate_chars;


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
