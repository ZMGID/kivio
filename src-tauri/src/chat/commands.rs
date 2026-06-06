use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_shell::ShellExt;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
use crate::chat::agent::{prepare as agent_prepare, stop as agent_stop};
use crate::chat::model::{
    generate_request_from_openai_messages, model_messages_from_openai_messages,
    openai_messages_from_model_messages, AnthropicMessagesProvider, AppleLocalProvider,
    GenerateOptions, GenerateOutput, LanguageModelProvider, MessagePart, ModelMessage, ModelRole,
    OpenAiChatProvider,
};
use crate::mcp::types::ChatToolArtifact;
use crate::mcp::{self, ChatToolDefinition};
use crate::settings::{persist_settings, ModelInfo, ModelProvider, ProviderApiFormat, Settings};
use crate::skills;
use crate::state::AppState;

use super::storage::{
    archive_assistant, assistant_snapshot, conversation_attachments_dir, create_assistant,
    create_project, delete_conversation as delete_conv, delete_project, duplicate_assistant,
    find_reusable_blank_conversation, get_assistants, get_conversations as get_convs, get_projects,
    load_conversation, save_conversation, update_assistant, update_project,
};
use super::{
    Attachment, ChatAssistant, ChatMessage, ContextUsageSegment, Conversation,
    ConversationContextState, ConversationContextSummary, ToolCallRecord, ToolCallStatus,
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
    let assistant_id_for_reuse = assistant_snapshot
        .as_ref()
        .map(|assistant| assistant.id.clone());

    let conversation = {
        let _create_guard = state
            .chat_create_conversation_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(conversation) = find_reusable_blank_conversation(
            &app,
            &provider_id,
            &model,
            folder.as_deref(),
            assistant_id_for_reuse.as_deref(),
        )? {
            conversation
        } else {
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
            conversation
        }
    };

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
        crate::chat::agent::AgentRunEntry::Send,
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
const IMAGE_ATTACHMENT_TOKEN_ESTIMATE: usize = 1_600;
const AUXILIARY_VISION_RESULT_TOKEN_ESTIMATE: usize = 800;

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
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsm" => "application/vnd.ms-excel.sheet.macroenabled.12",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
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

fn attachment_extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

fn attachment_skill_for_name(name: &str) -> Option<&'static str> {
    match attachment_extension(name).as_str() {
        "pdf" => Some("pdf"),
        "doc" | "docx" => Some("docx"),
        "xls" | "xlsx" | "xlsm" | "csv" | "tsv" => Some("xlsx"),
        _ => None,
    }
}

fn attachment_format_label(attachment: &Attachment) -> &'static str {
    if attachment.attachment_type == "image" {
        return "图片";
    }

    match attachment_extension(&attachment.name).as_str() {
        "pdf" => "PDF",
        "doc" | "docx" => "Word 文档",
        "xls" | "xlsx" | "xlsm" => "Excel 工作簿",
        "csv" => "CSV 表格",
        "tsv" => "TSV 表格",
        "txt" | "md" => "文本文件",
        _ => attachment_type_label(&attachment.attachment_type),
    }
}

fn stored_attachment_path_for_prompt(
    attachment: &Attachment,
    attachment_dir: Option<&Path>,
) -> String {
    attachment_dir
        .map(|dir| dir.join(&attachment.path).display().to_string())
        .unwrap_or_else(|| attachment.path.clone())
}

fn attachment_processing_hint(attachment: &Attachment) -> String {
    if attachment.attachment_type == "image" {
        return "图片附件会随本轮请求发送给视觉模型。".to_string();
    }

    if let Some(skill) = attachment_skill_for_name(&attachment.name) {
        format!(
            "推荐复用现成 `{skill}` Skill：需要读取或分析该文件时，先调用 skill_activate(name=\"{skill}\")，再按该 Skill 的 SKILL.md / reference / scripts 流程处理安全副本路径。"
        )
    } else {
        "此文件已保存为 Kivio 安全副本；仅在有可用读取工具或对应 Skill 时处理正文。".to_string()
    }
}

