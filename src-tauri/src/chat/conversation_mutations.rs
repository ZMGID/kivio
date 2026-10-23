//! Existing Kivio history mutation rules, independent of the window and storage owner.
use crate::chat::conversation_data::{
    AgentPlanState, ChatMessage, Conversation, ConversationContextState, ForkOrigin, truncate_chars,
};
use uuid::Uuid;

fn prune_clear_boundaries_if_needed(conversation: &mut Conversation) {
    conversation
        .context_state
        .clear_boundaries
        .retain(|boundary| {
            conversation
                .messages
                .iter()
                .any(|message| message.id == boundary.source_until_message_id)
                || conversation
                    .messages
                    .iter()
                    .any(|message| message.timestamp > boundary.created_at)
        });
}

pub fn mark_summary_stale_if_needed(conversation: &mut Conversation, changed_index: usize) {
    prune_clear_boundaries_if_needed(conversation);
    conversation.invalidate_summary_from(changed_index);
}

pub fn find_message_index(conversation: &Conversation, message_id: &str) -> Result<usize, String> {
    conversation
        .messages
        .iter()
        .position(|m| m.id == message_id)
        .ok_or_else(|| "消息不存在".to_string())
}

/// 只拿掉这一条助手回答。同组其它模型、用户提问和之后的轮次都留着。
/// 组选中若正指向被删的那条，清掉该组记录，续聊回退到剩余回答里的第一条；
/// 选中的是别的回答、或别的组，则不动。
pub fn remove_assistant_answer(
    conversation: &mut Conversation,
    message_id: &str,
) -> Result<(), String> {
    let idx = find_message_index(conversation, message_id)?;
    if conversation.messages[idx].role != "assistant" {
        return Err("仅支持删除助手回复".to_string());
    }
    mark_summary_stale_if_needed(conversation, idx);
    let removed = conversation.messages.remove(idx);
    if let Some(group_id) = removed.group_id.as_deref() {
        if conversation
            .group_selections
            .get(group_id)
            .map(String::as_str)
            == Some(removed.id.as_str())
        {
            conversation.group_selections.remove(group_id);
        }
    }
    Ok(())
}

/// `chat_regenerate_message` 的截断/编辑核心（纯函数，便于单测）：
/// - assistant：截到它之前（`new_content` 无意义 → 报错）。
/// - user + `new_content`：trim 校验非空 → 替换内容（编辑提问；附件保留）→ 保留该条截掉其后。
///   摘要失效用 `idx`（内容变了，覆盖到该条的摘要即失效）。
/// - user 无 `new_content`：孤儿重试，摘要失效用 `idx + 1`，保留该条截掉其后。
pub fn apply_regenerate_truncation(
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

/// Truncating history also rewinds the agent todo list to the last snapshot the
/// retained messages recorded, so it never shows steps from a removed future.
/// External CLIs keep their list: Codex plan updates and ACP todo notifications
/// leave no tool record to rebuild it from.
pub fn restore_todo_state_from_history(conversation: &mut Conversation) {
    if conversation.agent_runtime.is_external() {
        return;
    }
    let restored = crate::chat::todo_history::state_from_messages(
        &conversation.messages,
        &conversation.group_selections,
    );
    if restored.items != conversation.agent_todo_state.items {
        conversation.agent_todo_state = restored;
    }
}

/// 组装分叉分支的消息前缀（纯函数，便于单测）：
/// - 取 `messages[0..=anchor_idx]`（含锚点）。
/// - 若锚点属于某多模型多答组（`group_id = Some(g)`，决策 Q2）：只保留锚点那一列，
///   移除前缀内其余同组兄弟列（组内更晚的列已被切片排除），并把锚点的 `group_id` 置 None
///   转为普通单答，使新对话该轮成为干净的线性单答。
/// - 保留原 message id（跨对话无需唯一，`group_selections` 引用因此仍有效）。
pub fn build_fork_messages(messages: &[ChatMessage], anchor_idx: usize) -> Vec<ChatMessage> {
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

pub fn build_fork_messages_before_anchor(
    messages: &[ChatMessage],
    anchor_idx: usize,
) -> Vec<ChatMessage> {
    anchor_idx
        .checked_sub(1)
        .map(|index| build_fork_messages(messages, index))
        .unwrap_or_default()
}

/// Clone the canonical prefix and conversation configuration for an independent branch.
/// The caller owns copying referenced files and persisting the returned record.
pub fn build_fork_conversation(
    source: &Conversation,
    message_id: &str,
    exclude_anchor: bool,
) -> Result<Conversation, String> {
    let anchor_idx = find_message_index(source, message_id)?;

    let now = chrono::Local::now().timestamp();
    let new_id = format!("conv_{}", Uuid::new_v4());
    let messages = if exclude_anchor {
        build_fork_messages_before_anchor(&source.messages, anchor_idx)
    } else {
        build_fork_messages(&source.messages, anchor_idx)
    };

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

    // The branch keeps the todo list as it stood at the fork point.
    let agent_todo_state =
        crate::chat::todo_history::state_from_messages(&messages, &group_selections);
    let conversation = Conversation {
        id: new_id,
        revision: 0,
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
        archived: false,
        folder: source.folder.clone(),
        project_id: source.project_id.clone(),
        set_id: source.set_id.clone(),
        context_state: ConversationContextState::default(),
        agent_todo_state,
        agent_plan_state: AgentPlanState::default(),
        goal_state: None,
        knowledge_base_ids: source.knowledge_base_ids.clone(),
        force_knowledge_search: source.force_knowledge_search,
        additional_directories: source.additional_directories.clone(),
        thinking_level: source.thinking_level.clone(),
        web_search_mode: source.web_search_mode,
        reply_models: source.reply_models.clone(),
        group_selections,
        forked_from: Some(ForkOrigin {
            conversation_id: source.id.clone(),
            message_id: source.messages[anchor_idx].id.clone(),
            title: source.title.clone(),
        }),
    };

    Ok(conversation)
}

/// Edit the visible final answer and its canonical replay together.
pub fn edit_assistant_answer(
    conversation: &mut Conversation,
    message_id: &str,
    content: &str,
) -> Result<(), String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("消息内容不能为空".into());
    }
    let idx = find_message_index(conversation, message_id)?;
    if conversation.messages[idx].role != "assistant" {
        return Err("仅支持编辑助手回复".into());
    }
    mark_summary_stale_if_needed(conversation, idx);
    crate::chat::message_content::replace_final_text_segments_for_edit(
        &mut conversation.messages[idx],
        content,
    );
    conversation.messages[idx].timestamp = chrono::Local::now().timestamp();
    Ok(())
}

/// Remove the selected user turn and the following history, returning its draft.
/// External hosts must realign their native session before invoking this mutation.
pub fn rewind_to_user_message(
    conversation: &mut Conversation,
    message_id: &str,
) -> Result<String, String> {
    let idx = find_message_index(conversation, message_id)?;
    if conversation.messages[idx].role != "user" {
        return Err("仅支持回到用户提问".into());
    }
    let content = conversation.messages[idx].content.clone();
    mark_summary_stale_if_needed(conversation, idx);
    conversation.messages.truncate(idx);
    let existing_ids: std::collections::HashSet<String> =
        conversation.messages.iter().map(|m| m.id.clone()).collect();
    conversation
        .group_selections
        .retain(|_, selected| existing_ids.contains(selected));
    restore_todo_state_from_history(conversation);
    Ok(content)
}
