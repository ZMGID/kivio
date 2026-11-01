use std::future::Future;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::chat::commands::send::{send_user_message_when_idle, QueuedSendOutcome};
use crate::chat::repository::{repository, repository_error};
use crate::chat::storage::ConversationLibraryQuery;
use crate::chat::types::{
    AgentPlanMode, AgentRuntimeConfig, AgentRuntimeKind, ChatAssistantSnapshot, ChatMessage,
    Conversation,
};
use crate::state::AppState;

const BRAIN_ID: &str = "asst_voice_orchestrator";
const BRAIN_PROMPT: &str = "你是 Kivio 的语音编排助手。输入是可能有噪声的语音转录。使用 kivio_* 工具了解和管理项目、对话与代理；编码和文件修改必须委派给其他对话，不要亲自执行。彼此独立的任务应并行创建多个对话，不要逐个等待。只报告工具验证过的状态；用用户的语言回复一至四句简短、适合朗读的话，不使用 Markdown，也不要朗读路径。";
const REFRESH_EVENT: &str = "chat-conversations-changed";
const SEND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
static BRAIN_CREATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn is_brain_snapshot(snapshot: &ChatAssistantSnapshot) -> bool {
    snapshot.id == BRAIN_ID
}

pub(crate) fn is_brain_conversation(conversation: &Conversation) -> bool {
    conversation
        .assistant_snapshot
        .as_ref()
        .is_some_and(is_brain_snapshot)
}

fn brain_snapshot() -> ChatAssistantSnapshot {
    ChatAssistantSnapshot {
        id: BRAIN_ID.into(),
        name: "语音编排".into(),
        description: "通过语音管理 Kivio 对话与代理".into(),
        source: "builtin".into(),
        system_prompt: BRAIN_PROMPT.into(),
        provider_id: String::new(),
        model: String::new(),
        mcp_server_ids: Vec::new(),
        skill_ids: Vec::new(),
    }
}

pub(crate) async fn ensure_brain_conversation(
    app: &AppHandle,
    provider_id: Option<String>,
    model: Option<String>,
) -> Result<String, String> {
    let _creation = BRAIN_CREATION.lock().await;
    let state = app.state::<AppState>();
    let (default_provider, default_model) = state.settings_read().effective_chat_model();
    let provider_id = nonempty(provider_id.as_deref()).unwrap_or(default_provider);
    let model = nonempty(model.as_deref()).unwrap_or(default_model);
    let mut latest: Option<(String, i64)> = None;
    for shelf in ["all", "archived"] {
        let mut offset = 0;
        loop {
            let page = repository(app)
                .query_library(
                    app,
                    ConversationLibraryQuery {
                        offset,
                        limit: 200,
                        shelf: shelf.into(),
                        assistant_id: Some(BRAIN_ID.into()),
                        ..Default::default()
                    },
                )
                .await
                .map_err(repository_error)?;
            let count = page.items.len();
            for hit in page.items {
                if latest
                    .as_ref()
                    .is_none_or(|(_, updated)| hit.item.updated_at > *updated)
                {
                    latest = Some((hit.item.id, hit.item.updated_at));
                }
            }
            offset += count;
            if count == 0 || offset >= page.total {
                break;
            }
        }
    }

    if let Some((id, _)) = latest {
        reuse_brain_conversation(state.inner(), &id, async {
            repository(app)
                .mutate(app, &id, |conversation| {
                    configure_brain(conversation, provider_id, model)
                })
                .await
                .map(|_| ())
                .map_err(repository_error)
        })
        .await?;
        return Ok(id);
    }

    let conversation = crate::chat::commands::catalog::create_chat_conversation_internal(
        app,
        state.inner(),
        Some(provider_id),
        Some(model),
        None,
        None,
        None,
        None,
        false,
    )
    .await?;
    repository(app)
        .mutate(app, &conversation.id, |conversation| {
            conversation.title = "语音编排".into();
            conversation.assistant_id = Some(BRAIN_ID.into());
            conversation.assistant_snapshot = Some(brain_snapshot());
            conversation.agent_runtime = AgentRuntimeConfig::default();
            Ok(())
        })
        .await
        .map_err(repository_error)?;
    let _ = app.emit(REFRESH_EVENT, ());
    Ok(conversation.id)
}