fn compose_user_content_for_api(
    content: &str,
    attachments: &[Attachment],
    attachment_dir: Option<&Path>,
) -> String {
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
            let stored_path = stored_attachment_path_for_prompt(attachment, attachment_dir);
            format!(
                "- {} ({})\n  - 附件 ID：{}\n  - Kivio 安全副本路径：{}\n  - 处理建议：{}",
                attachment.name,
                attachment_format_label(attachment),
                attachment.id,
                stored_path,
                attachment_processing_hint(attachment)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let capability_note = match (has_images, has_files) {
        (true, true) => {
            "图片附件会随本轮请求发送给视觉模型；文档/表格附件不会直接随模型请求内联正文，必须复用对应 Agent Skill 或可用工具实际读取安全副本后再分析。"
        }
        (true, false) => "图片附件会随本轮请求发送给视觉模型。",
        (false, true) => {
            "文档/表格附件不会直接随模型请求内联正文，必须复用对应 Agent Skill 或可用工具实际读取安全副本后再分析；不要仅凭文件名臆测内容。"
        }
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
    entry: crate::chat::agent::AgentRunEntry,
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
    let use_auxiliary_vision =
        should_route_images_through_auxiliary_vision(&settings, last_user_image_paths);
    let mut auxiliary_tool_records = Vec::new();
    let auxiliary_vision_result = if use_auxiliary_vision {
        let mut record = auxiliary_vision_tool_record(&settings, last_user_image_paths.len());
        let started = Instant::now();
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
    let tools_capable = agent_prepare::chat_tools_capable(&provider, &effective_chat_tools);
    let mut tools = list_tools_for_chat(state.inner(), &settings, provider.supports_tools).await;
    agent_prepare::apply_assistant_tool_preset(
        &mut tools,
        conversation.assistant_snapshot.as_ref(),
    );
    if let Some(skill) = active_skill_record.as_ref() {
        agent_prepare::apply_active_skill_tool_filter(&mut tools, skill);
    }
    let tools_available = tools_capable && !tools.is_empty();
    agent_prepare::apply_skill_fallback_when_tools_unavailable(
        &mut effective_chat_tools,
        skill_id.as_deref(),
        tools_available,
    );
    let available_builtin_tools = agent_prepare::available_builtin_tool_names(&tools);
    let system_prompt = agent_prepare::build_chat_system_prompt(
        &language,
        !main_image_paths.is_empty(),
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
        if !main_image_paths.is_empty() {
            return Err(
                "Apple Intelligence 暂不支持图片附件，请为 AI 对话配置云端视觉 provider".into(),
            );
        }
        let prompt = build_apple_chat_prompt(
            &system_prompt,
            conversation,
            last_user_idx,
            last_user_content_for_main,
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
            auxiliary_tool_records,
            Vec::new(),
            skill_id.as_deref(),
            title_from_first_user,
        )
        .await?;
        return Ok(());
    }

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
        settings.chat.system_prompt.as_str(),
    );

    let host = ChatAgentHost {
        app: app.clone(),
        state: state.inner(),
    };
    let executor = RegistryToolExecutor {
        app: app.clone(),
        state: state.inner(),
    };
    let result = crate::chat::agent::run_agent_loop(
        crate::chat::agent::AgentRunConfig {
            entry,
            state: state.inner(),
            conversation_id: conversation.id.clone(),
            run_id: run_id.clone(),
            message_id: assistant_message_id.clone(),
            generation: run_generation,
            provider,
            model: conversation.model.clone(),
            runtime_messages,
            tools,
            settings: settings.clone(),
            effective_chat_tools,
            language,
            has_image: !main_image_paths.is_empty(),
            thinking_enabled,
            stream_enabled,
            retry_attempts,
            skill_registry,
            active_skill_id: skill_id.clone(),
            active_skill_detail,
            assistant_snapshot: conversation.assistant_snapshot.clone(),
            custom_system_prompt: settings.chat.system_prompt.clone(),
            provider_tools_fallback_system_prompt,
        },
        &host,
        &executor,
    )
    .await?;

    let mut tool_records = auxiliary_tool_records;
    tool_records.extend(result.tool_records);
    push_assistant_message(
        app,
        state,
        &settings,
        conversation,
        assistant_message_id,
        result.content,
        result.reasoning,
        tool_records,
        result.api_messages,
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
        model_messages: assistant_model_messages_for_storage(
            &content,
            reasoning.as_deref(),
            &api_messages,
            &tool_calls,
        ),
        tool_calls,
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
        agent_stop::assistant_content_from_api_message(&message)
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

fn context_window_from_model_info(info: Option<&ModelInfo>) -> Option<usize> {
    info.and_then(|info| info.context_window)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn model_database_entries() -> Option<&'static serde_json::Map<String, Value>> {
    static MODEL_DATABASE: OnceLock<Value> = OnceLock::new();
    MODEL_DATABASE
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../../src/data/modelDatabase.json"))
                .unwrap_or(Value::Null)
        })
        .as_object()
}

fn model_database_context_window(model: &str) -> Option<usize> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }

    let entries = model_database_entries()?;
    let name = model.to_ascii_lowercase();
    let stripped = name.rsplit('/').next().unwrap_or(&name);

    if let Some(tokens) = context_window_from_database_entry(entries.get(stripped)) {
        return Some(tokens);
    }

    entries
        .iter()
        .filter_map(|(key, entry)| {
            if key == "_meta" || !stripped.starts_with(key) || key.len() >= stripped.len() {
                return None;
            }
            context_window_from_database_entry(Some(entry)).map(|tokens| (key.len(), tokens))
        })
        .max_by_key(|(key_len, _)| *key_len)
        .map(|(_, tokens)| tokens)
        .or_else(|| {
            entries
                .iter()
                .filter_map(|(key, entry)| {
                    if key == "_meta" || key == stripped || !stripped.contains(key) {
                        return None;
                    }
                    context_window_from_database_entry(Some(entry))
                        .map(|tokens| (key.len(), tokens))
                })
                .max_by_key(|(key_len, _)| *key_len)
                .map(|(_, tokens)| tokens)
        })
}

