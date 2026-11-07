use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::state::AppState;

use super::context::{compute_context_state, emit_chat_context_state};
use super::messages::{
    build_error_arm_message, commit_reply_history, upsert_assistant_message,
    validate_reply_history_branch,
};
use super::reply_runtime::{ArmReplyOutcome, ReplyArm};
use super::{agent_run_entry_label, complete_assistant_reply_inner, Conversation};

/// 多模型一问多答（任务 06-30 步骤 3）的协调者。
///
/// 对每个臂 `(provider_id, model)`：在会话的**独立克隆**上并发跑一次 agent loop
/// （`complete_assistant_reply_inner` 的 arm 模式），各臂自带 message_id/run_id/generation +
/// 共享 `group_id`，工具自动批准、**不直接落盘**。全部臂结束后，把各臂产出的 assistant
/// 消息按 id 一次性 `upsert` 进最新会话，再统一计算并提交上下文，
/// 从根本上避开 N 条并发 run 同写 `conversations/{id}.json` 的竞态。
///
/// 返回：
/// - 至少一列产出（成功**或**报错）→ `Ok(())`。报错臂也会合成一条 `stream_outcome="error"`
///   的列消息落库，避免整列被吞（只剩能正常回答的模型）。
/// - 全部臂被取消 → `Err("cancelled")`。
/// - 无任何产出（理论兜底）→ `Err(首个错误信息)`。
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_reply_fan_out(
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
    let source_branch_id = conversation
        .builtin_history
        .as_ref()
        .map(|history| history.active_branch_id.clone());
    let arm_futures = arms
        .iter()
        .enumerate()
        .map(|(arm_index, (provider_id, model))| {
            let mut arm_conversation = conversation.clone();
            let provider_id = provider_id.clone();
            let model = model.clone();
            let arm = ReplyArm {
                group_id: group_id.to_string(),
                group_size: arms.len(),
                arm_index,
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
                    None,
                )
                .await;
                (outcome, provider_id, model)
            }
        });

    let results = futures::future::join_all(arm_futures).await;

    let mut produced = 0usize;
    let mut completed = 0usize;
    let mut cancelled = 0usize;
    let mut first_error: Option<String> = None;
    let mut terminals = Vec::new();
    let mut arm_results = Vec::new();
    for (outcome, provider_id, model) in results {
        match outcome {
            Ok(ArmReplyOutcome {
                message: Some(message),
                run_id,
                error: _,
                history,
            }) => {
                if message.stream_outcome.as_deref() == Some("completed") {
                    completed += 1;
                }
                if let Some(run_id) = run_id {
                    terminals.push((
                        run_id,
                        message
                            .stream_outcome
                            .clone()
                            .unwrap_or_else(|| "completed".to_string()),
                        message.content.clone(),
                    ));
                }
                arm_results.push((message, history));
                produced += 1;
            }
            Ok(ArmReplyOutcome {
                message: None,
                run_id: Some(run_id),
                error: Some(err),
                history,
            }) => {
                if err == "cancelled" {
                    terminals.push((run_id, "cancelled".to_string(), String::new()));
                    cancelled += 1;
                    continue;
                }
                let message = build_error_arm_message(
                    group_id,
                    provider_id,
                    model,
                    err.clone(),
                    run_entry,
                    active_skill_id,
                );
                terminals.push((run_id, "error".to_string(), message.content.clone()));
                arm_results.push((message, history));
                produced += 1;
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
            Ok(ArmReplyOutcome {
                message: None,
                run_id: _,
                error: _,
                history: _,
            }) => {
                // 不应发生（arm 模式必返回消息），保守计为无产出。
            }
            Err(err) if err == "cancelled" => {
                cancelled += 1;
            }
            Err(err) => {
                // 报错臂也保留为一列：否则整列被吞、只剩能正常回答的模型。合成一条
                // content=错误信息、stream_outcome="error" 的 assistant 列消息落库。
                let message = build_error_arm_message(
                    group_id,
                    provider_id,
                    model,
                    err.clone(),
                    run_entry,
                    active_skill_id,
                );
                arm_results.push((message, None));
                produced += 1;
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }

    if produced > 0 {
        let selected_id = arm_results
            .iter()
            .find(|(message, _)| message.stream_outcome.as_deref() != Some("error"))
            .or_else(|| arm_results.first())
            .map(|(message, _)| message.id.clone())
            .expect("produced arm result");
        let persisted_result = crate::chat::repository::repository(app)
            .mutate(app, &conversation.id, |latest| {
                if let Some(branch_id) = source_branch_id.as_deref() {
                    validate_reply_history_branch(latest, branch_id)?;
                }
                for (_, history) in &arm_results {
                    if let Some(history) = history {
                        validate_reply_history_branch(latest, &history.branch_id)?;
                    }
                }
                for (message, history) in arm_results {
                    let message_id = message.id.clone();
                    upsert_assistant_message(latest, message);
                    if let Some(history) = history {
                        commit_reply_history(
                            app,
                            latest,
                            &message_id,
                            history,
                            message_id == selected_id,
                        )?;
                    }
                }
                latest.select_group_answer(group_id.to_string(), selected_id);
                Ok(())
            })
            .await;
        let (mut persisted, warning) = match persisted_result {
            Ok(persisted) => (persisted, None),
            Err(error @ crate::chat::repository::ConversationRepositoryError::Committed { .. }) => {
                let (persisted, warning) =
                    crate::chat::repository::recover_committed(app, error).await?;
                (persisted, Some(warning))
            }
            Err(error) => {
                let error = crate::chat::repository::repository_error(error);
                for (run_id, _, _) in terminals {
                    crate::chat::protocol::finish_run(
                        app,
                        &run_id,
                        &error,
                        "",
                        conversation.revision,
                    );
                }
                return Err(error);
            }
        };
        for attempt in 0..2 {
            match compute_context_state(app, state, &persisted, None, &[]).await {
                Ok(context_state) => {
                    match crate::chat::repository::repository(app)
                        .update_context(app, &persisted.id, persisted.revision, context_state)
                        .await
                    {
                        Ok(latest) => {
                            persisted = latest;
                            emit_chat_context_state(
                                app,
                                &persisted.id,
                                persisted.revision,
                                &persisted.context_state,
                            );
                            break;
                        }
                        Err(crate::chat::repository::ConversationRepositoryError::Conflict {
                            ..
                        }) if attempt == 0 => {
                            match crate::chat::repository::repository(app)
                                .get(app, &conversation.id)
                                .await
                                .map_err(crate::chat::repository::repository_error)
                            {
                                Ok(latest) => persisted = latest,
                                Err(error) => {
                                    eprintln!(
                                        "Conversation reload failed after multi-model fan-out context conflict: {error}"
                                    );
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "Context state commit failed after multi-model fan-out: {err}"
                            );
                            break;
                        }
                    }
                }
                Err(err) => {
                    eprintln!("Context usage estimate failed after multi-model fan-out: {err}");
                    break;
                }
            }
        }
        if let Some(warning) = warning {
            persisted.context_state.warning = Some(warning);
            emit_chat_context_state(
                app,
                &persisted.id,
                persisted.revision,
                &persisted.context_state,
            );
        }
        *conversation = persisted;
        for (run_id, outcome, content) in terminals {
            crate::chat::protocol::finish_run(
                app,
                &run_id,
                &outcome,
                &content,
                conversation.revision,
            );
        }
        if completed > 0 {
            crate::chat::completion_notification::notify_reply_completed(
                app,
                state.inner(),
                conversation,
            );
        }
        return Ok(());
    }

    for (run_id, outcome, content) in terminals {
        crate::chat::protocol::finish_run(app, &run_id, &outcome, &content, conversation.revision);
    }
    if cancelled > 0 && first_error.is_none() {
        return Err("cancelled".to_string());
    }
    Err(first_error.unwrap_or_else(|| "全部模型回答均失败".to_string()))
}