fn configure_brain(
    conversation: &mut Conversation,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    if !is_brain_conversation(conversation) {
        return Err("语音编排对话已被修改，请重试".into());
    }
    conversation.provider_id = provider_id;
    conversation.model = model;
    conversation.agent_runtime = AgentRuntimeConfig::default();
    conversation.agent_plan_state =
        crate::chat::plan::with_mode(&conversation.agent_plan_state, AgentPlanMode::Act);
    conversation.archived = false;
    Ok(())
}

async fn reuse_brain_conversation(
    state: &AppState,
    id: &str,
    update: impl Future<Output = Result<(), String>>,
) -> Result<(), String> {
    let _reservation = crate::chat::commands::ChatSendReservation::try_acquire(state, id)
        .ok_or_else(|| "语音编排对话正在生成中，请稍后再试".to_string())?;
    update.await
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_string<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} 不能为空"))
}

fn limit_arg(arguments: &Value, default: usize) -> Result<usize, String> {
    match arguments.get("limit") {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| "limit 必须是非负整数".into()),
    }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.trim().chars();
    let mut result: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() && max_chars > 0 {
        result.pop();
        result.push('…');
    }
    result
}

fn answer_for_message(conversation: &Conversation, user_id: &str) -> Result<String, String> {
    let start = conversation
        .messages
        .iter()
        .position(|message| message.id == user_id)
        .ok_or_else(|| "消息未发送到对话".to_string())?;
    let mut first_success = None;
    let mut last_failure = None;
    let mut last_answer = None;
    for answer in conversation.messages[start + 1..]
        .iter()
        .take_while(|message| message.role != "user")
        .filter(|message| message.role == "assistant")
    {
        last_answer = Some(answer);
        if answer.degraded.is_some()
            || answer
                .stream_outcome
                .as_deref()
                .is_some_and(|outcome| outcome != "completed")
        {
            last_failure = Some(answer);
            continue;
        }
        let content = answer.content.trim();
        if content.is_empty() {
            continue;
        }
        if answer
            .group_id
            .as_ref()
            .and_then(|group| conversation.group_selections.get(group))
            .is_some_and(|selected| selected == &answer.id)
        {
            return Ok(content.to_string());
        }
        if first_success.is_none() {
            first_success = Some(content);
        }
    }
    if let Some(content) = first_success {
        return Ok(content.to_string());
    }
    let answer = last_failure
        .or(last_answer)
        .ok_or_else(|| "回复未完成或已取消".to_string())?;
    if let Some(degraded) = answer.degraded.as_ref() {
        return Err(degraded.reason.clone());
    }
    if answer
        .stream_outcome
        .as_deref()
        .is_some_and(|outcome| outcome != "completed")
    {
        return Err(if answer.content.trim().is_empty() {
            "回复失败或已取消".into()
        } else {
            truncate_text(&answer.content, 1500)
        });
    }
    let content = answer.content.trim();
    if content.is_empty() {
        return Err("回复为空".into());
    }
    Ok(content.to_string())
}

async fn send_and_answer(app: &AppHandle, id: &str, request: String) -> Result<String, String> {
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let outcome =
        send_user_message_when_idle(app, id, request, receiver, &|| true, &|| {}, &|| {}).await;
    drop(sender);
    let user_id = match outcome? {
        QueuedSendOutcome::Sent { user_message_id } => user_message_id,
        QueuedSendOutcome::Cancelled => return Err("发送已取消".into()),
    };
    let conversation = repository(app)
        .get(app, id)
        .await
        .map_err(repository_error)?;
    answer_for_message(&conversation, &user_id)
}