fn context_window_from_database_entry(entry: Option<&Value>) -> Option<usize> {
    entry?
        .get("contextWindow")
        .and_then(Value::as_u64)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn context_window_for_model(provider: Option<&ModelProvider>, model: &str) -> (usize, bool) {
    if let Some(tokens) = context_window_from_model_info(
        provider.and_then(|provider| provider.model_overrides.get(model)),
    ) {
        return (tokens, false);
    }
    if let Some(tokens) = model_database_context_window(model) {
        return (tokens, false);
    }

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
        Value::String(text) => agent_prepare::estimate_tokens(text),
        Value::Array(items) => items.iter().map(count_tokens_in_value).sum(),
        Value::Object(map) => {
            if let Some(kind) = map.get("type").and_then(|value| value.as_str()) {
                match kind {
                    "image_url" | "input_image" | "image" => return 0,
                    "text" | "input_text" => {
                        return map.get("text").map(count_tokens_in_value).unwrap_or(0);
                    }
                    _ => {}
                }
            }
            map.iter()
                .map(|(key, value)| {
                    agent_prepare::estimate_tokens(key) + count_tokens_in_value(value)
                })
                .sum()
        }
        _ => agent_prepare::estimate_tokens(&value.to_string()),
    }
}

fn ceil_div_u32(value: u32, divisor: u32) -> usize {
    value.div_ceil(divisor) as usize
}

