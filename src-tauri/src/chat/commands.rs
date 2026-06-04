use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_shell::ShellExt;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::api::extract_status_code;
use crate::apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
use crate::chat::model::{
    generate_request_from_openai_messages, model_messages_from_openai_messages,
    openai_messages_from_model_messages, pending_tool_calls_from_openai_message,
    AnthropicMessagesProvider, AppleLocalProvider, GenerateOptions, GenerateOutput,
    LanguageModelProvider, MessagePart, ModelError, ModelMessage, ModelRole, OpenAiChatProvider,
    PendingToolCall, StreamPart, StreamSink,
};
use crate::mcp::types::ChatToolArtifact;
use crate::mcp::{self, ChatToolDefinition};
use crate::settings::{
    chat_no_think_instruction, default_chat_system_prompt, persist_settings, ProviderApiFormat,
    Settings,
};
use crate::skills;
use crate::state::AppState;

use super::storage::{
    archive_assistant, assistant_snapshot, conversation_attachments_dir, create_assistant,
    create_project, delete_conversation as delete_conv, delete_project, duplicate_assistant,
    get_assistants, get_conversations as get_convs, get_projects, load_conversation,
    save_conversation, update_assistant, update_project,
};
use super::{
    Attachment, ChatAssistant, ChatAssistantSnapshot, ChatMessage, ContextUsageSegment,
    Conversation, ConversationContextState, ConversationContextSummary, ToolCallRecord,
    ToolCallStatus,
};

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
    assistant_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let settings = state.settings_read().clone();
    let assistant_snapshot = assistant_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| assistant_snapshot(&app, id))
        .transpose()?;

    // 使用提供的 provider/model，或者回退到默认模型配置。
    let (default_provider_id, default_model) = settings.effective_chat_model();
    let provider_id = provider_id
        .and_then(non_empty_string)
        .or_else(|| {
            assistant_snapshot
                .as_ref()
                .and_then(|assistant| non_empty_string(assistant.provider_id.clone()))
        })
        .unwrap_or(default_provider_id);
    let model = model
        .and_then(non_empty_string)
        .or_else(|| {
            assistant_snapshot
                .as_ref()
                .and_then(|assistant| non_empty_string(assistant.model.clone()))
        })
        .unwrap_or(default_model);
    let folder = folder.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let now = chrono::Local::now().timestamp();
    let conversation = Conversation {
        id: format!("conv_{}", Uuid::new_v4()),
        title: "新对话".to_string(),
        provider_id,
        model,
        messages: vec![],
        active_skill_id: assistant_snapshot
            .as_ref()
            .and_then(|assistant| assistant.skill_id.clone()),
        assistant_id: assistant_snapshot
            .as_ref()
            .map(|assistant| assistant.id.clone()),
        assistant_snapshot,
        created_at: now,
        updated_at: now,
        pinned: false,
        folder,
        context_state: ConversationContextState::default(),
    };

    save_conversation(&app, &conversation)?;

    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[tauri::command]
pub(crate) fn chat_get_assistants(app: AppHandle) -> Result<serde_json::Value, String> {
    let assistants = get_assistants(&app, false)?;
    Ok(serde_json::json!({
        "success": true,
        "assistants": assistants,
    }))
}

#[tauri::command]
pub(crate) fn chat_create_assistant(
    app: AppHandle,
    assistant: ChatAssistant,
) -> Result<serde_json::Value, String> {
    let assistant = create_assistant(&app, assistant)?;
    Ok(serde_json::json!({
        "success": true,
        "assistant": assistant,
    }))
}

#[tauri::command]
pub(crate) fn chat_update_assistant(
    app: AppHandle,
    assistant: ChatAssistant,
) -> Result<serde_json::Value, String> {
    let assistant = update_assistant(&app, assistant)?;
    Ok(serde_json::json!({
        "success": true,
        "assistant": assistant,
    }))
}

#[tauri::command]
pub(crate) fn chat_duplicate_assistant(
    app: AppHandle,
    assistant_id: String,
) -> Result<serde_json::Value, String> {
    let assistant = duplicate_assistant(&app, &assistant_id)?;
    Ok(serde_json::json!({
        "success": true,
        "assistant": assistant,
    }))
}

#[tauri::command]
pub(crate) fn chat_delete_assistant(
    app: AppHandle,
    assistant_id: String,
) -> Result<serde_json::Value, String> {
    archive_assistant(&app, &assistant_id)?;
    Ok(serde_json::json!({
        "success": true,
    }))
}

#[tauri::command]
pub(crate) fn chat_get_projects(app: AppHandle) -> Result<serde_json::Value, String> {
    let projects = get_projects(&app)?;
    Ok(serde_json::json!({
        "success": true,
        "projects": projects,
    }))
}

#[tauri::command]
pub(crate) fn chat_create_project(
    app: AppHandle,
    name: String,
    description: Option<String>,
    color: Option<String>,
) -> Result<serde_json::Value, String> {
    let now = chrono::Local::now().timestamp();
    let project = create_project(
        &app,
        super::ChatProject {
            id: format!("proj_{}", Uuid::new_v4()),
            name,
            description,
            color,
            created_at: now,
            updated_at: now,
        },
    )?;

    Ok(serde_json::json!({
        "success": true,
        "project": project,
    }))
}

#[tauri::command]
pub(crate) fn chat_update_project(
    app: AppHandle,
    project_id: String,
    name: Option<String>,
    description: Option<String>,
    color: Option<String>,
) -> Result<serde_json::Value, String> {
    let project = update_project(&app, &project_id, name, description, color)?;
    Ok(serde_json::json!({
        "success": true,
        "project": project,
    }))
}

#[tauri::command]
pub(crate) fn chat_delete_project(
    app: AppHandle,
    project_id: String,
) -> Result<serde_json::Value, String> {
    delete_project(&app, &project_id)?;
    Ok(serde_json::json!({
        "success": true,
    }))
}

#[tauri::command]
pub(crate) async fn chat_get_context_stats(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let context_state = compute_context_state(&app, &state, &conversation, None, &[]).await?;
    conversation.context_state = context_state.clone();
    save_conversation(&app, &conversation)?;
    Ok(serde_json::json!({
        "success": true,
        "contextState": context_state,
        "conversation": conversation,
    }))
}

