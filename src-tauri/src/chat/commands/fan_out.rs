use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::state::AppState;

use super::context::{compute_context_state, emit_chat_context_state};
use super::messages::{
    commit_reply_history, upsert_assistant_message, validate_reply_history_branch,
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
/// - 至少一臂完成回答 → `Ok(())`。
/// - 全部臂被取消 → `Err("cancelled")`。
/// - 无臂完成回答且有错误 → `Err(首个错误信息)`。
/// 报错臂的列消息以 `stream_outcome="error"` 保存。
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
    let registered: Vec<_> = arms
        .iter()
        .enumerate()
        .map(|(arm_index, (provider_id, model))| {
            let arm = ReplyArm::new(
                group_id.to_string(),
                arms.len(),
                arms.len(),
                arm_index,
                provider_id.clone(),
                model.clone(),
            );
            let guard = arm.register(app, &conversation.id, conversation.revision);
            (arm, guard)
        })
        .collect();
    let arm_futures = registered.iter().map(|(arm, _)| {
        let mut arm_conversation = conversation.clone();
        async move {
            complete_assistant_reply_inner(
                app,
                state,
                &mut arm_conversation,
                None,
                last_user_api_content,
                last_user_image_paths,
                active_skill_id,
                crate::chat::agent::AgentRunEntry::Send,
                Some(arm),
                false,
                None,
            )
            .await
        }
    });
    let results = futures::future::join_all(arm_futures).await;

    let mut completed = 0usize;
    let mut first_error: Option<String> = None;
    let mut terminals = Vec::new();
    let mut arm_results = Vec::new();
    for ((arm, _), outcome) in registered.iter().zip(results) {
        let ArmReplyOutcome {
            message,
            history,
            error,
        } = arm.normalize_outcome(outcome, run_entry, active_skill_id);
        let Some(message) = message else {
            terminals.push((&arm.run_id, "cancelled", String::new()));
            continue;
        };
        let status = match message.stream_outcome.as_deref() {
            Some("cancelled") => "cancelled",
            Some("completed" | "recovered" | "done") | None => {
                completed += 1;
                "completed"
            }
            Some(status) => {
                first_error.get_or_insert_with(|| {
                    error.unwrap_or_else(|| {
                        if status == "error" && !message.content.is_empty() {
                            let preview = crate::chat::protocol::terminal_content(&message.content);
                            if preview.is_empty() {
                                "模型回答失败".to_string()
                            } else {
                                preview.to_owned()
                            }
                        } else {
                            status.to_string()
                        }
                    })
                });
                "error"
            }
        };
        terminals.push((
            &arm.run_id,
            status,
            crate::chat::protocol::terminal_content(&message.content).to_owned(),
        ));
        arm_results.push((message, history));
    }

    if !arm_results.is_empty() {
        let selected_id = arm_results
            .iter()
            .find(|(message, _)| {
                matches!(
                    message.stream_outcome.as_deref(),
                    Some("completed" | "recovered" | "done") | None
                )
            })
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
                latest.context_state = compute_context_state(app, state.inner(), latest);
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
                        run_id,
                        &error,
                        "",
                        conversation.revision,
                    );
                }
                return Err(error);
            }
        };
        if let Some(warning) = warning {
            persisted.context_state.warning = Some(warning);
        }
        emit_chat_context_state(
            app,
            &persisted.id,
            persisted.revision,
            &persisted.context_state,
        );
        *conversation = persisted;
        for (run_id, outcome, content) in terminals {
            crate::chat::protocol::finish_run(
                app,
                run_id,
                outcome,
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
    } else {
        for (run_id, outcome, content) in terminals {
            crate::chat::protocol::finish_run(
                app,
                run_id,
                outcome,
                &content,
                conversation.revision,
            );
        }
    }
    match (completed, first_error) {
        (0, Some(error)) => Err(error),
        (0, None) => Err("cancelled".to_string()),
        _ => Ok(()),
    }
}