fn estimate_openai_tile_image_tokens(
    width: u32,
    height: u32,
    base_tokens: usize,
    tile_tokens: usize,
) -> usize {
    let mut scaled_width = width.max(1) as f64;
    let mut scaled_height = height.max(1) as f64;
    let longest = scaled_width.max(scaled_height);
    if longest > 2048.0 {
        let scale = 2048.0 / longest;
        scaled_width *= scale;
        scaled_height *= scale;
    }
    let shortest = scaled_width.min(scaled_height);
    if shortest > 768.0 {
        let scale = 768.0 / shortest;
        scaled_width *= scale;
        scaled_height *= scale;
    }
    let tiles = (scaled_width / 512.0).ceil().max(1.0) as usize
        * (scaled_height / 512.0).ceil().max(1.0) as usize;
    base_tokens + tiles * tile_tokens
}

fn estimate_openai_patch_image_tokens(
    width: u32,
    height: u32,
    patch_budget: usize,
    multiplier: f64,
    max_dimension: u32,
) -> usize {
    let patch_budget = patch_budget.max(1);
    let width = width.max(1);
    let height = height.max(1);
    let original_patches = ceil_div_u32(width, 32) * ceil_div_u32(height, 32);
    let mut scale = 1.0_f64;
    let longest = width.max(height);
    if longest > max_dimension.max(1) {
        scale = scale.min(max_dimension.max(1) as f64 / longest as f64);
    }
    if original_patches > patch_budget {
        let pixel_budget = patch_budget as f64 * 32.0 * 32.0;
        let shrink_factor = (pixel_budget / (width as f64 * height as f64)).sqrt();
        let target_width_patches = (width as f64 * shrink_factor) / 32.0;
        let target_height_patches = (height as f64 * shrink_factor) / 32.0;
        let width_adjust = target_width_patches.floor().max(1.0) / target_width_patches.max(1.0);
        let height_adjust = target_height_patches.floor().max(1.0) / target_height_patches.max(1.0);
        scale = scale.min(shrink_factor * width_adjust.min(height_adjust));
    }
    let mut scaled_width = ((width as f64 * scale).floor() as u32).max(1);
    let mut scaled_height = ((height as f64 * scale).floor() as u32).max(1);
    while ceil_div_u32(scaled_width, 32) * ceil_div_u32(scaled_height, 32) > patch_budget
        || scaled_width.max(scaled_height) > max_dimension.max(1)
    {
        scaled_width = ((scaled_width as f64 * 0.99).floor() as u32).max(1);
        scaled_height = ((scaled_height as f64 * 0.99).floor() as u32).max(1);
    }
    let patches = ceil_div_u32(scaled_width, 32) * ceil_div_u32(scaled_height, 32);
    (patches as f64 * multiplier).ceil() as usize
}

fn estimate_anthropic_image_tokens(model: &str, width: u32, height: u32) -> usize {
    let lower = model.to_ascii_lowercase();
    let high_resolution_opus = lower.contains("opus")
        && (lower.contains("4.7")
            || lower.contains("4-7")
            || lower.contains("4.8")
            || lower.contains("4-8"));
    let cap = if high_resolution_opus { 4_784 } else { 1_600 };
    ((width.max(1) as f64 * height.max(1) as f64) / 750.0)
        .ceil()
        .min(cap as f64) as usize
}

fn estimate_gemini_image_tokens(width: u32, height: u32) -> usize {
    if width <= 384 && height <= 384 {
        return 258;
    }
    let tiles = ceil_div_u32(width.max(1), 768) * ceil_div_u32(height.max(1), 768);
    tiles.max(1) * 258
}

fn provider_image_estimator_descriptor(provider: Option<&ModelProvider>, model: &str) -> String {
    let Some(provider) = provider else {
        return model.to_ascii_lowercase();
    };
    format!(
        "{} {} {} {}",
        provider.name, provider.base_url, provider.api_format, model
    )
    .to_ascii_lowercase()
}