pub(crate) async fn delegate(
    app: &AppHandle,
    conversation_id: &str,
    request: String,
) -> Result<String, String> {
    if request.trim().is_empty() {
        return Err("语音委派内容不能为空".into());
    }
    let conversation = repository(app)
        .get(app, conversation_id)
        .await
        .map_err(repository_error)?;
    if !is_brain_conversation(&conversation) {
        return Err("指定的对话不是语音编排对话".into());
    }
    let response = send_and_answer(app, conversation_id, request).await?;
    Ok(truncate_text(&response, 1500))
}

async fn available_agents(app: &AppHandle) -> Result<Vec<Value>, String> {
    let detected = crate::external_agents::commands::chat_detect_external_agents(
        app.clone(),
        app.state::<AppState>(),
        None,
        None,
    )
    .await?;
    let agents = detected
        .get("agents")
        .and_then(Value::as_array)
        .ok_or_else(|| "无法获取外部 CLI 代理列表".to_string())?;
    let mut available = vec![json!({"id": "kivio", "name": "Kivio Agent"})];
    available.extend(
        agents
            .iter()
            .filter(|agent| {
                agent.get("available").and_then(Value::as_bool) == Some(true)
                    && agent.get("disabled").and_then(Value::as_bool) != Some(true)
            })
            .filter_map(|agent| {
                Some(
                    json!({"id": agent.get("id")?.as_str()?, "name": agent.get("name")?.as_str()?}),
                )
            }),
    );
    Ok(available)
}

async fn list_conversations(app: &AppHandle, arguments: &Value) -> Result<Value, String> {
    let limit = limit_arg(arguments, 20)?;
    let project_id = nonempty(arguments.get("project_id").and_then(Value::as_str));
    let folder = project_id
        .as_deref()
        .map(|id| crate::chat::storage::find_project_by_id(app, id).map(|project| project.name))
        .transpose()?;
    let state = app.state::<AppState>();
    let mut conversations = Vec::new();
    let mut offset = 0;
    while conversations.len() < limit {
        let page = repository(app)
            .list(
                app,
                offset,
                (limit - conversations.len()).min(200),
                folder.clone(),
                project_id.clone(),
                None,
            )
            .await
            .map_err(repository_error)?;
        let count = page.len();
        for item in page {
            if item.assistant_id.as_deref() == Some(BRAIN_ID) {
                continue;
            }
            let runtime = if item.agent_runtime.is_external() {
                item.agent_runtime
                    .external_agent_id
                    .as_deref()
                    .unwrap_or("kivio")
            } else {
                "kivio"
            };
            let project = if item.project_id.is_some() || item.folder.is_some() {
                Some(json!({"id": item.project_id, "name": item.folder}))
            } else {
                None
            };
            conversations.push(json!({
                "id": item.id,
                "title": item.title,
                "project": project,
                "runtime": runtime,
                "running": state.chat_runtime().has_active_reply(&item.id),
                "updated_at": item.updated_at,
            }));
        }
        offset += count;
        if count == 0 {
            break;
        }
    }
    Ok(json!({"conversations": conversations}))
}

async fn read_conversation(app: &AppHandle, arguments: &Value) -> Result<Value, String> {
    let id = required_string(arguments, "conversation_id")?;
    let limit = limit_arg(arguments, 6)?;
    let conversation = repository(app)
        .get(app, id)
        .await
        .map_err(repository_error)?;
    let messages: Vec<Value> = conversation
        .messages
        .iter()
        .skip(conversation.messages.len().saturating_sub(limit))
        .map(|message| json!({"role": message.role, "text": truncate_text(&message.content, 2000)}))
        .collect();
    Ok(
        json!({"conversation_id": id, "running": app.state::<AppState>().chat_runtime().has_active_reply(id), "messages": messages}),
    )
}

async fn start_send(
    app: &AppHandle,
    id: String,
    text: String,
    wait: bool,
) -> Result<Value, String> {
    let task_app = app.clone();
    let task_id = id.clone();
    let mut task = tokio::spawn(async move {
        let result = send_and_answer(&task_app, &task_id, text).await;
        if let Err(error) = &result {
            eprintln!("语音编排发送失败 ({task_id}): {error}");
        }
        let _ = task_app.emit(REFRESH_EVENT, ());
        result
    });
    let _ = app.emit(REFRESH_EVENT, ());
    if !wait {
        return Ok(json!({"conversation_id": id, "status": "queued"}));
    }
    match tokio::time::timeout(SEND_TIMEOUT, &mut task).await {
        Ok(Ok(Ok(response))) => {
            Ok(json!({"conversation_id": id, "status": "completed", "response": response}))
        }
        Ok(Ok(Err(error))) => Err(error),
        Ok(Err(error)) => Err(format!("发送任务失败：{error}")),
        Err(_) => Ok(json!({"conversation_id": id, "status": "running"})),
    }
}

