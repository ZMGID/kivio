//! 工作区面板的对话级 Git 能力：为新对话建立独立 worktree、用模型写提交信息。

use std::time::Duration;

use tauri::{AppHandle, State};
use tokio::time::timeout;

use crate::chat::model_metadata::model_can_generate_images_directly;
use crate::chat::types::ConversationWorktree;
use crate::settings::SessionModel;
use crate::state::AppState;

use super::super::storage::{load_conversation, resolve_conversation_project};
use super::catalog::strip_transcripts_for_frontend;

/// 为还没开始的项目对话创建独立 worktree，此后 agent 在 worktree 里读写。
/// 发出第一条消息后不能再切换，外部 CLI 绑定的会话也不能改目录。
#[tauri::command]
pub(crate) async fn chat_create_conversation_worktree(
    app: AppHandle,
    conversation_id: String,
    base_branch: Option<String>,
) -> Result<serde_json::Value, String> {
    let conversation = load_conversation(&app, &conversation_id)?;
    ensure_worktree_allowed(&conversation)?;
    let project = resolve_conversation_project(&app, &conversation)?
        .ok_or_else(|| "只有项目对话可以使用 worktree。".to_string())?;
    let root = project
        .root_path
        .clone()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| format!("项目 {} 没有配置目录。", project.name))?;
    let id = conversation_id.clone();
    let (path, branch) = tauri::async_runtime::spawn_blocking(move || {
        crate::dock::git::create_conversation_worktree(&root, &id, base_branch.as_deref())
    })
    .await
    .map_err(|e| format!("create worktree join: {e}"))??;
    let worktree = ConversationWorktree { path, branch };
    let mut conversation = crate::chat::repository::repository(&app)
        .mutate(&app, &conversation_id, move |conversation| {
            ensure_worktree_allowed(conversation)?;
            conversation.worktree = Some(worktree);
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({ "success": true, "conversation": conversation }))
}

fn ensure_worktree_allowed(conversation: &crate::chat::Conversation) -> Result<(), String> {
    if conversation.worktree.is_some() {
        return Err("这条对话已经在使用 worktree。".to_string());
    }
    if !conversation.messages.is_empty() {
        return Err("对话开始后不能再切换环境。".to_string());
    }
    if conversation.external_cwd.is_some() {
        return Err("这条对话绑定了原工作目录，不能切换到 worktree。".to_string());
    }
    Ok(())
}

const COMMIT_MESSAGE_MAX_TOKENS: u32 = 1024;

fn commit_message_prompt(stat: &str, patch: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "role": "system",
            "content": "You write git commit messages. Output only the message: a subject line in the imperative mood, at most 72 characters, no trailing period; optionally a blank line and a short body. No quotes, code fences, or explanations.",
        }),
        serde_json::json!({
            "role": "user",
            "content": format!("Write a commit message for these staged changes.\n\nSummary:\n{stat}\n\nPatch:\n{patch}"),
        }),
    ]
}

/// 去掉代码块、引号和「Commit message:」前缀；主题行超长时截断到 72 字符。
pub(super) fn sanitize_commit_message(raw: &str) -> Option<String> {
    let mut lines: Vec<&str> = raw.trim().lines().collect();
    if lines
        .first()
        .is_some_and(|line| line.trim_start().starts_with("```"))
    {
        lines.remove(0);
    }
    if lines.last().is_some_and(|line| line.trim() == "```") {
        lines.pop();
    }
    let text = lines.join("\n");
    let mut text = text.trim();
    for prefix in [
        "Commit message:",
        "commit message:",
        "提交信息：",
        "提交信息:",
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim();
        }
    }
    let text = text.trim_matches(['"', '\'', '`']).trim();
    let mut parts = text.splitn(2, '\n');
    let subject: String = parts
        .next()?
        .trim()
        .trim_end_matches('.')
        .chars()
        .take(72)
        .collect();
    if subject.is_empty() {
        return None;
    }
    let body = parts.next().map(str::trim).filter(|body| !body.is_empty());
    Some(match body {
        Some(body) => format!("{subject}\n\n{body}"),
        None => subject,
    })
}

/// 用「标题/摘要」默认模型根据暂存区改动写提交信息。模型不可用或失败时返回错误，
/// 由调用方退回按文件名生成。不改动任何 Git 状态。
#[tauri::command]
pub(crate) async fn chat_generate_commit_message(
    app: AppHandle,
    state: State<'_, AppState>,
    workdir: String,
    conversation_id: Option<String>,
) -> Result<String, String> {
    let settings = state.settings_read().clone();
    let session_owned = conversation_id
        .as_deref()
        .and_then(|id| load_conversation(&app, id).ok())
        .map(|conversation| (conversation.provider_id, conversation.model));
    let session = session_owned
        .as_ref()
        .map(|(provider_id, model)| SessionModel {
            provider_id: provider_id.as_str(),
            model: model.as_str(),
        });
    let (provider_id, model) = settings.effective_title_summary_model_for_session(session);
    let provider = settings
        .get_provider(&provider_id)
        .cloned()
        .ok_or_else(|| "没有可用的标题摘要模型。".to_string())?;
    if !provider.authentication_ready() || model.trim().is_empty() {
        return Err("标题摘要模型未就绪。".to_string());
    }
    if model_can_generate_images_directly(&provider, &model) {
        return Err("生图模型不能写提交信息。".to_string());
    }
    let context = tauri::async_runtime::spawn_blocking(move || {
        crate::dock::git::staged_change_context(&workdir)
    })
    .await
    .map_err(|e| format!("commit context join: {e}"))??;
    let Some((stat, patch)) = context else {
        return Err("暂存区没有改动。".to_string());
    };
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    let request = crate::chat::model::generate_request_from_openai_messages(
        &model,
        commit_message_prompt(&stat, &patch),
        None,
        crate::chat::model::GenerateOptions {
            thinking_enabled: true,
            max_tokens: COMMIT_MESSAGE_MAX_TOKENS,
            ..Default::default()
        },
        "Git commit message",
        crate::chat::model::GenerateRequestContext::new(conversation_id.as_deref(), Some("")),
    );
    let output = timeout(
        Duration::from_secs(45),
        crate::chat::model::generate_via_stream_collect(&state, &provider, retry_attempts, request),
    )
    .await
    .map_err(|_| "生成提交信息超时。".to_string())?
    .map_err(|err| format!("生成提交信息失败：{err}"))?;
    sanitize_commit_message(&output.text).ok_or_else(|| "模型没有返回提交信息。".to_string())
}

#[cfg(test)]
mod tests {
    use super::sanitize_commit_message;

    #[test]
    fn commit_message_strips_wrappers_and_keeps_body() {
        assert_eq!(
            sanitize_commit_message(
                "```\nCommit message: Fix login redirect.\n\nKeep the return URL.\n```"
            ),
            Some("Fix login redirect\n\nKeep the return URL.".to_string())
        );
        assert_eq!(sanitize_commit_message("  \n"), None);
        let long = "a".repeat(100);
        assert_eq!(sanitize_commit_message(&long).unwrap().len(), 72);
    }
}