fn estimate_image_tokens_for_dimensions(
    provider: Option<&ModelProvider>,
    model: &str,
    width: u32,
    height: u32,
) -> usize {
    // Provider docs meter image context by pixels/tiles, not by base64 payload bytes.
    let descriptor = provider_image_estimator_descriptor(provider, model);
    if provider
        .map(|provider| provider.api_format_kind() == ProviderApiFormat::AnthropicMessages)
        .unwrap_or(false)
        || descriptor.contains("anthropic")
        || descriptor.contains("claude")
    {
        return estimate_anthropic_image_tokens(model, width, height);
    }
    if descriptor.contains("gemini")
        || descriptor.contains("google")
        || descriptor.contains("generativelanguage.googleapis.com")
    {
        return estimate_gemini_image_tokens(width, height);
    }

    if descriptor.contains("gpt-5.4-mini")
        || descriptor.contains("gpt-5-4-mini")
        || descriptor.contains("gpt-4.1-mini")
        || descriptor.contains("gpt-4-1-mini")
        || descriptor.contains("gpt-5-mini")
    {
        return estimate_openai_patch_image_tokens(width, height, 1_536, 1.62, 2_048);
    }
    if descriptor.contains("gpt-5.4-nano")
        || descriptor.contains("gpt-5-4-nano")
        || descriptor.contains("gpt-4.1-nano")
        || descriptor.contains("gpt-4-1-nano")
        || descriptor.contains("gpt-5-nano")
    {
        return estimate_openai_patch_image_tokens(width, height, 1_536, 2.46, 2_048);
    }
    if descriptor.contains("o4-mini") {
        return estimate_openai_patch_image_tokens(width, height, 1_536, 1.72, 2_048);
    }
    if descriptor.contains("gpt-5.5") || descriptor.contains("gpt-5-5") {
        return estimate_openai_patch_image_tokens(width, height, 10_000, 1.0, 6_000);
    }
    if descriptor.contains("gpt-5.4") || descriptor.contains("gpt-5-4") {
        return estimate_openai_patch_image_tokens(width, height, 2_500, 1.0, 2_048);
    }
    if descriptor.contains("gpt-4o-mini") {
        return estimate_openai_tile_image_tokens(width, height, 2_833, 5_667);
    }
    if descriptor.contains("gpt-5") {
        return estimate_openai_tile_image_tokens(width, height, 70, 140);
    }
    if descriptor.contains("o1") || descriptor.contains("o3") {
        return estimate_openai_tile_image_tokens(width, height, 75, 150);
    }
    if descriptor.contains("computer-use") {
        return estimate_openai_tile_image_tokens(width, height, 65, 129);
    }
    estimate_openai_tile_image_tokens(width, height, 85, 170)
}

fn estimate_image_tokens_for_path(
    provider: Option<&ModelProvider>,
    model: &str,
    path: &Path,
) -> usize {
    match image::image_dimensions(path) {
        Ok((width, height)) => estimate_image_tokens_for_dimensions(provider, model, width, height),
        Err(_) => IMAGE_ATTACHMENT_TOKEN_ESTIMATE,
    }
}

fn estimate_image_attachment_tokens(
    provider: Option<&ModelProvider>,
    model: &str,
    image_paths: &[PathBuf],
) -> usize {
    image_paths
        .iter()
        .map(|path| estimate_image_tokens_for_path(provider, model, path))
        .sum()
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
        color: agent_prepare::context_segment_color(id).map(str::to_string),
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
    agent_prepare::merge_context_segments(segments)
}

fn estimate_messages_segments(
    conversation: &Conversation,
    messages: &[Value],
    attachment_tokens: usize,
) -> Vec<ContextUsageSegment> {
    let mut segments = Vec::new();
    let summary_tokens = active_summary(conversation)
        .map(|summary| agent_prepare::estimate_tokens(&summary.content))
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
        attachment_tokens,
    );
    agent_prepare::merge_context_segments(segments)
}