async fn create_conversation(app: &AppHandle, arguments: &Value) -> Result<Value, String> {
    let prompt = required_string(arguments, "prompt")?.to_string();
    let title = nonempty(arguments.get("title").and_then(Value::as_str));
    let agent =
        nonempty(arguments.get("agent").and_then(Value::as_str)).unwrap_or_else(|| "kivio".into());
    let provider_id = nonempty(arguments.get("provider_id").and_then(Value::as_str));
    let model = nonempty(arguments.get("model").and_then(Value::as_str));
    let project_id = nonempty(arguments.get("project_id").and_then(Value::as_str));
    let wait = arguments
        .get("wait")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if agent != "kivio"
        && !available_agents(app)
            .await?
            .iter()
            .any(|item| item.get("id").and_then(Value::as_str) == Some(agent.as_str()))
    {
        return Err(format!("外部 CLI 代理不可用：{agent}"));
    }
    let state = app.state::<AppState>();
    let conversation = crate::chat::commands::catalog::create_chat_conversation_internal(
        app,
        state.inner(),
        provider_id,
        model.clone(),
        None,
        project_id,
        None,
        None,
        false,
    )
    .await?;
    let runtime = if agent == "kivio" {
        AgentRuntimeConfig::default()
    } else {
        AgentRuntimeConfig {
            kind: AgentRuntimeKind::External,
            external_agent_id: Some(agent),
            external_model: model,
            ..Default::default()
        }
    };
    crate::external_agents::commands::set_agent_runtime(app, &conversation.id, runtime).await?;
    if let Some(title) = title {
        repository(app)
            .mutate(app, &conversation.id, |conversation| {
                conversation.title = title;
                Ok(())
            })
            .await
            .map_err(repository_error)?;
    }
    start_send(app, conversation.id, prompt, wait).await
}

