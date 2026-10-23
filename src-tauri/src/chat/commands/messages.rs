use serde_json::Value;
use tauri::{AppHandle, State};
use crate::mcp::types::ChatToolArtifact;
use crate::settings::Settings;
use crate::state::AppState;
use super::context::{compute_context_state, emit_chat_context_state};
use super::interaction::emit_chat_plan_state;
use super::title::{generate_title, is_auto_title};
use super::{AgentPlanState, ChatMessage, ChatMessageSegment, Conversation, ToolCallRecord};
pub(super) use crate::chat::message_content::*;

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
    // 降级兜底的结构化描述；Some 时前端渲染成独立错误卡片。
    degraded: Option<crate::chat::agent::recovery::DegradedAnswer>,
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
    let mut message = message;
    message.provider_id = Some(conversation.provider_id.clone());
    message.model = Some(conversation.model.clone());
    message.degraded = degraded;
    if let Some(live) = state
        .chat_runtime()
        .context_measurement(&conversation.id)
        .filter(|live| live.message_id == message.id)
    {
        message.cache_pair_input = live.run_cache.map(|pair| pair.0);
        message.cache_pair_read = live.run_cache.map(|pair| pair.1);
        conversation.context_state.measurement_seq = live.seq;
        conversation.context_state.lifecycle_id = live.lifecycle_id;
        conversation.context_state.request_measurement = Some(live.stored());
    }
    let stored_content = message.content.clone();

    // 标题还是自动生成的样子吗——占位「新对话」，或者等于第一句用户消息的启发式结果。
    // 用户手动重命名过的标题不会等于启发式结果，所以据此判断不会覆盖用户的命名。
    let title_looks_auto = is_auto_title(&conversation.title, title_from_first_user);
    let is_external =
        conversation.agent_runtime.kind == crate::chat::types::AgentRuntimeKind::External;

    // 外部 CLI 对话优先用 CLI 自己生成的标题：那是用户在 CLI 里看到的那个，而且不花模型调用。
    //
    // **每轮都试一次**（一次文件读，很便宜）：CLI 是异步写标题的（claude 的 `ai-title`），
    // 首轮回复落盘时往往还没有，只在第一轮试就永远拿不到。
    let external_cli_title = if is_external && title_looks_auto {
        crate::external_agents::import::cli_session_title(app, &conversation.id)
            .filter(|title| !title.trim().is_empty() && *title != conversation.title)
    } else {
        None
    };

    let mut background_title_user = None;
    let generated_title = if external_cli_title.is_some() {
        external_cli_title
    } else if let Some(user_content) = title_from_first_user {
        // 首轮发送后标题可能已经落成「第一句用户消息」截断（不再是占位「新对话」），
        // 卡在 `== "新对话"` 上的话，标题模型这一步压根不会触发。
        // 工具型首轮会在生成中落 assistant 草稿快照，不能用 messages.len() == 1
        // 判断首轮，否则只要首轮用了工具就会永远跳过标题请求。
        let is_first_user_turn = conversation
            .messages
            .iter()
            .filter(|message| message.role == "user")
            .count()
            == 1;
        let wants_generation = is_first_user_turn && title_looks_auto;
        if wants_generation {
            // 被取消的首条回复不值得花一次模型调用生成标题（标题生成是一次
            // 带 30s 超时的 LLM 请求，会显著拖慢"停止"后 invoke 的返回 / 输入框解锁）。
            // 用本地启发式标题兜底；下一条正常回复或重命名仍可得到更好的标题。
            if stream_outcome == Some("cancelled") {
                Some(generate_title(user_content))
            } else {
                // 回复终态不等待标题模型（最长 30s）；持久化完成后另起任务补标题。
                background_title_user = Some(user_content.to_string());
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let first_user = title_from_first_user.map(str::to_string);
    let message_plan = message.agent_plan.clone();
    let plan_update = message_plan.clone();
    let conversation_id = conversation.id.clone();
    let mut persisted = crate::chat::repository::repository(app)
        .mutate(app, &conversation_id, |latest| {
            adopt_compacted_context(latest, conversation, &message.id);
            upsert_assistant_message(latest, message);
            if let Some(title) = generated_title {
                let title_is_still_auto = is_auto_title(&latest.title, first_user.as_deref());
                if title_is_still_auto {
                    latest.title = title;
                }
            }
            if let Some(plan) = message_plan {
                latest.agent_plan_state = plan;
            }
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;

    // 终态已落主文件,run 期间的草稿日志随即失效(见 chat::draft_journal)。
    crate::chat::draft_journal::clear_draft(app, &conversation_id);

    // Context is derived from the full conversation. Commit it with CAS and
    // recompute once if an unrelated operation advanced the revision while the
    // potentially expensive estimate/compression was running.
    for attempt in 0..2 {
        match compute_context_state(app, state, &persisted, None, &[]).await {
            Ok(context_state) => {
                persisted.context_state = context_state;
                let next_context = persisted.context_state.clone();
                match crate::chat::repository::repository(app)
                    .update_context(app, &conversation_id, persisted.revision, next_context)
                    .await
                {
                    Ok(latest) => {
                        persisted = latest;
                        emit_chat_context_state(
                            app,
                            &conversation_id,
                            persisted.revision,
                            &persisted.context_state,
                        );
                        break;
                    }
                    Err(crate::chat::repository::ConversationRepositoryError::Conflict {
                        ..
                    }) if attempt == 0 => {
                        persisted = crate::chat::repository::repository(app)
                            .get(app, &conversation_id)
                            .await
                            .map_err(crate::chat::repository::repository_error)?;
                    }
                    Err(err) => {
                        eprintln!("Context state commit failed after assistant reply: {err}");
                        break;
                    }
                }
            }
            Err(err) => {
                eprintln!("Context usage estimate failed after assistant reply: {err}");
                break;
            }
        }
    }

    *conversation = persisted;
    if let Some(first_user) = background_title_user {
        super::title::schedule_auto_title_summary(
            app.clone(),
            settings.clone(),
            conversation.clone(),
            first_user,
            stored_content,
        );
    }
    if let Some(plan) = plan_update {
        emit_chat_plan_state(app, &conversation_id, conversation.revision, &plan);
    }
    Ok(())
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
pub(super) async fn persist_partial_assistant_snapshot(
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
        cache_pair_input: None,
        cache_pair_read: None,
        group_id: None,
        provider_id: None,
        model: None,
        timestamp: chrono::Local::now().timestamp(),
        degraded: None,
    };
    // 草稿不再整会话重写:追加进 append-only 日志(见 chat::draft_journal 模块注释),
    // 主会话文件与索引在 run 期间完全不动;终态写入后 push_assistant_message 清日志。
    let app_handle = app.clone();
    let conversation_id = conversation_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        crate::chat::draft_journal::append_draft(&app_handle, &conversation_id, draft)
    })
    .await
    .map_err(|e| format!("draft journal task failed: {e}"))?
}