fn should_route_images_through_auxiliary_vision(
    settings: &Settings,
    image_paths: &[PathBuf],
) -> bool {
    !image_paths.is_empty() && settings.has_explicit_vision_model()
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
    agent_prepare::apply_assistant_tool_preset(
        &mut tools,
        conversation.assistant_snapshot.as_ref(),
    );
    if let Some(skill) = active_skill_id
        .as_deref()
        .and_then(|id| skill_registry.find(id))
    {
        agent_prepare::apply_active_skill_tool_filter(&mut tools, skill);
    }
    let available_builtin_tools = agent_prepare::available_builtin_tool_names(&tools);
    let tools_available = tools_capable && !tools.is_empty();
    agent_prepare::apply_skill_fallback_when_tools_unavailable(
        &mut effective_chat_tools,
        active_skill_id.as_deref(),
        tools_available,
    );

    let route_images_through_auxiliary_vision =
        should_route_images_through_auxiliary_vision(&settings, last_user_image_paths);
    let empty_image_paths: &[PathBuf] = &[];
    let main_image_paths = if route_images_through_auxiliary_vision {
        empty_image_paths
    } else {
        last_user_image_paths
    };
    let attachment_tokens = if route_images_through_auxiliary_vision {
        last_user_image_paths.len() * AUXILIARY_VISION_RESULT_TOKEN_ESTIMATE
    } else {
        estimate_image_attachment_tokens(provider.as_ref(), &conversation.model, main_image_paths)
    };

    let (system_prompt, mut segments) = agent_prepare::build_chat_system_prompt_with_segments(
        &language,
        !main_image_paths.is_empty(),
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
        main_image_paths,
    )?;
    segments.extend(estimate_messages_segments(
        conversation,
        &request_messages,
        attachment_tokens,
    ));

    if tools_available {
        segments.extend(estimate_tool_segments(&tools));
    }

    let segments = agent_prepare::merge_context_segments(segments);
    let estimated_input_tokens = segments
        .iter()
        .map(|segment| segment.estimated_tokens)
        .sum::<usize>();
    let (context_window_tokens, context_window_estimated) =
        context_window_for_model(provider.as_ref(), &conversation.model);
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
    let token_estimate_before = agent_prepare::estimate_tokens(&source_text);
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
        agent_stop::assistant_content_from_api_message(&message)
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
        token_estimate_after: agent_prepare::estimate_tokens(&summary_content),
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

struct AuxiliaryVisionResult {
    provider_name: String,
    model: String,
    content: String,
}

fn auxiliary_vision_tool_record(settings: &Settings, image_count: usize) -> ToolCallRecord {
    let (provider_id, model) = settings.effective_vision_model();
    let provider_name = settings
        .get_provider(&provider_id)
        .map(|provider| provider.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| provider_id.clone());
    ToolCallRecord {
        id: format!("call_mixer_vision_{}", Uuid::new_v4()),
        name: "mixer_vision".to_string(),
        source: "mixer".to_string(),
        server_id: Some(format!("{provider_name} / {model}")),
        arguments: serde_json::json!({
            "task": "vision",
            "provider": provider_name,
            "model": model,
            "images": image_count,
        })
        .to_string(),
        status: ToolCallStatus::Running,
        result_preview: None,
        error: None,
        duration_ms: None,
        started_at: Some(chrono::Local::now().timestamp()),
        completed_at: None,
        round: 0,
        sensitive: false,
        artifacts: Vec::new(),
    }
}

fn finish_auxiliary_vision_tool_record(
    record: &mut ToolCallRecord,
    status: ToolCallStatus,
    started: Instant,
    result_preview: Option<String>,
    error: Option<String>,
) {
    record.status = status;
    record.duration_ms = Some(started.elapsed().as_millis() as u64);
    record.completed_at = Some(chrono::Local::now().timestamp());
    record.result_preview = result_preview;
    record.error = error;
}

async fn analyze_chat_images_with_auxiliary_model(
    state: &State<'_, AppState>,
    settings: &Settings,
    last_user_api_content: Option<&str>,
    image_paths: &[PathBuf],
    retry_attempts: usize,
    language: &str,
) -> Result<AuxiliaryVisionResult, String> {
    if image_paths.is_empty() {
        return Err("No image attachments to analyze".to_string());
    }
    let (provider_id, model) = settings.effective_vision_model();
    let provider = settings
        .get_provider(&provider_id)
        .ok_or_else(|| "Vision auxiliary provider not found".to_string())?
        .clone();
    if provider.api_format_kind() == ProviderApiFormat::AppleLocal {
        return Err(
            "Apple Intelligence 暂不支持图片附件，请在「混音器」里为视觉副任务选择云端视觉模型。"
                .to_string(),
        );
    }
    if provider.api_keys.is_empty() {
        return Err(format_chat_missing_api_key_error(&provider.name));
    }
    if model.trim().is_empty() {
        return Err(chat_missing_model_error());
    }

    let mut parts = image_paths
        .iter()
        .map(image_content_part)
        .collect::<Result<Vec<_>, _>>()?;
    parts.push(serde_json::json!({
        "type": "text",
        "text": auxiliary_vision_user_prompt(last_user_api_content, language),
    }));
    let messages = vec![
        serde_json::json!({
            "role": "system",
            "content": auxiliary_vision_system_prompt(language),
        }),
        serde_json::json!({
            "role": "user",
            "content": parts,
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
        "Chat auxiliary vision analysis",
    )
    .await?;
    let content = agent_stop::assistant_content_from_api_message(&message);
    if content.trim().is_empty() {
        return Err("Vision auxiliary model returned an empty analysis".to_string());
    }
    Ok(AuxiliaryVisionResult {
        provider_name: provider.name,
        model,
        content,
    })
}

fn auxiliary_vision_system_prompt(language: &str) -> &'static str {
    if language.starts_with("zh") {
        "你是 Kivio 的视觉副任务模型。你的任务是读取用户提供的图片，并输出给另一个主对话模型使用的客观文字观察。只描述图片中可见的信息、文字、结构、对象、界面状态和与用户问题相关的细节；不要回答最终问题，不要编造不可见内容。"
    } else {
        "You are Kivio's auxiliary vision model. Read the user's images and produce objective textual observations for another main chat model. Describe visible information, text, layout, objects, UI state, and details relevant to the user's request. Do not answer the final question and do not invent unseen content."
    }
}

fn auxiliary_vision_user_prompt(last_user_api_content: Option<&str>, language: &str) -> String {
    let content = last_user_api_content
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    if language.starts_with("zh") {
        if content.is_empty() {
            "请分析这些图片，输出主对话模型回答用户时需要知道的视觉事实。".to_string()
        } else {
            format!(
                "用户原始消息如下。请结合图片提取主对话模型回答时需要知道的视觉事实。\n\n{content}"
            )
        }
    } else if content.is_empty() {
        "Analyze these images and output the visual facts the main chat model needs.".to_string()
    } else {
        format!(
            "The user's original message is below. Extract the visual facts the main chat model needs to answer it.\n\n{content}"
        )
    }
}

fn user_content_with_auxiliary_vision_result(
    last_user_api_content: Option<&str>,
    result: &AuxiliaryVisionResult,
    language: &str,
) -> String {
    let original = last_user_api_content
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let aux_block = if language.starts_with("zh") {
        format!(
            "[混音器视觉副任务结果]\n图片附件已由视觉模型（{} - {}）预先分析。主对话模型不能直接访问图片，请基于以下视觉观察回答用户：\n{}",
            result.provider_name,
            result.model,
            result.content.trim()
        )
    } else {
        format!(
            "[Mixer vision auxiliary result]\nThe image attachments were pre-analyzed by the vision model ({} - {}). The main chat model cannot access the images directly; answer using the visual observations below:\n{}",
            result.provider_name,
            result.model,
            result.content.trim()
        )
    };
    if original.is_empty() {
        aux_block
    } else {
        format!("{original}\n\n{aux_block}")
    }
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

struct ChatAgentHost<'a> {
    app: AppHandle,
    state: &'a AppState,
}

impl crate::chat::agent::AgentHost for ChatAgentHost<'_> {
    fn emit_stream_delta(
        &self,
        conversation_id: &str,
        run_id: &str,
        message_id: &str,
        delta: &str,
        reasoning_delta: Option<&str>,
    ) {
        emit_chat_stream_delta(
            &self.app,
            conversation_id,
            run_id,
            message_id,
            delta,
            reasoning_delta,
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

struct RegistryToolExecutor<'a> {
    app: AppHandle,
    state: &'a AppState,
}

impl crate::chat::agent::ToolExecutor for RegistryToolExecutor<'_> {
    fn call<'a>(
        &'a self,
        _ctx: &'a crate::chat::agent::ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
        skill_cache: &'a mut skills::SkillRunCache,
    ) -> crate::chat::agent::ToolExecutorFuture<'a> {
        Box::pin(async move {
            mcp::registry::call_tool(&self.app, self.state, tool, arguments, Some(skill_cache))
                .await
        })
    }
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
    match complete_assistant_reply(
        &app,
        &state,
        &mut conversation,
        None,
        last_user_api_content.as_deref(),
        &last_user_image_paths,
        None,
        crate::chat::agent::AgentRunEntry::Regenerate,
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
    use std::collections::HashMap;

    fn test_provider_with_overrides(model_overrides: HashMap<String, ModelInfo>) -> ModelProvider {
        ModelProvider {
            id: "provider".to_string(),
            name: "Provider".to_string(),
            api_keys: vec!["sk-test".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            supports_tools: true,
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides,
        }
    }

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
    fn context_window_uses_model_override_before_name_heuristic() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "deepseek-v4-flash".to_string(),
            ModelInfo {
                context_window: Some(1_048_576),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            context_window_for_model(Some(&provider), "deepseek-v4-flash"),
            (1_048_576, false)
        );
    }

    #[test]
    fn context_window_uses_builtin_model_database_defaults() {
        assert_eq!(
            context_window_for_model(None, "deepseek-v4-flash"),
            (1_048_576, false)
        );
    }

    #[test]
    fn context_window_keeps_name_heuristic_when_metadata_missing() {
        assert_eq!(
            context_window_for_model(None, "custom-200k"),
            (200_000, false)
        );
        assert_eq!(
            context_window_for_model(None, "custom-deepseek-model"),
            (128_000, true)
        );
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
            None,
        );

        assert!(content.contains("看看这个"));
        assert!(content.contains("screen.png"));
        assert!(content.contains("图片附件会随本轮请求发送给视觉模型"));
    }

    #[test]
    fn compose_user_content_for_api_recommends_document_skill() {
        let content = compose_user_content_for_api(
            "总结一下",
            &[Attachment {
                id: "att_1".to_string(),
                attachment_type: "file".to_string(),
                name: "report.PDF".to_string(),
                path: "att_1-report.PDF".to_string(),
            }],
            Some(Path::new("/Users/test/Library/Application Support/com.zmair.kivio/conversations/conv_1_attachments")),
        );

        assert!(content.contains("report.PDF"));
        assert!(content.contains("PDF"));
        assert!(content.contains("skill_activate(name=\"pdf\")"));
        assert!(content.contains("Kivio 安全副本路径"));
        assert!(content.contains("不要仅凭文件名臆测内容"));
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
            context_state: ConversationContextState::default(),
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
}
