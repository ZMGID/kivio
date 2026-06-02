use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::api::call_vision_api;
use crate::settings::{persist_settings, ExplainMessage};
use crate::state::AppState;

use super::storage::{
    delete_conversation as delete_conv, get_conversations as get_convs, load_conversation,
    save_conversation,
};
use super::{ChatMessage, Conversation};

/// 获取对话列表
#[tauri::command]
pub(crate) fn chat_get_conversations(
    app: AppHandle,
    offset: usize,
    limit: usize,
    folder: Option<String>,
) -> Result<serde_json::Value, String> {
    let conversations = get_convs(&app, offset, limit, folder)?;
    Ok(serde_json::json!({
        "success": true,
        "conversations": conversations,
    }))
}

/// 获取对话详情
#[tauri::command]
pub(crate) fn chat_get_conversation(
    app: AppHandle,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let conversation = load_conversation(&app, &conversation_id)?;
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 创建新对话
#[tauri::command]
pub(crate) fn chat_create_conversation(
    app: AppHandle,
    state: State<AppState>,
    provider_id: Option<String>,
    model: Option<String>,
    folder: Option<String>,
) -> Result<serde_json::Value, String> {
    let settings = state.settings_read().clone();

    // 使用提供的 provider/model，或者回退到默认配置
    let provider_id = provider_id.unwrap_or_else(|| {
        if !settings.chat_provider_id.is_empty() {
            settings.chat_provider_id.clone()
        } else if !settings.lens.provider_id.is_empty() {
            settings.lens.provider_id.clone()
        } else {
            settings.translator_provider_id.clone()
        }
    });

    let model = model.unwrap_or_else(|| {
        if !settings.chat_model.is_empty() {
            settings.chat_model.clone()
        } else if !settings.lens.model.is_empty() {
            settings.lens.model.clone()
        } else {
            settings.translator_model.clone()
        }
    });

    let now = chrono::Local::now().timestamp();
    let conversation = Conversation {
        id: format!("conv_{}", Uuid::new_v4()),
        title: "新对话".to_string(),
        provider_id,
        model,
        messages: vec![],
        created_at: now,
        updated_at: now,
        pinned: false,
        folder,
    };

    save_conversation(&app, &conversation)?;

    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 发送消息
#[tauri::command]
pub(crate) async fn chat_send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    _attachments: Vec<String>, // attachment IDs; Phase 2 will wire image/file handling.
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let settings = state.settings_read().clone();

    // 创建用户消息
    let user_message = ChatMessage {
        id: format!("msg_{}", Uuid::new_v4()),
        role: "user".to_string(),
        content: content.clone(),
        attachments: vec![], // TODO: 处理附件
        reasoning: None,
        timestamp: chrono::Local::now().timestamp(),
    };

    conversation.messages.push(user_message.clone());

    // 转换为 ExplainMessage 格式
    let api_messages: Vec<ExplainMessage> = conversation
        .messages
        .iter()
        .map(|m| ExplainMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();

    // 调用 API（复用 call_vision_api）
    let language = if !settings.lens.default_language.is_empty() {
        settings.lens.default_language.clone()
    } else {
        "zh".to_string()
    };

    let stream_enabled = settings.lens.stream_enabled;
    let thinking_enabled = settings.lens.thinking_enabled;
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        0
    };

    // TODO: 支持附件（image_id）
    let image_id = String::new();

    match call_vision_api(
        &app,
        &state,
        &image_id,
        api_messages,
        &language,
        retry_attempts,
        stream_enabled,
        "answer",
        "chat-stream", // 使用 chat-stream 事件
        Some(&conversation.provider_id),
        Some(&conversation.model),
        None, // system_prompt
        thinking_enabled,
    )
    .await
    {
        Ok(response) => {
            // 创建 AI 回复消息
            let assistant_message = ChatMessage {
                id: format!("msg_{}", Uuid::new_v4()),
                role: "assistant".to_string(),
                content: response,
                attachments: vec![],
                reasoning: None,
                timestamp: chrono::Local::now().timestamp(),
            };

            conversation.messages.push(assistant_message);

            // 自动生成标题（如果是第一次对话）
            if conversation.messages.len() == 2 && conversation.title == "新对话" {
                conversation.title = generate_title(&content);
            }

            conversation.updated_at = chrono::Local::now().timestamp();
            save_conversation(&app, &conversation)?;

            Ok(serde_json::json!({
                "success": true,
                "conversation": conversation,
            }))
        }
        Err(err) => Ok(serde_json::json!({
            "success": false,
            "error": err,
        })),
    }
}

/// 删除对话
#[tauri::command]
pub(crate) fn chat_delete_conversation(
    app: AppHandle,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    delete_conv(&app, &conversation_id)?;
    Ok(serde_json::json!({
        "success": true,
    }))
}

/// 更新对话（标题、置顶、文件夹等）
#[tauri::command]
pub(crate) fn chat_update_conversation(
    app: AppHandle,
    state: State<AppState>,
    conversation_id: String,
    title: Option<String>,
    pinned: Option<bool>,
    folder: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;

    if let Some(t) = title {
        conversation.title = t;
    }
    if let Some(p) = pinned {
        conversation.pinned = p;
    }
    if folder.is_some() {
        conversation.folder = folder;
    }
    let provider_model_changed = provider_id.is_some() || model.is_some();
    if let Some(provider_id) = provider_id {
        conversation.provider_id = provider_id;
    }
    if let Some(model) = model {
        conversation.model = model;
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

    if provider_model_changed {
        let updated_settings = {
            let mut settings = state.settings_write();
            settings.chat_provider_id = conversation.provider_id.clone();
            settings.chat_model = conversation.model.clone();
            settings.clone()
        };
        persist_settings(&app, &updated_settings)?;
    }

    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 生成对话标题（从第一条用户消息）
fn generate_title(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.len() > 30 {
        format!("{}...", &trimmed[..30])
    } else {
        trimmed.to_string()
    }
}