#[tauri::command]
pub(crate) async fn chat_compress_context(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    compress_conversation_context(&app, &state, &mut conversation).await?;
    conversation.context_state.warning = None;
    let context_state = compute_context_state(&app, &state, &conversation, None, &[]).await?;
    conversation.context_state = context_state.clone();
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_context_state(&app, &conversation.id, &context_state);
    Ok(serde_json::json!({
        "success": true,
        "contextState": context_state,
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
    attachments: Vec<String>,
    active_skill_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let message_attachments = save_message_attachments(&app, &conversation_id, attachments)?;
    let api_content = compose_user_content_for_api(&content, &message_attachments);
    let title_source = title_source_for_user_message(&content, &message_attachments);
    let last_user_image_paths =
        stored_image_paths_for_attachments(&app, &conversation_id, &message_attachments)?;

    // 创建用户消息
    let user_message = ChatMessage {
        id: format!("msg_{}", Uuid::new_v4()),
        role: "user".to_string(),
        content: content.clone(),
        attachments: message_attachments,
        reasoning: None,
        tool_calls: Vec::new(),
        api_messages: Vec::new(),
        model_messages: Vec::new(),
        active_skill_id: None,
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
                match compress_conversation_context(&app, &state, &mut conversation).await {
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

    match complete_assistant_reply(
        &app,
        &state,
        &mut conversation,
        Some(title_source.as_str()),
        Some(api_content.as_str()),
        &last_user_image_paths,
        forced_skill_id.as_deref(),
    )
    .await
    {
        Ok(()) => Ok(serde_json::json!({
            "success": true,
            "conversation": conversation,
        })),
        Err(err) if err == "cancelled" => Ok(serde_json::json!({
            "success": true,
            "conversation": conversation,
        })),
        Err(err) => {
            rollback_user_message_after_failed_send(
                &app,
                &state,
                &mut conversation,
                &user_message.id,
            )
            .await?;
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

/// 前端 Pyodide 执行完成后回传结果。
#[tauri::command]
pub(crate) fn chat_python_complete(
    state: State<AppState>,
    run_id: String,
    content: String,
    is_error: bool,
    artifacts: Option<Vec<ChatToolArtifact>>,
) -> Result<(), String> {
    let sender = state
        .pending_python_runs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    if let Some(sender) = sender {
        let _ = sender.send(crate::mcp::types::PythonRunResult {
            content,
            is_error,
            artifacts: artifacts.unwrap_or_default(),
        });
    }
    Ok(())
}

const MAX_ATTACHMENT_PREVIEW_BYTES: u64 = 12 * 1024 * 1024;
const FALLBACK_CONTEXT_WINDOW_TOKENS: usize = 200_000;
const AUTO_COMPRESS_RATIO: f32 = 0.85;
const CONTEXT_BLOCK_RATIO: f32 = 1.0;
const KEEP_RECENT_RAW_MESSAGES: usize = 8;
const IMAGE_ATTACHMENT_TOKEN_ESTIMATE: usize = 1_200;

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

fn resolve_attachment_file_path(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: &str,
) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("附件路径为空".to_string());
    }

    if let Some(conversation_id) = conversation_id {
        if path.contains('/') || path.contains('\\') {
            return Err("无效的附件路径".to_string());
        }
        let dir = conversation_attachments_dir(app, conversation_id)?;
        let full = dir.join(path);
        if !full.is_file() {
            return Err(format!("附件不存在: {path}"));
        }
        return Ok(full);
    }

    let full = PathBuf::from(path);
    if !full.is_file() {
        return Err(format!("文件不存在: {path}"));
    }
    Ok(full)
}

fn mime_type_for_attachment(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "md" => "text/markdown",
        _ => "application/octet-stream",
    }
}

fn read_attachment_as_data_url(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("读取附件信息失败: {e}"))?;
    if metadata.len() > MAX_ATTACHMENT_PREVIEW_BYTES {
        return Err("附件过大，无法在界面内预览".to_string());
    }
    let bytes = fs::read(path).map_err(|e| format!("读取附件失败: {e}"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    let mime = mime_type_for_attachment(file_name);
    let encoded = general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn save_message_attachments(
    app: &AppHandle,
    conversation_id: &str,
    attachment_paths: Vec<String>,
) -> Result<Vec<Attachment>, String> {
    let mut attachments = Vec::new();
    if attachment_paths.is_empty() {
        return Ok(attachments);
    }

    let dir = conversation_attachments_dir(app, conversation_id)?;
    for source in attachment_paths {
        let source_path = Path::new(&source);
        if !source_path.is_file() {
            return Err(format!("附件不存在或不是文件: {source}"));
        }

        let id = format!("att_{}", Uuid::new_v4());
        let original_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment");
        let safe_name = sanitize_attachment_name(original_name);
        let stored_name = format!("{}-{}", id, safe_name);
        let dest = dir.join(&stored_name);
        fs::copy(source_path, &dest).map_err(|e| format!("保存附件失败: {e}"))?;

        attachments.push(Attachment {
            id,
            attachment_type: attachment_type_for_name(original_name).to_string(),
            name: original_name.to_string(),
            path: stored_name,
        });
    }

    Ok(attachments)
}

fn sanitize_attachment_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(['.', ' ', '_']).trim();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

fn attachment_type_for_name(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "heic" | "heif" => {
            "image"
        }
        _ => "file",
    }
}

fn attachment_type_label(attachment_type: &str) -> &'static str {
    match attachment_type {
        "image" => "图片",
        _ => "文件",
    }
}

fn compose_user_content_for_api(content: &str, attachments: &[Attachment]) -> String {
    let trimmed = content.trim();
    if attachments.is_empty() {
        return trimmed.to_string();
    }

    let has_images = attachments
        .iter()
        .any(|attachment| attachment.attachment_type == "image");
    let has_files = attachments
        .iter()
        .any(|attachment| attachment.attachment_type != "image");
    let attachment_lines = attachments
        .iter()
        .map(|attachment| {
            format!(
                "- {} ({})",
                attachment.name,
                attachment_type_label(&attachment.attachment_type)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let capability_note = match (has_images, has_files) {
        (true, true) => {
            "图片附件会随本轮请求发送给视觉模型；普通文件当前只保存元数据，模型不会读取文件正文。"
        }
        (true, false) => "图片附件会随本轮请求发送给视觉模型。",
        (false, true) => "当前聊天只保存文件元数据，模型不会读取文件正文。",
        (false, false) => "",
    };
    let attachment_note = format!(
        "[已添加附件]\n{}\n\n注意：{}",
        attachment_lines, capability_note
    );

    if trimmed.is_empty() {
        attachment_note
    } else {
        format!("{trimmed}\n\n{attachment_note}")
    }
}

fn title_source_for_user_message(content: &str, attachments: &[Attachment]) -> String {
    let trimmed = content.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }

    let names = attachments
        .iter()
        .map(|attachment| attachment.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if names.is_empty() {
        "新对话".to_string()
    } else {
        format!("附件: {names}")
    }
}

fn stored_image_paths_for_attachments(
    app: &AppHandle,
    conversation_id: &str,
    attachments: &[Attachment],
) -> Result<Vec<PathBuf>, String> {
    let image_attachments = attachments
        .iter()
        .filter(|attachment| attachment.attachment_type == "image")
        .collect::<Vec<_>>();
    if image_attachments.is_empty() {
        return Ok(Vec::new());
    }

    let dir = conversation_attachments_dir(app, conversation_id)?;
    image_attachments
        .into_iter()
        .map(|attachment| {
            let stored = Path::new(&attachment.path);
            if stored.components().count() != 1 {
                return Err(format!("Invalid attachment path: {}", attachment.path));
            }
            let path = dir.join(stored);
            if !path.is_file() {
                return Err(format!("图片附件不存在: {}", attachment.name));
            }
            Ok(path)
        })
        .collect()
}

async fn complete_assistant_reply(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    title_from_first_user: Option<&str>,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
    active_skill_id: Option<&str>,
) -> Result<(), String> {
    let settings = state.settings_read().clone();
    let provider = settings
        .get_provider(&conversation.provider_id)
        .ok_or_else(|| "Chat provider not found".to_string())?
        .clone();
    let provider_is_apple = provider.base_url == APPLE_INTELLIGENCE_BASE_URL;
    if !provider_is_apple && provider.api_keys.is_empty() {
        return Err(format_chat_missing_api_key_error(&provider.name));
    }
    if conversation.model.trim().is_empty() {
        return Err(chat_missing_model_error());
    }

    let last_user_idx = conversation.messages.iter().rposition(|m| m.role == "user");
    let language = crate::settings::resolve_chat_language(&settings);
    let stream_enabled = settings.chat.stream_enabled;
    let thinking_enabled = settings.chat.thinking_enabled;
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    let run_generation = state.next_chat_generation(&conversation.id);
    let run_id = format!("chat-run-{}-{}", run_generation, Uuid::new_v4());
    let assistant_message_id = format!("msg_{}", Uuid::new_v4());
    let skill_registry =
        skills::build_registry(app, &settings.chat_tools.skill_scan_paths).unwrap_or_default();
    let requested_skill_id = active_skill_id
        .or(conversation.active_skill_id.as_deref())
        .or_else(|| {
            conversation
                .assistant_snapshot
                .as_ref()
                .and_then(|assistant| assistant.skill_id.as_deref())
        });
    let skill_id =
        resolve_forced_skill_id(&settings.chat_tools, &skill_registry, requested_skill_id);
    let active_skill_record = skill_id
        .as_deref()
        .and_then(|id| skill_registry.find(id))
        .cloned();
    let active_skill_detail = skill_id.as_deref().and_then(|id| {
        skills::read_skill_detail(app, &settings.chat_tools.skill_scan_paths, id).ok()
    });
    let mut effective_chat_tools = settings.chat_tools.clone();
    let tools_capable = chat_tools_capable(&provider, &effective_chat_tools);
    let mut tools = list_tools_for_chat(state.inner(), &settings, provider.supports_tools).await;
    apply_assistant_tool_preset(&mut tools, conversation.assistant_snapshot.as_ref());
    if let Some(skill) = active_skill_record.as_ref() {
        apply_active_skill_tool_filter(&mut tools, skill);
    }
    let tools_available = tools_capable && !tools.is_empty();
    apply_skill_fallback_when_tools_unavailable(
        &mut effective_chat_tools,
        skill_id.as_deref(),
        tools_available,
    );
    let available_builtin_tools = available_builtin_tool_names(&tools);
    let system_prompt = build_chat_system_prompt(
        &language,
        !last_user_image_paths.is_empty(),
        thinking_enabled,
        &skill_registry,
        &effective_chat_tools,
        tools_available,
        &available_builtin_tools,
        skill_id.as_deref(),
        active_skill_detail.as_ref(),
        conversation.assistant_snapshot.as_ref(),
        settings.chat.system_prompt.as_str(),
    );

    if provider_is_apple {
        if !last_user_image_paths.is_empty() {
            return Err(
                "Apple Intelligence 暂不支持图片附件，请为 AI 对话配置云端视觉 provider".into(),
            );
        }
        let prompt = build_apple_chat_prompt(
            &system_prompt,
            conversation,
            last_user_idx,
            last_user_api_content,
        );
        let response = tokio::select! {
            result = state.apple_intelligence.call_text(&prompt) => result?,
            _ = wait_for_chat_cancel(state.inner(), &conversation.id, run_generation) => {
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
        emit_chat_stream_delta(
            app,
            &conversation.id,
            &run_id,
            &assistant_message_id,
            &response,
            None,
        );
        emit_chat_stream_done(
            app,
            &conversation.id,
            &run_id,
            &assistant_message_id,
            "done",
            &response,
        );
        push_assistant_message(
            app,
            state,
            &settings,
            conversation,
            assistant_message_id,
            response,
            None,
            Vec::new(),
            Vec::new(),
            skill_id.as_deref(),
            title_from_first_user,
        )
        .await?;
        return Ok(());
    }

    let mut runtime_messages = build_chat_api_messages(
        &system_prompt,
        conversation,
        last_user_idx,
        last_user_api_content,
        last_user_image_paths,
    )?;
    let mut generated_api_messages = Vec::new();
    let mut tool_records = Vec::new();
    let mut planning_reasoning_parts: Vec<String> = Vec::new();
    let max_rounds = settings.chat_tools.max_tool_rounds.max(1);
    let mut provider_tools_unsupported = false;
    let mut tool_planning_finished = false;

    if !tools.is_empty() {
        let mut tried_skill_only_tools = false;
        let mut skill_cache = skills::SkillRunCache::default();
        for round in 0..max_rounds {
            if !state.is_chat_generation_active(&conversation.id, run_generation) {
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
            let planning_result = tokio::select! {
                result = call_chat_completion_message(
                    state,
                    &provider,
                    &conversation.model,
                    runtime_messages.clone(),
                    Some(&tools),
                    retry_attempts,
                    thinking_enabled,
                    "Chat tools planning",
                ) => result,
                _ = wait_for_chat_cancel(state.inner(), &conversation.id, run_generation) => {
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
            let message = match planning_result {
                Ok(message) => message,
                Err(err) if is_tools_unsupported_error(&err) => {
                    let skill_only: Vec<ChatToolDefinition> = tools
                        .iter()
                        .filter(|tool| tool.source == "skill")
                        .cloned()
                        .collect();
                    if !tried_skill_only_tools
                        && skill_only.len() < tools.len()
                        && !skill_only.is_empty()
                    {
                        eprintln!(
                            "Chat provider {} rejected tools; retrying with skill-native tools only",
                            provider.id
                        );
                        tools = skill_only;
                        tried_skill_only_tools = true;
                        continue;
                    }
                    eprintln!(
                        "Chat provider {} rejected tools; falling back to plain chat",
                        provider.id
                    );
                    provider_tools_unsupported = true;
                    break;
                }
                Err(err) => return Err(err),
            };
            let tool_calls = extract_tool_calls(&message);
            if tool_calls.is_empty() {
                tool_planning_finished = true;
                break;
            }
            if let Some(reasoning) = extract_reasoning_content(&message) {
                emit_chat_stream_delta(
                    app,
                    &conversation.id,
                    &run_id,
                    &assistant_message_id,
                    "",
                    Some(&reasoning),
                );
                planning_reasoning_parts.push(reasoning);
            }

            let assistant_message = assistant_api_message_for_tool_calls(&message, &tool_calls);
            runtime_messages.push(assistant_message);
            generated_api_messages.push(runtime_messages.last().cloned().unwrap_or(Value::Null));
            for tool_call in tool_calls {
                let Some(tool) = match_tool_call(&tools, &tool_call.function_name) else {
                    let disabled = disabled_builtin_tool_feedback(&tool_call.function_name);
                    if disabled.is_none() {
                        let error = format!("Unknown tool requested: {}", tool_call.function_name);
                        let record = unknown_tool_record(&tool_call, round + 1, error);
                        emit_chat_tool_record(
                            app,
                            &conversation.id,
                            &run_id,
                            &assistant_message_id,
                            &record,
                        );
                        tool_records.push(record);
                    }
                    let content = disabled.unwrap_or_else(|| {
                        format!("Unknown tool requested: {}", tool_call.function_name)
                    });
                    let tool_message = serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_call.id,
                        "content": content,
                    });
                    runtime_messages.push(tool_message.clone());
                    generated_api_messages.push(tool_message);
                    continue;
                };
                let tool_call_id = tool_call.id.clone();
                if let Some(error) = tool_call.arguments_parse_error.clone() {
                    let record = invalid_tool_arguments_record(&tool_call, tool, round + 1, error);
                    emit_chat_tool_record(
                        app,
                        &conversation.id,
                        &run_id,
                        &assistant_message_id,
                        &record,
                    );
                    let tool_message = serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": "Tool arguments JSON is invalid or incomplete. Retry this tool call with a compact, valid JSON object for arguments.",
                    });
                    runtime_messages.push(tool_message.clone());
                    generated_api_messages.push(tool_message);
                    tool_records.push(record);
                    continue;
                }
                let (record, tool_content) = execute_chat_tool_call(
                    app,
                    state.inner(),
                    &conversation.id,
                    &run_id,
                    &assistant_message_id,
                    run_generation,
                    round + 1,
                    tool,
                    tool_call,
                    &mut skill_cache,
                )
                .await;
                let tool_message = serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": tool_content,
                });
                runtime_messages.push(tool_message.clone());
                generated_api_messages.push(tool_message);
                tool_records.push(record);
            }
        }
        if !provider_tools_unsupported && !tool_planning_finished {
            runtime_messages.push(serde_json::json!({
                "role": "system",
                "content": "已达到本轮工具调用轮次上限。请根据对话中已有的工具返回结果直接回答用户，不要再调用任何工具。"
            }));
        }
    }

    if provider_tools_unsupported {
        apply_provider_tools_fallback(
            &mut runtime_messages,
            &language,
            !last_user_image_paths.is_empty(),
            thinking_enabled,
            &skill_registry,
            &mut effective_chat_tools,
            skill_id.as_deref(),
            active_skill_detail.as_ref(),
            conversation.assistant_snapshot.as_ref(),
            settings.chat.system_prompt.as_str(),
        );
    }

    let (response, reasoning) = if stream_enabled {
        let stream = stream_scoped_chat_completion(
            app,
            state,
            &provider,
            &conversation.model,
            runtime_messages,
            retry_attempts,
            thinking_enabled,
            &conversation.id,
            &run_id,
            &assistant_message_id,
            run_generation,
        )
        .await?;
        if stream.cancelled {
            if !tool_records.is_empty() {
                let stored_content = if stream.content.trim().is_empty() {
                    "已停止生成。".to_string()
                } else {
                    stream.content.clone()
                };
                let final_reasoning_for_api = stream.reasoning.clone();
                let reasoning = merge_reasoning(&planning_reasoning_parts, stream.reasoning);
                if !generated_api_messages.is_empty() {
                    generated_api_messages.push(final_assistant_api_message(
                        &stored_content,
                        final_reasoning_for_api.as_deref(),
                    ));
                }
                push_assistant_message(
                    app,
                    state,
                    &settings,
                    conversation,
                    assistant_message_id,
                    stored_content,
                    reasoning,
                    tool_records,
                    generated_api_messages,
                    skill_id.as_deref(),
                    title_from_first_user,
                )
                .await?;
            }
            return Err("cancelled".to_string());
        }
        let final_reasoning_for_api = stream.reasoning.clone();
        let reasoning = merge_reasoning(&planning_reasoning_parts, stream.reasoning);
        let response = sanitize_assistant_text_response(&stream.content);
        if response.trim().is_empty() {
            return Err(empty_assistant_response_error("Chat stream"));
        }
        if !generated_api_messages.is_empty() {
            generated_api_messages.push(final_assistant_api_message(
                &response,
                final_reasoning_for_api.as_deref(),
            ));
        }
        (response, reasoning)
    } else {
        let message = tokio::select! {
            result = call_chat_completion_message(
                state,
                &provider,
                &conversation.model,
                runtime_messages,
                None,
                retry_attempts,
                thinking_enabled,
                "Chat API",
            ) => result?,
            _ = wait_for_chat_cancel(state.inner(), &conversation.id, run_generation) => {
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
        let response = sanitize_assistant_text_response(
            message
                .get("content")
                .and_then(|content| content.as_str())
                .unwrap_or_default(),
        );
        if response.trim().is_empty() {
            emit_chat_stream_done(
                app,
                &conversation.id,
                &run_id,
                &assistant_message_id,
                "error",
                "",
            );
            return Err(empty_assistant_response_error("Chat API"));
        }
        let reasoning = merge_reasoning(
            &planning_reasoning_parts,
            extract_reasoning_content(&message),
        );
        emit_chat_stream_delta(
            app,
            &conversation.id,
            &run_id,
            &assistant_message_id,
            &response,
            None,
        );
        emit_chat_stream_done(
            app,
            &conversation.id,
            &run_id,
            &assistant_message_id,
            "done",
            &response,
        );
        if !generated_api_messages.is_empty() {
            generated_api_messages.push(message);
        }
        (response, reasoning)
    };

    push_assistant_message(
        app,
        state,
        &settings,
        conversation,
        assistant_message_id,
        response,
        reasoning,
        tool_records,
        generated_api_messages,
        skill_id.as_deref(),
        title_from_first_user,
    )
    .await?;
    Ok(())
}

async fn push_assistant_message(
    app: &AppHandle,
    state: &State<'_, AppState>,
    settings: &Settings,
    conversation: &mut Conversation,
    message_id: String,
    content: String,
    reasoning: Option<String>,
    tool_calls: Vec<ToolCallRecord>,
    api_messages: Vec<Value>,
    active_skill_id: Option<&str>,
    title_from_first_user: Option<&str>,
) -> Result<(), String> {
    let generated_title = if let Some(user_content) = title_from_first_user {
        if conversation.messages.len() == 1 && conversation.title == "新对话" {
            Some(resolve_conversation_title(settings, state, user_content, &content).await)
        } else {
            None
        }
    } else {
        None
    };

    conversation.messages.push(ChatMessage {
        id: message_id,
        role: "assistant".to_string(),
        content: content.clone(),
        attachments: vec![],
        reasoning: reasoning.clone(),
        tool_calls,
        model_messages: assistant_model_messages_for_storage(
            &content,
            reasoning.as_deref(),
            &api_messages,
        ),
        api_messages,
        active_skill_id: active_skill_id.map(|id| id.to_string()),
        timestamp: chrono::Local::now().timestamp(),
    });

    if let Some(title) = generated_title {
        conversation.title = title;
    }

    match compute_context_state(app, state, conversation, None, &[]).await {
        Ok(context_state) => {
            conversation.context_state = context_state.clone();
            emit_chat_context_state(app, &conversation.id, &context_state);
        }
        Err(err) => {
            eprintln!("Context usage estimate failed after assistant reply: {err}");
        }
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(app, conversation)?;
    Ok(())
}

fn assistant_model_messages_for_storage(
    content: &str,
    reasoning: Option<&str>,
    api_messages: &[Value],
) -> Vec<ModelMessage> {
    if !api_messages.is_empty() {
        let canonical = model_messages_from_openai_messages(api_messages.to_vec());
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

async fn resolve_conversation_title(
    settings: &Settings,
    state: &State<'_, AppState>,
    user_content: &str,
    assistant_content: &str,
) -> String {
    match timeout(
        Duration::from_secs(8),
        generate_title_with_model(settings, state, user_content, assistant_content),
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
    user_content: &str,
    assistant_content: &str,
) -> Option<String> {
    let (provider_id, model) = settings.effective_title_summary_model();
    let provider = settings.get_provider(&provider_id)?.clone();
    let provider_is_apple = provider.base_url == APPLE_INTELLIGENCE_BASE_URL;
    if !provider_is_apple && (provider.api_keys.is_empty() || model.trim().is_empty()) {
        return None;
    }

    let language = crate::settings::resolve_chat_language(settings);
    let prompt = build_title_summary_prompt(user_content, assistant_content, &language);
    let raw = if provider_is_apple {
        state.apple_intelligence.call_text(&prompt).await.ok()?
    } else {
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
            "Chat title summary",
        )
        .await
        .ok()?;
        assistant_content_from_api_message(&message)
    };

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

fn final_assistant_api_message(content: &str, reasoning: Option<&str>) -> Value {
    let mut message = serde_json::json!({
        "role": "assistant",
        "content": content,
    });
    if let Some(reasoning) = reasoning.map(str::trim).filter(|value| !value.is_empty()) {
        message["reasoning_content"] = Value::String(reasoning.to_string());
    }
    message
}

fn assistant_content_from_api_message(message: &Value) -> String {
    message
        .get("content")
        .and_then(|content| content.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn chat_tools_capable(
    provider: &crate::settings::ModelProvider,
    chat_tools: &crate::settings::ChatToolsConfig,
) -> bool {
    provider.supports_tools
        && (chat_tools.enabled || crate::settings::chat_native_tools_enabled(chat_tools))
}

fn resolve_forced_skill_id(
    chat_tools: &crate::settings::ChatToolsConfig,
    registry: &skills::SkillRegistry,
    requested: Option<&str>,
) -> Option<String> {
    let requested = requested.map(str::trim).filter(|id| !id.is_empty())?;
    let enabled = registry
        .records
        .iter()
        .filter(|record| crate::settings::is_skill_enabled(chat_tools, &record.meta.id))
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

fn build_chat_system_prompt(
    language: &str,
    has_image: bool,
    thinking_enabled: bool,
    registry: &skills::SkillRegistry,
    chat_tools: &crate::settings::ChatToolsConfig,
    tools_available: bool,
    available_builtin_tools: &[String],
    active_skill_id: Option<&str>,
    active_skill_detail: Option<&skills::SkillDetail>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
    custom_system_prompt: &str,
) -> String {
    build_chat_system_prompt_with_segments(
        language,
        has_image,
        thinking_enabled,
        registry,
        chat_tools,
        tools_available,
        available_builtin_tools,
        active_skill_id,
        active_skill_detail,
        assistant_snapshot,
        custom_system_prompt,
    )
    .0
}

fn build_chat_system_prompt_with_segments(
    language: &str,
    has_image: bool,
    thinking_enabled: bool,
    registry: &skills::SkillRegistry,
    chat_tools: &crate::settings::ChatToolsConfig,
    tools_available: bool,
    available_builtin_tools: &[String],
    active_skill_id: Option<&str>,
    active_skill_detail: Option<&skills::SkillDetail>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
    custom_system_prompt: &str,
) -> (String, Vec<ContextUsageSegment>) {
    let mut prompt = String::new();
    let mut segments = Vec::new();
    let base_prompt = if custom_system_prompt.trim().is_empty() {
        default_chat_system_prompt(language, has_image)
    } else {
        custom_system_prompt.trim().to_string()
    };
    append_context_segment(
        &mut prompt,
        &mut segments,
        "system_prompt",
        "System prompt",
        &base_prompt,
    );
    if let Some(assistant) = assistant_snapshot {
        let assistant_prompt = assistant_prompt_segment(assistant);
        if !assistant_prompt.trim().is_empty() {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "assistant",
                "Assistant",
                &assistant_prompt,
            );
        }
    }
    append_context_segment(
        &mut prompt,
        &mut segments,
        "runtime_context",
        "Runtime context",
        &crate::settings::chat_current_datetime_context(language),
    );

    if tools_available {
        let mut action_examples = vec!["activating a skill", "reading a file", "running a script"];
        if available_builtin_tools
            .iter()
            .any(|tool| matches!(tool.as_str(), "web_search" | "web_fetch"))
        {
            action_examples.push("using the web");
        }
        let mut runtime = format!(
            "You have access to tools (functions). When the user's request requires action—such as {}—YOU MUST call the appropriate enabled tool instead of describing what to do. Never say \"I cannot run commands\" or \"you can do it yourself\" when an enabled tool is available for that action. Do not call tools that are not listed as enabled.",
            action_examples.join(", ")
        );
        if language.starts_with("zh") {
            runtime.push_str(
                " 若用户只问今天/明天/星期几等可由上文「当前本地时间」直接推算的日期问题，直接回答，不要调用工具。",
            );
        } else {
            runtime.push_str(
                " If the user only asks for today/tomorrow/weekday derivable from the system local time above, answer directly without calling tools.",
            );
        }
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            &runtime,
        );
        if let Some(native_prompt) = native_tools_prompt(available_builtin_tools, language) {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "native_tools",
                "Native tools",
                &native_prompt,
            );
        }
    }

    let include_catalog = chat_tools.skill_auto_match
        || active_skill_id.is_some()
        || chat_tools.skill_fallback_mode != "legacy_full_body";
    if include_catalog {
        let catalog =
            skills::format_catalog(registry, active_skill_id, tools_available, |skill_id| {
                crate::settings::is_skill_enabled(chat_tools, skill_id)
            });
        if !catalog.is_empty() {
            append_context_segment(&mut prompt, &mut segments, "skills", "Skills", &catalog);
        }
    }

    if !chat_tools.skill_auto_match {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "skills",
            "Skills",
            "Only activate skills that are enabled in Settings (listed in the catalog below).",
        );
    }

    let fallback = chat_tools.skill_fallback_mode.as_str();
    if let Some(skill_id) = active_skill_id.filter(|id| !id.trim().is_empty()) {
        let mut skill_prompt = format!("User pinned skill for this message: {skill_id}");
        if tools_available {
            skill_prompt.push_str(
                ". Call skill_activate with this name only because the user pinned it; otherwise prefer Kivio built-in tools when they fit.",
            );
        } else if matches!(fallback, "skill_md_only" | "legacy_full_body") {
            skill_prompt.push_str(". Follow the Active Skill instructions below.");
        } else {
            skill_prompt.push_str(
                ". Progressive skill loading requires tool support; switch provider or set fallback to SKILL.md only.",
            );
        }
        append_context_segment(
            &mut prompt,
            &mut segments,
            "skills",
            "Skills",
            &skill_prompt,
        );
    } else if tools_available && chat_tools.skill_auto_match {
        let builtin_hint = if available_builtin_tools.is_empty() {
            "Kivio built-in tools".to_string()
        } else {
            format!("Kivio 内置工具（{}）", available_builtin_tools.join(", "))
        };
        if language.starts_with("zh") {
            append_context_segment(
                &mut prompt,
                &mut segments,
                "skills",
                "Skills",
                &format!("Skill 目录仅供参考：仅当用户明确需要某个 Skill 的能力（或点名 Skill 名称）时才 skill_activate。泛泛请求若已启用 {builtin_hint} 能覆盖，应优先使用对应内置工具；不要只因 Skill 描述里提到 Python/脚本/联网就激活无关 Skill。"),
            );
        } else {
            let builtin_hint = if available_builtin_tools.is_empty() {
                "Kivio built-in tools".to_string()
            } else {
                format!(
                    "Kivio built-in tools ({})",
                    available_builtin_tools.join(", ")
                )
            };
            append_context_segment(
                &mut prompt,
                &mut segments,
                "skills",
                "Skills",
                &format!("The skill catalog is optional: call skill_activate only when the user clearly needs that skill (or names it). For generic requests covered by enabled {builtin_hint}, prefer the corresponding built-in tool instead of activating an unrelated skill just because its description mentions Python, scripts, or web access."),
            );
        }
    }

    if matches!(fallback, "skill_md_only" | "legacy_full_body") {
        if let Some(skill) = active_skill_detail {
            if !skill.body.trim().is_empty() {
                append_context_segment(
                    &mut prompt,
                    &mut segments,
                    "skills",
                    "Skills",
                    &format!("Active Skill:\n{}", skill.body),
                );
            }
        }
    }

    if !thinking_enabled && !tools_available {
        append_context_segment(
            &mut prompt,
            &mut segments,
            "runtime_context",
            "Runtime context",
            chat_no_think_instruction(language),
        );
    }
    (prompt, merge_context_segments(segments))
}

fn append_context_segment(
    prompt: &mut String,
    segments: &mut Vec<ContextUsageSegment>,
    id: &str,
    label: &str,
    content: &str,
) {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str(trimmed);
    segments.push(ContextUsageSegment {
        id: id.to_string(),
        label: label.to_string(),
        estimated_tokens: estimate_tokens(trimmed),
        color: context_segment_color(id).map(str::to_string),
    });
}

fn assistant_prompt_segment(assistant: &ChatAssistantSnapshot) -> String {
    let mut parts = vec![format!("Active assistant: {}", assistant.name)];
    if !assistant.description.trim().is_empty() {
        parts.push(format!(
            "Assistant purpose: {}",
            assistant.description.trim()
        ));
    }
    if !assistant.system_prompt.trim().is_empty() {
        parts.push(format!(
            "Assistant instructions:\n{}",
            assistant.system_prompt.trim()
        ));
    }
    if !assistant.greeting.trim().is_empty() {
        parts.push(format!("Assistant greeting: {}", assistant.greeting.trim()));
    }
    if !assistant.conversation_starters.is_empty() {
        parts.push(format!(
            "Representative starter prompts: {}",
            assistant.conversation_starters.join(" | ")
        ));
    }
    parts.join("\n\n")
}

fn merge_context_segments(segments: Vec<ContextUsageSegment>) -> Vec<ContextUsageSegment> {
    let mut merged: Vec<ContextUsageSegment> = Vec::new();
    for segment in segments {
        if segment.estimated_tokens == 0 {
            continue;
        }
        if let Some(existing) = merged.iter_mut().find(|item| item.id == segment.id) {
            existing.estimated_tokens += segment.estimated_tokens;
        } else {
            merged.push(segment);
        }
    }
    merged
}

fn context_segment_color(id: &str) -> Option<&'static str> {
    match id {
        "system_prompt" => Some("#7A7A7A"),
        "assistant" => Some("#8A6FBD"),
        "runtime_context" => Some("#3E8B60"),
        "tool_definitions" => Some("#7553CF"),
        "skills" => Some("#BD8A3E"),
        "mcp" => Some("#B04B8D"),
        "native_tools" => Some("#4E7FB8"),
        "summarized_conversation" => Some("#BF3F66"),
        "conversation" => Some("#D07652"),
        "attachments" => Some("#6A8FBD"),
        _ => None,
    }
}

fn estimate_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }
    ascii.div_ceil(4) + non_ascii
}

fn context_window_for_model(model: &str) -> (usize, bool) {
    let lower = model.to_ascii_lowercase();
    let known = [
        ("1m", 1_000_000usize),
        ("200k", 200_000usize),
        ("128k", 128_000usize),
        ("100k", 100_000usize),
        ("64k", 64_000usize),
        ("32k", 32_000usize),
        ("16k", 16_000usize),
        ("8k", 8_000usize),
    ];
    for (needle, tokens) in known {
        if lower.contains(needle) {
            return (tokens, false);
        }
    }
    if lower.contains("claude") {
        return (200_000, false);
    }
    if lower.contains("gpt-4o")
        || lower.contains("gpt-4.1")
        || lower.contains("gpt-5")
        || lower.contains("deepseek")
        || lower.contains("qwen")
        || lower.contains("gemini")
    {
        return (128_000, true);
    }
    (FALLBACK_CONTEXT_WINDOW_TOKENS, true)
}

fn active_summary(conversation: &Conversation) -> Option<&ConversationContextSummary> {
    conversation
        .context_state
        .summary
        .as_ref()
        .filter(|summary| !summary.stale)
        .filter(|summary| !summary.content.trim().is_empty())
        .filter(|summary| {
            conversation
                .messages
                .iter()
                .any(|message| message.id == summary.source_until_message_id)
        })
}

fn summary_boundary_index(conversation: &Conversation) -> Option<usize> {
    let summary = active_summary(conversation)?;
    conversation
        .messages
        .iter()
        .position(|message| message.id == summary.source_until_message_id)
}

fn summary_message(summary: &ConversationContextSummary) -> Value {
    serde_json::json!({
        "role": "system",
        "content": format!("Previous conversation summary:\n{}", summary.content.trim()),
    })
}

fn mark_summary_stale_if_needed(conversation: &mut Conversation, changed_index: usize) {
    let Some(summary) = conversation.context_state.summary.as_mut() else {
        return;
    };
    let boundary_index = conversation
        .messages
        .iter()
        .position(|message| message.id == summary.source_until_message_id);
    if boundary_index
        .map(|boundary| changed_index <= boundary)
        .unwrap_or(true)
    {
        summary.stale = true;
        conversation.context_state.status = "stale".to_string();
    }
}

fn count_tokens_in_value(value: &Value) -> usize {
    match value {
        Value::String(text) => estimate_tokens(text),
        Value::Array(items) => items.iter().map(count_tokens_in_value).sum(),
        Value::Object(_) => estimate_tokens(&serde_json::to_string(value).unwrap_or_default()),
        _ => estimate_tokens(&value.to_string()),
    }
}

fn push_estimated_segment(
    segments: &mut Vec<ContextUsageSegment>,
    id: &str,
    label: &str,
    tokens: usize,
) {
    if tokens == 0 {
        return;
    }
    segments.push(ContextUsageSegment {
        id: id.to_string(),
        label: label.to_string(),
        estimated_tokens: tokens,
        color: context_segment_color(id).map(str::to_string),
    });
}

fn estimate_tool_segments(tools: &[ChatToolDefinition]) -> Vec<ContextUsageSegment> {
    let mut segments = Vec::new();
    for tool in tools {
        let tool_value = tool.to_openai_tool();
        let id = match tool.source.as_str() {
            "mcp" => "mcp",
            "native" => "native_tools",
            "skill" => "skills",
            _ => "tool_definitions",
        };
        let label = match id {
            "mcp" => "MCP",
            "native_tools" => "Native tools",
            "skills" => "Skills",
            _ => "Tool definitions",
        };
        push_estimated_segment(&mut segments, id, label, count_tokens_in_value(&tool_value));
    }
    merge_context_segments(segments)
}

fn estimate_messages_segments(
    conversation: &Conversation,
    messages: &[Value],
    last_user_image_paths: &[PathBuf],
) -> Vec<ContextUsageSegment> {
    let mut segments = Vec::new();
    let summary_tokens = active_summary(conversation)
        .map(|summary| estimate_tokens(&summary.content))
        .unwrap_or_default();
    push_estimated_segment(
        &mut segments,
        "summarized_conversation",
        "Summarized conversation",
        summary_tokens,
    );

    let conversation_tokens = messages
        .iter()
        .filter(|message| {
            message
                .get("role")
                .and_then(|role| role.as_str())
                .map(|role| role != "system")
                .unwrap_or(true)
        })
        .map(count_tokens_in_value)
        .sum::<usize>();
    push_estimated_segment(
        &mut segments,
        "conversation",
        "Conversation",
        conversation_tokens,
    );
    push_estimated_segment(
        &mut segments,
        "attachments",
        "Attachments",
        last_user_image_paths.len() * IMAGE_ATTACHMENT_TOKEN_ESTIMATE,
    );
    merge_context_segments(segments)
}

async fn compute_context_state(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &Conversation,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
) -> Result<ConversationContextState, String> {
    let settings = state.settings_read().clone();
    let provider = settings.get_provider(&conversation.provider_id).cloned();
    let provider_supports_tools = provider
        .as_ref()
        .map(|provider| provider.supports_tools)
        .unwrap_or(false);
    let provider_is_apple = provider
        .as_ref()
        .map(|provider| provider.base_url == APPLE_INTELLIGENCE_BASE_URL)
        .unwrap_or(false);
    let language = crate::settings::resolve_chat_language(&settings);
    let thinking_enabled = settings.chat.thinking_enabled;
    let skill_registry =
        skills::build_registry(app, &settings.chat_tools.skill_scan_paths).unwrap_or_default();
    let requested_skill_id = conversation.active_skill_id.as_deref().or_else(|| {
        conversation
            .assistant_snapshot
            .as_ref()
            .and_then(|assistant| assistant.skill_id.as_deref())
    });
    let active_skill_id =
        resolve_forced_skill_id(&settings.chat_tools, &skill_registry, requested_skill_id);
    let active_skill_detail = active_skill_id.as_deref().and_then(|id| {
        skills::read_skill_detail(app, &settings.chat_tools.skill_scan_paths, id).ok()
    });
    let mut effective_chat_tools = settings.chat_tools.clone();
    let tools_capable = provider_supports_tools
        && !provider_is_apple
        && (settings.chat_tools.enabled
            || crate::settings::chat_native_tools_enabled(&settings.chat_tools));
    let mut tools = list_tools_for_chat(state.inner(), &settings, provider_supports_tools).await;
    apply_assistant_tool_preset(&mut tools, conversation.assistant_snapshot.as_ref());
    if let Some(skill) = active_skill_id
        .as_deref()
        .and_then(|id| skill_registry.find(id))
    {
        apply_active_skill_tool_filter(&mut tools, skill);
    }
    let available_builtin_tools = available_builtin_tool_names(&tools);
    let tools_available = tools_capable && !tools.is_empty();
    apply_skill_fallback_when_tools_unavailable(
        &mut effective_chat_tools,
        active_skill_id.as_deref(),
        tools_available,
    );

    let (system_prompt, mut segments) = build_chat_system_prompt_with_segments(
        &language,
        !last_user_image_paths.is_empty(),
        thinking_enabled,
        &skill_registry,
        &effective_chat_tools,
        tools_available,
        &available_builtin_tools,
        active_skill_id.as_deref(),
        active_skill_detail.as_ref(),
        conversation.assistant_snapshot.as_ref(),
        settings.chat.system_prompt.as_str(),
    );
    let last_user_idx = conversation.messages.iter().rposition(|m| m.role == "user");
    let request_messages = build_chat_api_messages(
        &system_prompt,
        conversation,
        last_user_idx,
        last_user_api_content,
        last_user_image_paths,
    )?;
    segments.extend(estimate_messages_segments(
        conversation,
        &request_messages,
        last_user_image_paths,
    ));

    if tools_available {
        segments.extend(estimate_tool_segments(&tools));
    }

    let segments = merge_context_segments(segments);
    let estimated_input_tokens = segments
        .iter()
        .map(|segment| segment.estimated_tokens)
        .sum::<usize>();
    let (context_window_tokens, context_window_estimated) =
        context_window_for_model(&conversation.model);
    let usage_ratio = if context_window_tokens == 0 {
        None
    } else {
        Some(estimated_input_tokens as f32 / context_window_tokens as f32)
    };
    let summary = conversation.context_state.summary.clone();
    let status = context_status(usage_ratio, summary.as_ref());
    let last_compressed_at = summary
        .as_ref()
        .filter(|summary| !summary.stale)
        .map(|summary| summary.created_at)
        .or(conversation.context_state.last_compressed_at);
    let compressed_message_count = summary
        .as_ref()
        .filter(|summary| !summary.stale)
        .map(|summary| summary.source_message_ids.len())
        .unwrap_or_default();

    Ok(ConversationContextState {
        estimated_input_tokens,
        context_window_tokens: Some(context_window_tokens),
        context_window_estimated,
        usage_ratio,
        status,
        segments,
        last_measured_at: chrono::Local::now().timestamp(),
        last_compressed_at,
        compressed_message_count,
        summary,
        warning: conversation.context_state.warning.clone(),
    })
}

fn context_likely_over_limit(context_state: &ConversationContextState) -> bool {
    context_state
        .usage_ratio
        .map(|ratio| ratio >= CONTEXT_BLOCK_RATIO)
        .unwrap_or(false)
}

async fn rollback_user_message_after_failed_send(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    user_message_id: &str,
) -> Result<(), String> {
    conversation
        .messages
        .retain(|message| message.id != user_message_id);
    conversation.updated_at = chrono::Local::now().timestamp();
    match compute_context_state(app, state, conversation, None, &[]).await {
        Ok(mut context_state) => {
            context_state.warning = None;
            conversation.context_state = context_state.clone();
            emit_chat_context_state(app, &conversation.id, &context_state);
        }
        Err(context_err) => {
            eprintln!("Context usage estimate failed after send rollback: {context_err}");
        }
    }
    save_conversation(app, conversation)
}

fn should_auto_compress_context(
    context_state: &ConversationContextState,
    conversation: &Conversation,
) -> bool {
    let Some(ratio) = context_state.usage_ratio else {
        return false;
    };
    if ratio < AUTO_COMPRESS_RATIO {
        return false;
    }
    if active_summary(conversation).is_some() {
        return false;
    }
    compression_boundary_index(conversation).is_some()
}

async fn compress_conversation_context(
    _app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
) -> Result<(), String> {
    let boundary_index = compression_boundary_index(conversation)
        .ok_or_else(|| "没有足够的旧消息可以压缩".to_string())?;
    let source_messages = conversation
        .messages
        .iter()
        .take(boundary_index + 1)
        .cloned()
        .collect::<Vec<_>>();
    if source_messages.len() < 2 {
        return Err("没有足够的旧消息可以压缩".to_string());
    }

    let settings = state.settings_read().clone();
    let (provider_id, model) = settings.effective_compression_model();
    let provider = settings
        .get_provider(&provider_id)
        .ok_or_else(|| "Compression provider not found".to_string())?
        .clone();
    let provider_is_apple = provider.base_url == APPLE_INTELLIGENCE_BASE_URL;
    if !provider_is_apple && provider.api_keys.is_empty() {
        return Err(format_chat_missing_api_key_error(&provider.name));
    }
    if model.trim().is_empty() {
        return Err(chat_missing_model_error());
    }

    let source_text = format_messages_for_context_summary(&source_messages);
    let prompt = build_context_compression_prompt(&source_text);
    let token_estimate_before = estimate_tokens(&source_text);
    let retry_attempts = if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    };
    let raw_summary = if provider_is_apple {
        state.apple_intelligence.call_text(&prompt).await?
    } else {
        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": "You compress chat history into dense factual memory for future assistant requests. Output only the summary.",
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
            "Chat context compression",
        )
        .await?;
        assistant_content_from_api_message(&message)
    };
    let summary_content = sanitize_context_summary(&raw_summary);
    if summary_content.trim().is_empty() {
        return Err("Compression model returned an empty summary".to_string());
    }

    let source_until_message_id = source_messages
        .last()
        .map(|message| message.id.clone())
        .ok_or_else(|| "没有足够的旧消息可以压缩".to_string())?;
    let source_message_ids = source_messages
        .iter()
        .map(|message| message.id.clone())
        .collect::<Vec<_>>();
    let created_at = chrono::Local::now().timestamp();
    conversation.context_state.summary = Some(ConversationContextSummary {
        id: format!("ctxsum_{}", Uuid::new_v4()),
        content: summary_content.clone(),
        source_message_ids,
        source_until_message_id,
        token_estimate_before,
        token_estimate_after: estimate_tokens(&summary_content),
        created_at,
        provider_id,
        model,
        stale: false,
    });
    conversation.context_state.last_compressed_at = Some(created_at);
    conversation.context_state.compressed_message_count = source_messages.len();
    conversation.context_state.warning = None;
    Ok(())
}

fn compression_boundary_index(conversation: &Conversation) -> Option<usize> {
    if conversation.messages.len() <= KEEP_RECENT_RAW_MESSAGES + 2 {
        return None;
    }
    let max_boundary = conversation
        .messages
        .len()
        .saturating_sub(KEEP_RECENT_RAW_MESSAGES + 1);
    (0..=max_boundary)
        .rev()
        .find(|idx| conversation.messages[*idx].role == "assistant")
}

fn format_messages_for_context_summary(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            let role = match message.role.as_str() {
                "assistant" => "Assistant",
                _ => "User",
            };
            let mut content = message.content.trim().to_string();
            if !message.attachments.is_empty() {
                let names = message
                    .attachments
                    .iter()
                    .map(|attachment| {
                        format!("{} ({})", attachment.name, attachment.attachment_type)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if !content.is_empty() {
                    content.push_str("\n");
                }
                content.push_str(&format!("[Attachments: {names}]"));
            }
            if !message.tool_calls.is_empty() {
                let tools = message
                    .tool_calls
                    .iter()
                    .map(|tool| {
                        let status = serde_json::to_string(&tool.status)
                            .unwrap_or_else(|_| "\"unknown\"".to_string());
                        format!(
                            "{} {}: {}{}",
                            tool.source,
                            tool.name,
                            status.trim_matches('"'),
                            tool.result_preview
                                .as_deref()
                                .map(|preview| format!(" - {}", truncate_chars(preview, 500)))
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !content.is_empty() {
                    content.push_str("\n");
                }
                content.push_str("[Tool calls]\n");
                content.push_str(&tools);
            }
            format!("{role}:\n{content}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_context_compression_prompt(source_text: &str) -> String {
    format!(
        "Compress the older part of this Kivio Chat conversation into a dense factual memory for future model requests.\n\nRules:\n- Preserve user goals, preferences, constraints, decisions, file paths, commands, tool results, unresolved questions, and important facts.\n- Preserve chronological cause/effect when it matters.\n- Mention attachments by file name and relevance, but do not invent image contents.\n- Do not include small talk, redundant phrasing, or style commentary.\n- Do not invent facts.\n- Output concise Markdown only.\n\nConversation to compress:\n\n{source_text}"
    )
}

fn sanitize_context_summary(raw: &str) -> String {
    raw.trim()
        .trim_matches(['`', ' ', '\n', '\r'])
        .trim()
        .to_string()
}

fn emit_chat_context_state(
    app: &AppHandle,
    conversation_id: &str,
    context_state: &ConversationContextState,
) {
    let _ = app.emit(
        "chat-context",
        serde_json::json!({
            "conversationId": conversation_id,
            "contextState": context_state,
        }),
    );
}

fn context_status(
    usage_ratio: Option<f32>,
    summary: Option<&ConversationContextSummary>,
) -> String {
    if summary.is_some_and(|item| item.stale) {
        return "stale".to_string();
    }
    if summary.is_some() {
        return "compressed".to_string();
    }
    let Some(ratio) = usage_ratio else {
        return "unknown".to_string();
    };
    if ratio >= 0.95 {
        "critical".to_string()
    } else if ratio >= 0.70 {
        "warning".to_string()
    } else {
        "normal".to_string()
    }
}

fn tool_matches_recommended_name(tool: &ChatToolDefinition, recommended: &str) -> bool {
    let recommended = recommended.trim();
    if recommended.is_empty() {
        return false;
    }
    tool.name == recommended
        || tool.id == recommended
        || tool.openai_tool_name() == recommended
        || tool
            .server_id
            .as_deref()
            .map(|server_id| format!("{server_id}:{}", tool.name) == recommended)
            .unwrap_or(false)
}

fn is_native_skill_tool_name(name: &str) -> bool {
    matches!(
        name,
        "skill_activate" | "skill_read_file" | "skill_run_script"
    )
}

fn is_kivio_builtin_tool(tool: &ChatToolDefinition) -> bool {
    tool.source == "native" && !is_native_skill_tool_name(&tool.name)
}

async fn list_tools_for_chat(
    state: &AppState,
    settings: &Settings,
    provider_supports_tools: bool,
) -> Vec<ChatToolDefinition> {
    if !provider_supports_tools
        || !(settings.chat_tools.enabled
            || crate::settings::chat_native_tools_enabled(&settings.chat_tools))
    {
        return Vec::new();
    }
    mcp::registry::list_enabled_tool_defs(state)
        .await
        .unwrap_or_default()
}

fn apply_active_skill_tool_filter(
    tools: &mut Vec<ChatToolDefinition>,
    skill: &skills::SkillRecord,
) {
    if skill.allowed_tools.is_empty() {
        return;
    }
    tools.retain(|tool| {
        tool.source == "skill"
            || is_native_skill_tool_name(&tool.name)
            || is_kivio_builtin_tool(tool)
            || skill
                .allowed_tools
                .iter()
                .any(|recommended| tool_matches_recommended_name(tool, recommended))
    });
}

fn apply_assistant_tool_preset(
    tools: &mut Vec<ChatToolDefinition>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
) {
    let preset = assistant_snapshot
        .map(|assistant| assistant.tool_preset.trim())
        .filter(|preset| !preset.is_empty())
        .unwrap_or("inherit");
    match preset {
        "none" => tools.clear(),
        "skills" => tools.retain(|tool| tool.source == "skill"),
        "inherit" | "all" => {}
        _ => {}
    }
}

fn apply_skill_fallback_when_tools_unavailable(
    chat_tools: &mut crate::settings::ChatToolsConfig,
    active_skill_id: Option<&str>,
    tools_available: bool,
) {
    if !tools_available
        && active_skill_id
            .map(|id| !id.trim().is_empty())
            .unwrap_or(false)
        && chat_tools.skill_fallback_mode == "progressive"
    {
        chat_tools.skill_fallback_mode = "skill_md_only".to_string();
    }
}

fn available_builtin_tool_names(tools: &[ChatToolDefinition]) -> Vec<String> {
    let mut names = tools
        .iter()
        .filter(|tool| is_kivio_builtin_tool(tool))
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn disabled_builtin_tool_feedback(function_name: &str) -> Option<String> {
    const BUILTIN_NAMES: &[&str] = &[
        "web_search",
        "web_fetch",
        "read_file",
        "write_file",
        "edit_file",
        "run_command",
        "run_python",
    ];
    if BUILTIN_NAMES.contains(&function_name) {
        Some(format!(
            "Kivio tool `{function_name}` is not enabled for this chat. Do not call it again; answer using the available context and enabled tools only."
        ))
    } else {
        None
    }
}

/// Kivio 内置工具始终自动执行，不走审批弹窗。
fn builtin_tool_bypasses_approval(tool: &ChatToolDefinition) -> bool {
    (tool.source == "skill" && is_native_skill_tool_name(&tool.name)) || tool.source == "native"
}

fn native_tools_prompt(available_builtin_tools: &[String], language: &str) -> Option<String> {
    if available_builtin_tools.is_empty() {
        return None;
    }
    let list = available_builtin_tools.join(", ");
    let has_web_search = available_builtin_tools
        .iter()
        .any(|tool| tool.as_str() == "web_search");
    let has_web_fetch = available_builtin_tools
        .iter()
        .any(|tool| tool.as_str() == "web_fetch");
    let zh_live_access_hint = match (has_web_search, has_web_fetch) {
        (true, true) => "实时搜索或网页读取必须优先用 web_search/web_fetch 或对应 Skill 脚本。",
        (true, false) => "实时搜索必须优先用 web_search 或对应 Skill 脚本。",
        (false, true) => "网页读取必须优先用 web_fetch 或对应 Skill 脚本。",
        (false, false) => "需要联网/API 访问时，请启用对应联网工具或使用对应 Skill 脚本。",
    };
    let en_live_access_hint = match (has_web_search, has_web_fetch) {
        (true, true) => {
            "Use web_search/web_fetch or the relevant Skill script for live web/API access."
        }
        (true, false) => "Use web_search or the relevant Skill script for live web/API access.",
        (false, true) => "Use web_fetch or the relevant Skill script for web page access.",
        (false, false) => {
            "Enable the relevant web tool or use the relevant Skill script for live web/API access."
        }
    };
    let prompt = if language.starts_with("zh") {
        format!(
            "Kivio 内置工具（已启用）：{list}。只允许调用这里列出的内置工具。文件路径须在用户主目录内；可选工作区根目录进一步收紧。write_file、edit_file、run_command 会请求用户确认；run_command 非零退出码代表执行失败，不要用它运行 Skill 自带脚本，Skill 脚本必须走 skill_run_script。run_command 不得用 pip/pip3/python -m pip 安装包来绕过 run_python 沙盒失败；只有用户明确要求修改本机 Python 环境时，才能设置 allow_host_python_package_install=true 且使用 --user 或虚拟环境。run_python 在 Pyodide 沙盒中运行，无本机文件系统和联网能力；导入 numpy、matplotlib、pandas、scipy、sympy、scikit-learn、statsmodels、pillow、seaborn、micropip 等常用包时会自动加载，适合数据运算、统计分析、机器学习基础分析和生成图表；用 run_python 生成图像/图表时，保存为 Pyodide 当前目录下的相对文件名（例如 output.png），不要保存到 /Users 等本机路径，不要 print base64 或 data:image URL；Kivio 会自动捕获并渲染生成的图片。不要在 run_python 里使用 tavily、requests、httpx、urllib3、aiohttp 等联网/API 客户端，{zh_live_access_hint}不要为了这些 Python 包使用 host pip 安装，除非用户明确要求操作本机环境。用户要用 Python 跑代码/计算时优先 run_python，不要用 skill_run_script，除非用户点名某个 Skill。"
        )
    } else {
        format!(
            "Kivio built-in tools enabled: {list}. Only call built-in tools listed here. Paths must stay under the user home directory (optional workspace roots further restrict). write_file, edit_file, and run_command require user approval; run_command treats non-zero exit codes as failures. Do not use run_command to run Skill bundled scripts; use skill_run_script. Do not use pip/pip3/python -m pip through run_command to bypass run_python sandbox failures; only set allow_host_python_package_install=true when the user explicitly asks to modify the host Python environment, and then use --user or a virtual environment. run_python runs in a Pyodide sandbox with no host filesystem or network access and auto-loads common packages when imported, including numpy, matplotlib, pandas, scipy, sympy, scikit-learn, statsmodels, pillow, seaborn, and micropip; use it for data computation, statistical analysis, basic machine-learning analysis, code execution, and charts. When generating images/charts with run_python, save them to relative filenames in the Pyodide current directory such as output.png; do not save to host paths such as /Users, and do not print base64 or data:image URLs. Kivio captures and renders generated images automatically. Do not use network/API clients such as tavily, requests, httpx, urllib3, or aiohttp in run_python; {en_live_access_hint} Do not use host pip to install these Python packages unless the user explicitly asks to modify the host environment. For generic Python requests, use run_python—not skill_run_script—unless the user named a specific skill."
        )
    };
    Some(prompt)
}

fn apply_provider_tools_fallback(
    runtime_messages: &mut [Value],
    language: &str,
    has_image: bool,
    thinking_enabled: bool,
    registry: &skills::SkillRegistry,
    chat_tools: &mut crate::settings::ChatToolsConfig,
    active_skill_id: Option<&str>,
    active_skill_detail: Option<&skills::SkillDetail>,
    assistant_snapshot: Option<&ChatAssistantSnapshot>,
    custom_system_prompt: &str,
) {
    if active_skill_id.is_some() && chat_tools.skill_fallback_mode == "progressive" {
        chat_tools.skill_fallback_mode = "skill_md_only".to_string();
    }
    let fallback_prompt = build_chat_system_prompt(
        language,
        has_image,
        thinking_enabled,
        registry,
        chat_tools,
        false,
        &[],
        active_skill_id,
        active_skill_detail,
        assistant_snapshot,
        custom_system_prompt,
    );
    patch_system_message(runtime_messages, &fallback_prompt);
}

fn patch_system_message(messages: &mut [Value], prompt: &str) {
    if let Some(first) = messages.first_mut() {
        if first.get("role").and_then(|role| role.as_str()) == Some("system") {
            first["content"] = Value::String(prompt.to_string());
        }
    }
}

fn build_chat_api_messages(
    system_prompt: &str,
    conversation: &Conversation,
    last_user_idx: Option<usize>,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
) -> Result<Vec<Value>, String> {
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": system_prompt,
    })];

    let start_idx = if let Some(summary) = active_summary(conversation) {
        messages.push(summary_message(summary));
        summary_boundary_index(conversation)
            .map(|idx| idx + 1)
            .unwrap_or_default()
    } else {
        0
    };

    for (idx, message) in conversation.messages.iter().enumerate() {
        if idx < start_idx {
            continue;
        }
        let content = if Some(idx) == last_user_idx {
            last_user_api_content.unwrap_or(message.content.as_str())
        } else {
            message.content.as_str()
        };
        let sanitized_content = sanitize_image_payloads_for_model(content);
        if Some(idx) == last_user_idx && !last_user_image_paths.is_empty() {
            let mut parts = last_user_image_paths
                .iter()
                .map(image_content_part)
                .collect::<Result<Vec<_>, _>>()?;
            parts.push(serde_json::json!({ "type": "text", "text": sanitized_content }));
            messages.push(serde_json::json!({
                "role": message.role,
                "content": parts,
            }));
        } else {
            messages.push(serde_json::json!({
                "role": message.role,
                "content": sanitized_content,
            }));
        }
        if message.role == "assistant" && !message.model_messages.is_empty() {
            messages.pop();
            messages.extend(
                openai_messages_from_model_messages(&message.model_messages)
                    .iter()
                    .map(sanitize_api_message_for_model),
            );
        } else if message.role == "assistant" && !message.api_messages.is_empty() {
            messages.pop();
            messages.extend(
                message
                    .api_messages
                    .iter()
                    .map(sanitize_api_message_for_model),
            );
        }
    }

    Ok(messages)
}

fn build_apple_chat_prompt(
    system_prompt: &str,
    conversation: &Conversation,
    last_user_idx: Option<usize>,
    last_user_api_content: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if !system_prompt.trim().is_empty() {
        parts.push(format!("System:\n{}", system_prompt.trim()));
    }
    let start_idx = if let Some(summary) = active_summary(conversation) {
        parts.push(format!(
            "System:\nPrevious conversation summary:\n{}",
            summary.content.trim()
        ));
        summary_boundary_index(conversation)
            .map(|idx| idx + 1)
            .unwrap_or_default()
    } else {
        0
    };
    for (idx, message) in conversation.messages.iter().enumerate() {
        if idx < start_idx {
            continue;
        }
        let role = match message.role.as_str() {
            "assistant" => "Assistant",
            _ => "User",
        };
        let content = if Some(idx) == last_user_idx {
            last_user_api_content.unwrap_or(message.content.as_str())
        } else {
            message.content.as_str()
        };
        let content = sanitize_image_payloads_for_model(content);
        if !content.trim().is_empty() {
            parts.push(format!("{role}:\n{}", content.trim()));
        }
    }
    parts.push("Assistant:".to_string());
    parts.join("\n\n")
}

async fn call_chat_completion_message(
    state: &State<'_, AppState>,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    thinking_enabled: bool,
    label: &str,
) -> Result<Value, String> {
    let request = generate_request_from_openai_messages(
        model,
        messages,
        tools,
        GenerateOptions {
            thinking_enabled,
            ..GenerateOptions::default()
        },
        label,
    );
    let output =
        generate_with_chat_provider(state.inner(), provider, retry_attempts, request).await?;
    Ok(output.to_openai_compatible_message())
}

#[allow(clippy::too_many_arguments)]
async fn stream_scoped_chat_completion(
    app: &AppHandle,
    state: &State<'_, AppState>,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    retry_attempts: usize,
    thinking_enabled: bool,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
) -> Result<ChatStreamOutput, String> {
    let request = generate_request_from_openai_messages(
        model,
        messages,
        None,
        GenerateOptions {
            stream: true,
            thinking_enabled,
            ..GenerateOptions::default()
        },
        "Chat stream",
    );
    let accumulator = Arc::new(Mutex::new(ChatStreamAccumulator::default()));
    let mut sink = ChatTauriStreamSink::new(
        app.clone(),
        conversation_id,
        run_id,
        message_id,
        accumulator.clone(),
    );
    let output = tokio::select! {
        result = stream_with_chat_provider(
            state.inner(),
            provider,
            retry_attempts,
            request,
            &mut sink,
        ) => result?,
        _ = wait_for_chat_cancel(state.inner(), conversation_id, generation) => {
            let snapshot = chat_stream_snapshot(&accumulator);
            emit_chat_stream_done(
                app,
                conversation_id,
                run_id,
                message_id,
                "cancelled",
                snapshot.content.trim(),
            );
            return Ok(ChatStreamOutput::new(
                snapshot.content.trim().to_string(),
                snapshot.reasoning.trim().to_string(),
                true,
            ));
        }
    };
    let snapshot = chat_stream_snapshot(&accumulator);
    let content = if output.text.trim().is_empty() {
        snapshot.content
    } else {
        output.text
    };
    let cleaned = sanitize_assistant_text_response(content.trim());
    if cleaned.trim().is_empty() {
        emit_chat_stream_done(app, conversation_id, run_id, message_id, "error", "");
        return Err(empty_assistant_response_error("Chat stream"));
    }
    emit_chat_stream_done(app, conversation_id, run_id, message_id, "done", &cleaned);
    let reasoning = output.reasoning.unwrap_or(snapshot.reasoning);
    Ok(ChatStreamOutput::new(cleaned, reasoning, false))
}

async fn generate_with_chat_provider(
    state: &AppState,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: crate::chat::model::GenerateRequest,
) -> Result<GenerateOutput, String> {
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        ProviderApiFormat::AppleLocal => {
            AppleLocalProvider::new(state.apple_intelligence.clone())
                .generate(request)
                .await
        }
    }
    .map_err(|err| err.to_string())
}

async fn stream_with_chat_provider(
    state: &AppState,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: crate::chat::model::GenerateRequest,
    sink: &mut (dyn StreamSink + Send),
) -> Result<GenerateOutput, String> {
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        ProviderApiFormat::AppleLocal => {
            AppleLocalProvider::new(state.apple_intelligence.clone())
                .stream(request, sink)
                .await
        }
    }
    .map_err(|err| err.to_string())
}

#[derive(Default)]
struct ChatStreamAccumulator {
    content: String,
    reasoning: String,
}

struct ChatStreamSnapshot {
    content: String,
    reasoning: String,
}

fn chat_stream_snapshot(accumulator: &Arc<Mutex<ChatStreamAccumulator>>) -> ChatStreamSnapshot {
    let guard = accumulator.lock().unwrap_or_else(|err| err.into_inner());
    ChatStreamSnapshot {
        content: guard.content.clone(),
        reasoning: guard.reasoning.clone(),
    }
}

struct ChatTauriStreamSink {
    app: AppHandle,
    conversation_id: String,
    run_id: String,
    message_id: String,
    accumulator: Arc<Mutex<ChatStreamAccumulator>>,
}

impl ChatTauriStreamSink {
    fn new(
        app: AppHandle,
        conversation_id: &str,
        run_id: &str,
        message_id: &str,
        accumulator: Arc<Mutex<ChatStreamAccumulator>>,
    ) -> Self {
        Self {
            app,
            conversation_id: conversation_id.to_string(),
            run_id: run_id.to_string(),
            message_id: message_id.to_string(),
            accumulator,
        }
    }
}

impl StreamSink for ChatTauriStreamSink {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        match part {
            StreamPart::TextDelta { delta } => {
                self.accumulator
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .content
                    .push_str(&delta);
                emit_chat_stream_delta(
                    &self.app,
                    &self.conversation_id,
                    &self.run_id,
                    &self.message_id,
                    &delta,
                    None,
                );
            }
            StreamPart::ReasoningDelta { delta } => {
                self.accumulator
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .reasoning
                    .push_str(&delta);
                emit_chat_stream_delta(
                    &self.app,
                    &self.conversation_id,
                    &self.run_id,
                    &self.message_id,
                    "",
                    Some(&delta),
                );
            }
            StreamPart::Error { message } => return Err(ModelError::new(message)),
            StreamPart::Finish { .. }
            | StreamPart::ToolCallStart { .. }
            | StreamPart::ToolCallDelta { .. }
            | StreamPart::ToolCallDone { .. }
            | StreamPart::ToolResult { .. } => {}
        }
        Ok(())
    }
}

struct ChatStreamOutput {
    content: String,
    reasoning: Option<String>,
    cancelled: bool,
}

impl ChatStreamOutput {
    fn new(content: String, reasoning: String, cancelled: bool) -> Self {
        Self {
            content,
            reasoning: if reasoning.trim().is_empty() {
                None
            } else {
                Some(reasoning)
            },
            cancelled,
        }
    }
}

fn extract_reasoning_content(message: &Value) -> Option<String> {
    message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn merge_reasoning(planning_parts: &[String], final_reasoning: Option<String>) -> Option<String> {
    let mut parts = planning_parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(reasoning) = final_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|reasoning| !reasoning.is_empty())
    {
        parts.push(reasoning.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn extract_tool_calls(message: &Value) -> Vec<PendingToolCall> {
    let from_api = extract_openai_tool_calls(message);
    if !from_api.is_empty() {
        return from_api;
    }
    let content = assistant_content_from_api_message(message);
    pending_tool_calls_from_dsml(&content)
}

fn extract_openai_tool_calls(message: &Value) -> Vec<PendingToolCall> {
    pending_tool_calls_from_openai_message(message)
}

fn pending_tool_calls_from_dsml(content: &str) -> Vec<PendingToolCall> {
    crate::chat::dsml_tools::extract_dsml_tool_calls(content)
        .into_iter()
        .map(|call| {
            let arguments = Value::Object(call.arguments);
            let arguments_raw =
                serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string());
            PendingToolCall {
                id: format!("tool_{}", Uuid::new_v4()),
                function_name: call.name,
                arguments,
                arguments_raw,
                arguments_parse_error: None,
            }
        })
        .collect()
}

fn assistant_api_message_for_tool_calls(message: &Value, tool_calls: &[PendingToolCall]) -> Value {
    if message
        .get("tool_calls")
        .and_then(|value| value.as_array())
        .is_some_and(|calls| !calls.is_empty())
    {
        return message.clone();
    }
    serde_json::json!({
        "role": "assistant",
        "content": Value::Null,
        "tool_calls": tool_calls.iter().map(|call| {
            serde_json::json!({
                "id": call.id,
                "type": "function",
                "function": {
                    "name": call.function_name,
                    "arguments": call.arguments_raw,
                }
            })
        }).collect::<Vec<_>>(),
    })
}

fn sanitize_assistant_text_response(content: &str) -> String {
    let stripped = crate::chat::dsml_tools::strip_dsml_tool_markup(content);
    if stripped.is_empty() && crate::chat::dsml_tools::contains_dsml_tool_markup(content) {
        return String::new();
    }
    stripped
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatStreamTextMode {
    Delta,
    Snapshot,
}

fn extract_chat_stream_text(value: &Value) -> Option<(&str, ChatStreamTextMode)> {
    let choice = value.get("choices").and_then(|choices| choices.get(0))?;

    if let Some(content) = choice
        .get("delta")
        .and_then(|delta| delta.get("content"))
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
    {
        return Some((content, ChatStreamTextMode::Delta));
    }

    if let Some(content) = choice
        .get("text")
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
    {
        return Some((content, ChatStreamTextMode::Delta));
    }

    choice
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
        .map(|content| (content, ChatStreamTextMode::Snapshot))
}

fn extract_chat_stream_reasoning(value: &Value) -> Option<(&str, ChatStreamTextMode)> {
    let choice = value.get("choices").and_then(|choices| choices.get(0))?;

    if let Some(reasoning) = choice
        .get("delta")
        .and_then(|delta| {
            delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
        })
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
    {
        return Some((reasoning, ChatStreamTextMode::Delta));
    }

    choice
        .get("message")
        .and_then(|message| {
            message
                .get("reasoning_content")
                .or_else(|| message.get("reasoning"))
        })
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
        .map(|content| (content, ChatStreamTextMode::Snapshot))
}

fn append_chat_stream_text(
    full: &mut String,
    text: &str,
    mode: ChatStreamTextMode,
) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    match mode {
        ChatStreamTextMode::Delta => {
            full.push_str(text);
            Some(text.to_string())
        }
        ChatStreamTextMode::Snapshot => {
            if text == full || full.starts_with(text) {
                return None;
            }
            if text.starts_with(full.as_str()) {
                let delta = text[full.len()..].to_string();
                full.clear();
                full.push_str(text);
                return if delta.is_empty() { None } else { Some(delta) };
            }

            full.push_str(text);
            Some(text.to_string())
        }
    }
}

fn empty_assistant_response_error(scope: &str) -> String {
    format!("{scope} returned an empty assistant response")
}

fn match_tool_call<'a>(
    tools: &'a [ChatToolDefinition],
    function_name: &str,
) -> Option<&'a ChatToolDefinition> {
    tools
        .iter()
        .find(|tool| tool.openai_tool_name() == function_name || tool.name == function_name)
}

fn is_tools_unsupported_error(err: &str) -> bool {
    let Some(code) = extract_status_code(err) else {
        return false;
    };
    if !matches!(code, 400 | 404 | 422 | 501) {
        return false;
    }
    let lower = err.to_ascii_lowercase();
    lower.contains("tools")
        || lower.contains("tool_choice")
        || lower.contains("tool_calls")
        || lower.contains("function calling")
        || lower.contains("function_call")
        || lower.contains("function call")
        || lower.contains("not support")
        || (code == 400 && lower.contains("tool"))
}

fn unknown_tool_record(call: &PendingToolCall, round: u8, error: String) -> ToolCallRecord {
    let now = chrono::Local::now().timestamp();
    ToolCallRecord {
        id: call.id.clone(),
        name: call.function_name.clone(),
        source: "unknown".to_string(),
        server_id: None,
        arguments: call.arguments_raw.clone(),
        status: ToolCallStatus::Error,
        result_preview: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round,
        sensitive: false,
        artifacts: Vec::new(),
    }
}

fn invalid_tool_arguments_record(
    call: &PendingToolCall,
    tool: &ChatToolDefinition,
    round: u8,
    error: String,
) -> ToolCallRecord {
    let now = chrono::Local::now().timestamp();
    ToolCallRecord {
        id: call.id.clone(),
        name: tool.name.clone(),
        source: tool.source.clone(),
        server_id: tool.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: ToolCallStatus::Error,
        result_preview: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round,
        sensitive: false,
        artifacts: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_chat_tool_call(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    round: u8,
    tool: &ChatToolDefinition,
    call: PendingToolCall,
    skill_cache: &mut skills::SkillRunCache,
) -> (ToolCallRecord, String) {
    let now = chrono::Local::now().timestamp();
    let mut record = ToolCallRecord {
        id: call.id.clone(),
        name: tool.name.clone(),
        source: tool.source.clone(),
        server_id: tool.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: ToolCallStatus::Pending,
        result_preview: None,
        error: None,
        duration_ms: None,
        started_at: Some(now),
        completed_at: None,
        round,
        sensitive: tool.sensitive,
        artifacts: Vec::new(),
    };
    emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);

    let settings = state.settings_read().clone();
    let requires_approval = if builtin_tool_bypasses_approval(tool) {
        false
    } else {
        match settings.chat_tools.approval_policy.as_str() {
            "auto" => false,
            "always_confirm" => true,
            _ => tool.sensitive,
        }
    };
    if requires_approval {
        let approved = request_tool_approval(
            app,
            state,
            conversation_id,
            run_id,
            message_id,
            generation,
            &record,
        )
        .await;
        if !approved {
            record.status = ToolCallStatus::Skipped;
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some("Tool call was not approved".to_string());
            emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);
            let content = record.error.clone().unwrap_or_default();
            return (record, content);
        }
    }

    record.status = ToolCallStatus::Running;
    emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);
    let started = Instant::now();
    let timeout_ms = effective_tool_timeout_ms(&settings, tool, &call.arguments);
    let result = tokio::select! {
        result = timeout(
            Duration::from_millis(timeout_ms),
            mcp::registry::call_tool(app, state, tool, call.arguments.clone(), Some(skill_cache)),
        ) => result,
        _ = wait_for_chat_cancel(state, conversation_id, generation) => {
            record.status = ToolCallStatus::Cancelled;
            record.duration_ms = Some(started.elapsed().as_millis() as u64);
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some("Tool call cancelled".to_string());
            emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);
            let content = record.error.clone().unwrap_or_default();
            return (record, content);
        }
    };
    record.duration_ms = Some(started.elapsed().as_millis() as u64);
    record.completed_at = Some(chrono::Local::now().timestamp());
    let max_tool_output_chars = settings.chat_tools.max_tool_output_chars;
    let tool_content = match result {
        Ok(Ok(output)) if !output.is_error => {
            record.status = ToolCallStatus::Success;
            record.artifacts = output.artifacts.clone();
            record.result_preview = Some(truncate_chars(
                &format_tool_result_preview(&output.content),
                max_tool_output_chars,
            ));
            truncate_tool_content_for_model(&output.content, max_tool_output_chars)
        }
        Ok(Ok(output)) => {
            record.status = ToolCallStatus::Error;
            record.error = Some(truncate_chars(&output.content, 1000));
            truncate_tool_content_for_model(&output.content, max_tool_output_chars)
        }
        Ok(Err(err)) => {
            record.status = ToolCallStatus::Error;
            record.error = Some(err.clone());
            truncate_tool_content_for_model(&err, max_tool_output_chars)
        }
        Err(_) => {
            record.status = ToolCallStatus::Error;
            let err =
                format!("工具调用超时（{timeout_ms}ms）。请缩小任务，或在设置中调高工具超时时间。");
            record.error = Some(err.clone());
            err
        }
    };
    emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);
    (record, tool_content)
}

fn effective_tool_timeout_ms(
    settings: &Settings,
    tool: &ChatToolDefinition,
    arguments: &Value,
) -> u64 {
    let default_timeout_ms = settings.chat_tools.tool_timeout_ms;
    if tool.source == "skill" && tool.name == "skill_run_script" {
        return crate::mcp::registry::effective_skill_script_timeout_ms(
            default_timeout_ms,
            arguments.get("timeout_ms").and_then(|value| value.as_u64()),
        );
    }
    if tool.source == "native" && matches!(tool.name.as_str(), "run_command" | "run_python") {
        return arguments
            .get("timeout_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(default_timeout_ms)
            .clamp(1_000, 300_000)
            .max(default_timeout_ms);
    }
    default_timeout_ms
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

async fn wait_for_chat_cancel(state: &AppState, conversation_id: &str, generation: u64) {
    while state.is_chat_generation_active(conversation_id, generation) {
        sleep(Duration::from_millis(100)).await;
    }
}

fn emit_chat_tool_record(
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
        }),
    );
}

fn emit_chat_stream_delta(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    delta: &str,
    reasoning_delta: Option<&str>,
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
        }),
    );
}

fn emit_chat_stream_done(
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

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn truncate_tool_content_for_model(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str(&format!(
        "\n\n[Tool output truncated: original {char_count} chars, showing first {max_chars}.]"
    ));
    truncated
}

fn format_chat_missing_api_key_error(provider_name: &str) -> String {
    let provider = provider_name.trim();
    if provider.is_empty() {
        "Chat 模型供应商缺少 API Key，请到设置 > 模型中填写后再发送。".to_string()
    } else {
        format!("Chat 模型供应商「{provider}」缺少 API Key，请到设置 > 模型中填写后再发送。")
    }
}

fn chat_missing_model_error() -> String {
    "请先为当前 Chat 对话选择模型，或到设置 > AI 客户端配置默认模型。".to_string()
}

fn format_tool_approval_summary(record: &ToolCallRecord) -> String {
    let parsed = serde_json::from_str::<Value>(&record.arguments).ok();
    let mut lines = Vec::new();
    match record.name.as_str() {
        "run_command" => {
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
        "write_file" | "edit_file" | "read_file" => {
            if let Some(path) = parsed
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Path: {path}"));
            }
            if record.name == "edit_file" {
                if let Some(old) = parsed
                    .as_ref()
                    .and_then(|value| value.get("old"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
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

/// UI/storage preview for Tavily-style JSON stdout (`answer` is often null for search).
fn format_tool_result_preview(content: &str) -> String {
    let trimmed = content.trim();
    let json_str = trimmed
        .strip_prefix("stdout:")
        .map(str::trim)
        .unwrap_or(trimmed);
    let Ok(value) = serde_json::from_str::<Value>(json_str) else {
        return content.to_string();
    };

    if let Some(answer) = value
        .get("answer")
        .and_then(|answer| answer.as_str())
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
    {
        return format!("答: {answer}");
    }

    let Some(results) = value.get("results").and_then(|results| results.as_array()) else {
        return content.to_string();
    };

    if results.is_empty() {
        return "无搜索结果".to_string();
    }

    let query = value
        .get("query")
        .and_then(|query| query.as_str())
        .unwrap_or_default();
    let query_label = if query.is_empty() {
        String::new()
    } else {
        format!("「{query}」")
    };
    let first = &results[0];
    let title = first
        .get("title")
        .or_else(|| first.get("url"))
        .and_then(|title| title.as_str())
        .unwrap_or_default();
    let snippet = first
        .get("content")
        .or_else(|| first.get("raw_content"))
        .and_then(|content| content.as_str())
        .unwrap_or_default();
    let snippet: String = snippet.chars().take(80).collect();
    let head = format!("{} 条结果{query_label}", results.len());
    if title.is_empty() && snippet.is_empty() {
        return head;
    }
    if snippet.is_empty() {
        return format!("{head}: {title}");
    }
    format!("{head}: {title} — {snippet}")
}

fn image_content_part(path: &PathBuf) -> Result<serde_json::Value, String> {
    let bytes = fs::read(path).map_err(|e| format!("读取图片附件失败: {e}"))?;
    let base64 = general_purpose::STANDARD.encode(bytes);
    let mime = image_mime_for_path(path);
    Ok(serde_json::json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{mime};base64,{base64}") },
    }))
}

fn image_mime_for_path(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        _ => "image/png",
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
    conversation.messages[idx].content = trimmed.to_string();
    conversation.messages[idx].timestamp = chrono::Local::now().timestamp();
    let context_state = compute_context_state(&app, &state, &conversation, None, &[]).await?;
    conversation.context_state = context_state.clone();
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_context_state(&app, &conversation.id, &context_state);

    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
}

/// 重新生成助手回复（移除该条及之后的消息，再基于此前上下文请求新回复）
#[tauri::command]
pub(crate) async fn chat_regenerate_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let idx = find_message_index(&conversation, &message_id)?;
    if conversation.messages[idx].role != "assistant" {
        return Err("仅支持重新生成助手回复".to_string());
    }

    mark_summary_stale_if_needed(&mut conversation, idx);
    conversation.messages.truncate(idx);
    if conversation.messages.last().map(|m| m.role.as_str()) != Some("user") {
        return Err("缺少对应的用户消息，无法重新生成".to_string());
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

    let last_user_api_content = conversation
        .messages
        .last()
        .filter(|message| message.role == "user")
        .map(|message| compose_user_content_for_api(&message.content, &message.attachments));
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
    match complete_assistant_reply(
        &app,
        &state,
        &mut conversation,
        None,
        last_user_api_content.as_deref(),
        &last_user_image_paths,
        None,
    )
    .await
    {
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
    conversation.messages.remove(idx);
    let context_state = compute_context_state(&app, &state, &conversation, None, &[]).await?;
    conversation.context_state = context_state.clone();
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_context_state(&app, &conversation.id, &context_state);

    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
    }))
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
    active_skill_id: Option<String>,
    assistant_id: Option<String>,
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
    }
    let provider_model_changed = provider_id.is_some() || model.is_some();
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
            conversation.active_skill_id = snapshot.skill_id.clone();
            conversation.assistant_id = Some(snapshot.id.clone());
            conversation.assistant_snapshot = Some(snapshot);
        }
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

    if provider_model_changed {
        let updated_settings = {
            let mut settings = state.settings_write();
            settings.default_models.chat.provider_id = conversation.provider_id.clone();
            settings.default_models.chat.model = conversation.model.clone();
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

    #[test]
    fn attachment_type_detects_images_case_insensitively() {
        assert_eq!(attachment_type_for_name("screenshot.PNG"), "image");
        assert_eq!(attachment_type_for_name("scan.tif"), "image");
        assert_eq!(attachment_type_for_name("photo.heic"), "image");
        assert_eq!(attachment_type_for_name("notes.pdf"), "file");
    }

    #[test]
    fn sanitize_attachment_name_removes_path_like_characters() {
        assert_eq!(sanitize_attachment_name("../secret?.png"), "secret_.png");
        assert_eq!(sanitize_attachment_name("   "), "attachment");
    }

    #[test]
    fn compose_user_content_for_api_mentions_attachment_names() {
        let content = compose_user_content_for_api(
            "看看这个",
            &[Attachment {
                id: "att_1".to_string(),
                attachment_type: "image".to_string(),
                name: "screen.png".to_string(),
                path: "att_1-screen.png".to_string(),
            }],
        );

        assert!(content.contains("看看这个"));
        assert!(content.contains("screen.png"));
        assert!(content.contains("图片附件会随本轮请求发送给视觉模型"));
    }

    #[test]
    fn title_source_uses_attachment_name_when_content_empty() {
        let title = title_source_for_user_message(
            "",
            &[Attachment {
                id: "att_1".to_string(),
                attachment_type: "file".to_string(),
                name: "notes.pdf".to_string(),
                path: "att_1-notes.pdf".to_string(),
            }],
        );

        assert_eq!(title, "附件: notes.pdf");
    }

    #[test]
    fn merge_reasoning_keeps_planning_and_final_sections() {
        let reasoning = merge_reasoning(
            &[
                "  planning round one  ".to_string(),
                String::new(),
                "planning round two".to_string(),
            ],
            Some(" final answer reasoning ".to_string()),
        )
        .expect("reasoning should be merged");

        assert_eq!(
            reasoning,
            "planning round one\n\nplanning round two\n\nfinal answer reasoning"
        );
    }

    #[test]
    fn generate_title_truncates_unicode_safely() {
        let title = generate_title("附件: 这是一张非常非常非常非常非常非常非常长的图片文件名.png");

        assert!(title.ends_with("..."));
        assert!(title.chars().count() <= 33);
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
    fn is_tools_unsupported_error_detects_provider_rejection_messages() {
        assert!(is_tools_unsupported_error(
            "Chat tools planning Error: 400 Bad Request - tools not supported (attempt 1/3)"
        ));
        assert!(is_tools_unsupported_error(
            "Chat tools planning Error: 422 Unprocessable Entity - invalid tool_choice (attempt 1/1)"
        ));
        assert!(is_tools_unsupported_error(
            "Chat tools planning Error: 400 Bad Request - function call is not supported (attempt 1/3)"
        ));
        assert!(is_tools_unsupported_error(
            "Chat tools planning Error: 400 Bad Request - tools[0]: unknown variant `function`, expected `web_search_20250305` or `web_search_20260209` (attempt 1/1)"
        ));
        assert!(!is_tools_unsupported_error(
            "Chat tools planning Error: 429 Too Many Requests - rate limited (attempt 1/3)"
        ));
        assert!(!is_tools_unsupported_error("network timeout"));
    }

    #[test]
    fn is_native_skill_tool_name_matches_runtime_tools() {
        assert!(is_native_skill_tool_name("skill_activate"));
        assert!(is_native_skill_tool_name("skill_run_script"));
        assert!(!is_native_skill_tool_name("web_search"));
    }

    #[test]
    fn format_tool_result_preview_summarizes_tavily_search_json() {
        let raw = r#"stdout:
{
  "answer": null,
  "query": "吉林市 明天 天气",
  "results": [
    {
      "title": "吉林市天气预报",
      "content": "明天晴有时多云，最高33℃"
    }
  ]
}"#;
        let preview = format_tool_result_preview(raw);
        assert!(preview.contains("1 条结果"));
        assert!(preview.contains("吉林市天气预报"));
        assert!(preview.contains("33"));
        assert!(!preview.contains("\"answer\": null"));
    }

    #[test]
    fn truncate_tool_content_for_model_marks_truncated_output() {
        let content = "abcdef";
        let truncated = truncate_tool_content_for_model(content, 3);

        assert!(truncated.starts_with("abc"));
        assert!(truncated.contains("Tool output truncated"));
        assert!(truncated.contains("original 6 chars"));
        assert!(truncated.contains("first 3"));
    }

    #[test]
    fn truncate_tool_content_for_model_keeps_short_output_unchanged() {
        assert_eq!(truncate_tool_content_for_model("abc", 3), "abc");
    }

    #[test]
    fn format_tool_approval_summary_highlights_run_command() {
        let record = ToolCallRecord {
            id: "call_1".to_string(),
            name: "run_command".to_string(),
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
            name: "write_file".to_string(),
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
        };

        let summary = format_tool_approval_summary(&record);
        assert!(summary.contains("Path: /tmp/project/out.txt"));
        assert!(summary.contains("Raw arguments"));
    }

    #[test]
    fn extract_tool_calls_parses_dsml_when_api_tool_calls_missing() {
        const SAMPLE: &str = concat!(
            "<|DSML|tool_calls><|DSML|invoke name=\"skill_run_script\">",
            "<|DSML|parameter name=\"name\" string=\"true\">tavily-multi-key</|DSML|parameter>",
            "<|DSML|parameter name=\"relative_path\" string=\"true\">scripts/tavily_cli.py</|DSML|parameter>",
            "</|DSML|invoke></|DSML|tool_calls>",
        );
        let message = serde_json::json!({
            "role": "assistant",
            "content": SAMPLE,
        });
        let calls = extract_tool_calls(&message);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_name, "skill_run_script");
        assert_eq!(
            calls[0]
                .arguments
                .get("name")
                .and_then(|value| value.as_str()),
            Some("tavily-multi-key")
        );
    }

    #[test]
    fn chat_prompt_omits_disabled_web_tools() {
        let registry = skills::SkillRegistry::default();
        let mut chat_tools = crate::settings::ChatToolsConfig::default();
        chat_tools.native_tools.skill_runtime = true;
        chat_tools.native_tools.run_python = true;
        chat_tools.native_tools.web_search = false;
        chat_tools.native_tools.web_fetch = false;

        let prompt = build_chat_system_prompt(
            "zh-CN",
            false,
            false,
            &registry,
            &chat_tools,
            true,
            &["run_python".to_string()],
            None,
            None,
            None,
            "",
        );

        assert!(prompt.contains("run_python"));
        assert!(!prompt.contains("web_search"));
        assert!(!prompt.contains("web_fetch"));
    }

    fn test_assistant_snapshot(tool_preset: &str, skill_id: Option<&str>) -> ChatAssistantSnapshot {
        ChatAssistantSnapshot {
            id: "asst_test".to_string(),
            name: "Test Assistant".to_string(),
            description: String::new(),
            system_prompt: String::new(),
            provider_id: String::new(),
            model: String::new(),
            skill_id: skill_id.map(str::to_string),
            tool_preset: tool_preset.to_string(),
            conversation_starters: Vec::new(),
            greeting: String::new(),
        }
    }

    fn test_mcp_tool() -> ChatToolDefinition {
        ChatToolDefinition {
            id: "mcp__demo__search".to_string(),
            name: "search".to_string(),
            description: "Search demo".to_string(),
            source: "mcp".to_string(),
            server_id: Some("demo".to_string()),
            server_name: Some("Demo".to_string()),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            sensitive: false,
        }
    }

    #[test]
    fn assistant_tool_preset_none_disables_all_tools() {
        let assistant = test_assistant_snapshot("none", Some("doc"));
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        apply_assistant_tool_preset(&mut tools, Some(&assistant));

        assert!(tools.is_empty());
    }

    #[test]
    fn assistant_tool_preset_skills_keeps_only_skill_runtime_tools() {
        let assistant = test_assistant_snapshot("skills", Some("doc"));
        let mut tools = vec![
            crate::mcp::types::native_skill_activate_tool(),
            crate::mcp::types::native_skill_read_file_tool(),
            crate::mcp::types::native_web_fetch_tool(),
            test_mcp_tool(),
        ];

        apply_assistant_tool_preset(&mut tools, Some(&assistant));

        assert_eq!(tools.len(), 2);
        assert!(tools.iter().all(|tool| tool.source == "skill"));
        assert!(tools.iter().any(|tool| tool.name == "skill_activate"));
        assert!(tools.iter().any(|tool| tool.name == "skill_read_file"));
    }

    #[test]
    fn assistant_tool_preset_inherit_and_all_leave_tools_unchanged() {
        for preset in ["inherit", "all", "unexpected"] {
            let assistant = test_assistant_snapshot(preset, None);
            let mut tools = vec![
                crate::mcp::types::native_skill_activate_tool(),
                crate::mcp::types::native_web_fetch_tool(),
                test_mcp_tool(),
            ];

            apply_assistant_tool_preset(&mut tools, Some(&assistant));

            assert_eq!(tools.len(), 3, "preset {preset} should not filter tools");
        }
    }

    #[test]
    fn skill_fallback_switches_to_markdown_when_assistant_disables_tools() {
        let mut chat_tools = crate::settings::ChatToolsConfig::default();

        apply_skill_fallback_when_tools_unavailable(&mut chat_tools, Some("doc"), false);

        assert_eq!(chat_tools.skill_fallback_mode, "skill_md_only");
    }

    #[test]
    fn empty_assistant_response_error_exposes_flow_failure() {
        let response = empty_assistant_response_error("Chat stream");

        assert_eq!(response, "Chat stream returned an empty assistant response");
    }

    #[test]
    fn chat_stream_text_parser_accepts_delta_chunks() {
        let value = serde_json::json!({
            "choices": [{ "delta": { "content": "你" } }]
        });
        let mut full = String::new();
        let (text, mode) = extract_chat_stream_text(&value).expect("delta content");

        assert_eq!(mode, ChatStreamTextMode::Delta);
        assert_eq!(
            append_chat_stream_text(&mut full, text, mode),
            Some("你".to_string())
        );
        assert_eq!(full, "你");
    }

    #[test]
    fn chat_stream_text_parser_converts_message_snapshots_to_deltas() {
        let first = serde_json::json!({
            "choices": [{ "message": { "content": "你" } }]
        });
        let second = serde_json::json!({
            "choices": [{ "message": { "content": "你好" } }]
        });
        let repeat = serde_json::json!({
            "choices": [{ "message": { "content": "你好" } }]
        });
        let mut full = String::new();

        let (text, mode) = extract_chat_stream_text(&first).expect("first snapshot");
        assert_eq!(mode, ChatStreamTextMode::Snapshot);
        assert_eq!(
            append_chat_stream_text(&mut full, text, mode),
            Some("你".to_string())
        );

        let (text, mode) = extract_chat_stream_text(&second).expect("second snapshot");
        assert_eq!(
            append_chat_stream_text(&mut full, text, mode),
            Some("好".to_string())
        );

        let (text, mode) = extract_chat_stream_text(&repeat).expect("repeat snapshot");
        assert_eq!(append_chat_stream_text(&mut full, text, mode), None);
        assert_eq!(full, "你好");
    }

    #[test]
    fn chat_stream_reasoning_parser_accepts_message_snapshots() {
        let value = serde_json::json!({
            "choices": [{ "message": { "reasoning_content": "思考中" } }]
        });
        let mut full = String::new();
        let (text, mode) = extract_chat_stream_reasoning(&value).expect("reasoning snapshot");

        assert_eq!(mode, ChatStreamTextMode::Snapshot);
        assert_eq!(
            append_chat_stream_text(&mut full, text, mode),
            Some("思考中".to_string())
        );
    }

    #[test]
    fn skill_run_script_timeout_uses_minimum_even_when_model_requests_less() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::native_skill_run_script_tool();
        let arguments = serde_json::json!({ "timeout_ms": 60_000 });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            120_000
        );
    }

    #[test]
    fn skill_run_script_timeout_clamps_large_model_requests() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::native_skill_run_script_tool();
        let arguments = serde_json::json!({ "timeout_ms": 500_000 });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            300_000
        );
    }

    #[test]
    fn disabled_builtin_tool_feedback_is_hidden_model_feedback() {
        let feedback = disabled_builtin_tool_feedback("web_search")
            .expect("disabled builtin tools should produce model feedback");

        assert!(feedback.contains("not enabled"));
        assert!(feedback.contains("web_search"));
        assert!(disabled_builtin_tool_feedback("mcp__server__tool").is_none());
    }

    #[test]
    fn extract_openai_tool_calls_preserves_invalid_arguments_error() {
        let message = serde_json::json!({
            "role": "assistant",
            "tool_calls": [{
                "id": "call_write",
                "type": "function",
                "function": {
                    "name": "write_file",
                    "arguments": "{\"path\":\"/tmp/out.html\",\"content\":\"unterminated"
                }
            }]
        });

        let calls = extract_openai_tool_calls(&message);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_name, "write_file");
        assert!(calls[0].arguments.is_null());
        assert!(calls[0]
            .arguments_parse_error
            .as_deref()
            .unwrap_or_default()
            .contains("invalid or incomplete"));
    }

    #[test]
    fn patch_system_message_replaces_first_system_entry() {
        let mut messages = vec![
            serde_json::json!({ "role": "system", "content": "old" }),
            serde_json::json!({ "role": "user", "content": "hi" }),
        ];
        patch_system_message(&mut messages, "new prompt");
        assert_eq!(
            messages[0].get("content").and_then(|value| value.as_str()),
            Some("new prompt")
        );
    }

    #[test]
    fn final_assistant_api_message_omits_reasoning_without_tool_calls() {
        let message = final_assistant_api_message("done", None);

        assert_eq!(
            message.get("role").and_then(|value| value.as_str()),
            Some("assistant")
        );
        assert_eq!(
            message.get("content").and_then(|value| value.as_str()),
            Some("done")
        );
        assert!(message.get("reasoning_content").is_none());
    }

    #[test]
    fn final_assistant_api_message_keeps_reasoning_when_requested() {
        let message = final_assistant_api_message("done", Some(" thinking "));

        assert_eq!(
            message
                .get("reasoning_content")
                .and_then(|value| value.as_str()),
            Some("thinking")
        );
    }

    #[test]
    fn assistant_content_from_api_message_trims_missing_or_null_content() {
        assert_eq!(
            assistant_content_from_api_message(&serde_json::json!({
                "role": "assistant",
                "content": " answer "
            })),
            "answer"
        );
        assert_eq!(
            assistant_content_from_api_message(&serde_json::json!({
                "role": "assistant",
                "content": null
            })),
            ""
        );
    }

    fn test_chat_message(id: &str, role: &str, content: &str, timestamp: i64) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            reasoning: None,
            tool_calls: Vec::new(),
            api_messages: Vec::new(),
            model_messages: Vec::new(),
            active_skill_id: None,
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
        }
    }

    #[test]
    fn estimate_tokens_counts_ascii_and_cjk() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("你好ab"), 3);
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
                    tool_calls: Vec::new(),
                    api_messages: Vec::new(),
                    model_messages: Vec::new(),
                    active_skill_id: None,
                    timestamp: 1,
                },
                ChatMessage {
                    id: "msg_assistant_1".to_string(),
                    role: "assistant".to_string(),
                    content: "visible answer".to_string(),
                    attachments: Vec::new(),
                    reasoning: Some("hidden thinking".to_string()),
                    tool_calls: Vec::new(),
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
            context_state: ConversationContextState::default(),
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
                    tool_calls: Vec::new(),
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
            context_state: ConversationContextState::default(),
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
}