pub(crate) async fn call_tool(
    app: &AppHandle,
    name: &str,
    arguments: &Value,
) -> Result<String, String> {
    let result = match name {
        "kivio_list_agents" => json!({"agents": available_agents(app).await?}),
        "kivio_list_projects" => {
            let projects: Vec<Value> = crate::chat::storage::get_projects(app)?.into_iter()
                .map(|project| json!({"id": project.id, "name": project.name, "root_path": project.root_path}))
                .collect();
            json!({"projects": projects})
        }
        "kivio_list_conversations" => list_conversations(app, arguments).await?,
        "kivio_read_conversation" => read_conversation(app, arguments).await?,
        "kivio_create_conversation" => create_conversation(app, arguments).await?,
        "kivio_send_message" => {
            let id = required_string(arguments, "conversation_id")?.to_string();
            let text = required_string(arguments, "text")?.to_string();
            let conversation = repository(app)
                .get(app, &id)
                .await
                .map_err(repository_error)?;
            if is_brain_conversation(&conversation) {
                return Err("不能向语音编排对话发送消息".into());
            }
            let wait = arguments
                .get("wait")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            start_send(app, id, text, wait).await?
        }
        "kivio_interrupt" => {
            let id = required_string(arguments, "conversation_id")?;
            repository(app)
                .get(app, id)
                .await
                .map_err(repository_error)?;
            crate::chat::commands::interaction::cancel_stream(
                app,
                app.state::<AppState>().inner(),
                id,
            )
            .await?;
            json!({"conversation_id": id, "interrupted": true})
        }
        _ => return Err(format!("未知的语音编排工具：{name}")),
    };
    Ok(result.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, role: &str, content: &str, outcome: Option<&str>) -> ChatMessage {
        serde_json::from_value(json!({"id": id, "role": role, "content": content, "timestamp": 1, "stream_outcome": outcome})).unwrap()
    }

    fn test_conversation(messages: Vec<ChatMessage>) -> Conversation {
        serde_json::from_value(json!({
            "id": "voice-conversation",
            "title": "voice",
            "provider_id": "old-provider",
            "model": "old-model",
            "messages": messages,
            "created_at": 1,
            "updated_at": 1
        }))
        .unwrap()
    }

    fn grouped_message(id: &str, role: &str, content: &str, outcome: Option<&str>) -> ChatMessage {
        let mut message = message(id, role, content, outcome);
        message.group_id = Some("group".into());
        message
    }

    fn degraded_message(id: &str, reason: &str) -> ChatMessage {
        let mut answer = grouped_message(id, "assistant", "partial answer", Some("completed"));
        answer.degraded = Some(
            serde_json::from_value(json!({
                "kind": "unknown",
                "reason": reason,
                "text": reason
            }))
            .unwrap(),
        );
        answer
    }

    #[tokio::test]
    async fn busy_brain_reconnect_reports_error_and_retry_applies_model() {
        let state = crate::state::test_app_state();
        let id = "voice-conversation";
        let reservation =
            crate::chat::commands::ChatSendReservation::try_acquire(&state, id).unwrap();
        let mut brain = test_conversation(Vec::new());
        brain.assistant_snapshot = Some(brain_snapshot());
        brain.archived = true;

        let result = reuse_brain_conversation(&state, id, async {
            configure_brain(&mut brain, "new-provider".into(), "new-model".into())
        })
        .await;
        assert!(result.unwrap_err().contains("正在生成中"));
        assert_eq!(brain.provider_id, "old-provider");
        assert_eq!(brain.model, "old-model");
        assert!(brain.archived);

        drop(reservation);
        let result = reuse_brain_conversation(&state, id, async {
            assert!(crate::chat::commands::ChatSendReservation::try_acquire(&state, id).is_none());
            configure_brain(&mut brain, "new-provider".into(), "new-model".into())
        })
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(brain.provider_id, "new-provider");
        assert_eq!(brain.model, "new-model");
        assert!(!brain.archived);
        assert!(crate::chat::commands::ChatSendReservation::try_acquire(&state, id).is_some());
    }

    #[tokio::test]
    async fn brain_reconnect_propagates_update_error_and_releases_reservation() {
        let state = crate::state::test_app_state();
        let id = "voice-conversation";
        let result =
            reuse_brain_conversation(&state, id, async { Err("保存失败".to_string()) }).await;
        assert_eq!(result.unwrap_err(), "保存失败");

        let mut changed = test_conversation(Vec::new());
        let result = reuse_brain_conversation(&state, id, async {
            configure_brain(&mut changed, "new-provider".into(), "new-model".into())
        })
        .await;
        assert_eq!(result.unwrap_err(), "语音编排对话已被修改，请重试");
        assert_eq!(changed.provider_id, "old-provider");
        assert_eq!(changed.model, "old-model");
        assert!(crate::chat::commands::ChatSendReservation::try_acquire(&state, id).is_some());
    }

    #[test]
    fn truncation_counts_unicode_characters_including_ellipsis() {
        assert_eq!(truncate_text("  一二三  ", 3), "一二三");
        assert_eq!(truncate_text("一二三四", 3), "一二…");
        assert_eq!(truncate_text("abcd", 0), "");
    }

    #[test]
    fn list_limits_default_and_reject_invalid_values() {
        assert_eq!(limit_arg(&json!({}), 20).unwrap(), 20);
        assert_eq!(limit_arg(&json!({"limit": 0}), 20).unwrap(), 0);
        assert_eq!(limit_arg(&json!({"limit": 7}), 20).unwrap(), 7);
        assert!(limit_arg(&json!({"limit": -1}), 20).is_err());
    }

    #[test]
    fn reply_is_bound_to_its_user_message_and_failed_replies_are_errors() {
        let conversation = test_conversation(vec![
            message("u1", "user", "same", None),
            message("a1", "assistant", " first ", Some("completed")),
            message("u2", "user", "same", None),
            message("a2", "assistant", "cancelled", Some("cancelled")),
        ]);
        assert_eq!(answer_for_message(&conversation, "u1").unwrap(), "first");
        assert_eq!(
            answer_for_message(&conversation, "u2").unwrap_err(),
            "cancelled"
        );
        assert_eq!(
            answer_for_message(&conversation, "missing").unwrap_err(),
            "消息未发送到对话"
        );
    }

    #[test]
    fn partial_fan_out_prefers_selected_success_then_first_success() {
        let mut conversation = test_conversation(vec![
            grouped_message("u1", "user", "question", None),
            grouped_message("failed", "assistant", "provider failed", Some("error")),
            grouped_message("blank", "assistant", "  ", Some("completed")),
            degraded_message("degraded", "provider degraded"),
            grouped_message("success1", "assistant", " first answer ", Some("completed")),
            grouped_message("success2", "assistant", "second answer", Some("completed")),
            grouped_message("cancelled", "assistant", "cancelled", Some("cancelled")),
        ]);

        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap(),
            "first answer"
        );
        conversation
            .group_selections
            .insert("group".into(), "success2".into());
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap(),
            "second answer"
        );
        conversation
            .group_selections
            .insert("group".into(), "failed".into());
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap(),
            "first answer"
        );

        conversation.messages[4].stream_outcome = None;
        conversation
            .group_selections
            .insert("group".into(), "success1".into());
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap(),
            "first answer"
        );
    }

    #[test]
    fn all_failed_answers_preserve_error_cancel_and_degradation() {
        let mut conversation = test_conversation(vec![
            grouped_message("u1", "user", "question", None),
            grouped_message("first", "assistant", "first error", Some("error")),
            grouped_message(
                "last",
                "assistant",
                "specific provider error",
                Some("error"),
            ),
        ]);
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap_err(),
            "specific provider error"
        );

        conversation.messages.push(grouped_message(
            "blank",
            "assistant",
            "  ",
            Some("completed"),
        ));
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap_err(),
            "specific provider error"
        );
        conversation.messages.push(grouped_message(
            "cancelled",
            "assistant",
            "",
            Some("cancelled"),
        ));
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap_err(),
            "回复失败或已取消"
        );
        conversation
            .messages
            .push(degraded_message("degraded", "specific degraded reason"));
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap_err(),
            "specific degraded reason"
        );

        let only_blank = test_conversation(vec![
            message("u1", "user", "question", None),
            message("blank", "assistant", " \n", Some("completed")),
        ]);
        assert_eq!(
            answer_for_message(&only_blank, "u1").unwrap_err(),
            "回复为空"
        );
    }

    #[test]
    fn answer_selection_never_crosses_next_user_turn() {
        let mut conversation = test_conversation(vec![
            grouped_message("u1", "user", "first", None),
            grouped_message("a1", "assistant", "first answer", Some("completed")),
            grouped_message("u2", "user", "second", None),
            grouped_message("error2", "assistant", "second failed", Some("error")),
            grouped_message("u3", "user", "third", None),
            grouped_message("a3", "assistant", "third answer", Some("completed")),
        ]);
        conversation
            .group_selections
            .insert("group".into(), "a3".into());
        assert_eq!(
            answer_for_message(&conversation, "u1").unwrap(),
            "first answer"
        );
        assert_eq!(
            answer_for_message(&conversation, "u2").unwrap_err(),
            "second failed"
        );
        assert_eq!(
            answer_for_message(&conversation, "u3").unwrap(),
            "third answer"
        );

        let empty = test_conversation(vec![
            message("u1", "user", "first", None),
            message("u2", "user", "second", None),
            message("a2", "assistant", "later answer", None),
        ]);
        assert_eq!(
            answer_for_message(&empty, "u1").unwrap_err(),
            "回复未完成或已取消"
        );
        assert_eq!(answer_for_message(&empty, "u2").unwrap(), "later answer");
    }
}
