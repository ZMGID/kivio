use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_shell::ShellExt;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::api::{extract_status_code, send_with_failover};
use crate::apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
use crate::mcp::{self, ChatToolDefinition};
use crate::settings::{
    chat_no_think_instruction, default_chat_system_prompt, persist_settings,
};
use crate::skills;
use crate::state::AppState;
use crate::utils;

use super::storage::{
    conversation_attachments_dir, delete_conversation as delete_conv,
    get_conversations as get_convs, load_conversation, save_conversation,
};
use super::{Attachment, ChatMessage, Conversation, ToolCallRecord, ToolCallStatus};

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
        active_skill_id: None,
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
        active_skill_id: None,
        timestamp: chrono::Local::now().timestamp(),
    };

    conversation.messages.push(user_message.clone());
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

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
            conversation
                .messages
                .retain(|message| message.id != user_message.id);
            conversation.updated_at = chrono::Local::now().timestamp();
            save_conversation(&app, &conversation)?;
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
) -> Result<(), String> {
    let sender = state
        .pending_python_runs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    if let Some(sender) = sender {
        let _ = sender.send((content, is_error));
    }
    Ok(())
}

const MAX_ATTACHMENT_PREVIEW_BYTES: u64 = 12 * 1024 * 1024;

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
    app.shell()
        .open(path_str, None)
        .map_err(|e| e.to_string())
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
        return Err("Missing API Key".to_string());
    }
    if conversation.model.trim().is_empty() {
        return Err("Please select a model first".to_string());
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
    let skill_id = resolve_forced_skill_id(
        &settings.chat_tools,
        &skill_registry,
        active_skill_id,
    );
    let active_skill_record = skill_id.as_deref().and_then(|id| skill_registry.find(id)).cloned();
    let active_skill_detail = skill_id.as_deref().and_then(|id| {
        skills::read_skill_detail(app, &settings.chat_tools.skill_scan_paths, id).ok()
    });
    let mut effective_chat_tools = settings.chat_tools.clone();
    let tools_capable = chat_tools_capable(&provider, &effective_chat_tools);
    if !tools_capable && effective_chat_tools.skill_fallback_mode == "progressive" {
        if skill_id.is_some() {
            effective_chat_tools.skill_fallback_mode = "skill_md_only".to_string();
        }
    }
    let system_prompt = build_chat_system_prompt(
        &language,
        !last_user_image_paths.is_empty(),
        thinking_enabled,
        &skill_registry,
        &effective_chat_tools,
        tools_capable,
        skill_id.as_deref(),
        active_skill_detail.as_ref(),
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
            conversation,
            assistant_message_id,
            response,
            None,
            Vec::new(),
            Vec::new(),
            skill_id.as_deref(),
            title_from_first_user,
        )?;
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
    let mut tools = if provider.supports_tools
        && (settings.chat_tools.enabled || crate::settings::chat_native_tools_enabled(&settings.chat_tools))
    {
        let tools = mcp::registry::list_enabled_tool_defs(state.inner())
            .await
            .unwrap_or_default();
        tools
    } else {
        Vec::new()
    };
    if let Some(skill) = active_skill_record.as_ref() {
        if !skill.allowed_tools.is_empty() {
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
    }
    let max_rounds = settings.chat_tools.max_tool_rounds.max(1);
    let mut provider_tools_unsupported = false;

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
                if !tool_records.is_empty() {
                    push_assistant_message(
                        app,
                        conversation,
                        assistant_message_id,
                        String::new(),
                        None,
                        tool_records,
                        generated_api_messages,
                        skill_id.as_deref(),
                        title_from_first_user,
                    )?;
                }
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
                    if !tool_records.is_empty() {
                        push_assistant_message(
                            app,
                            conversation,
                            assistant_message_id,
                            String::new(),
                            None,
                            tool_records,
                            generated_api_messages,
                            skill_id.as_deref(),
                            title_from_first_user,
                        )?;
                    }
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
            let tool_calls = extract_tool_calls(&message);
            if tool_calls.is_empty() {
                let mut response =
                    crate::chat::dsml_tools::strip_dsml_tool_markup(&assistant_content_from_api_message(&message));
                let reasoning = merge_reasoning(&planning_reasoning_parts, None);
                if !response.is_empty() {
                    emit_chat_stream_delta(
                        app,
                        &conversation.id,
                        &run_id,
                        &assistant_message_id,
                        &response,
                        None,
                    );
                }
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
                push_assistant_message(
                    app,
                    conversation,
                    assistant_message_id,
                    response,
                    reasoning,
                    tool_records,
                    generated_api_messages,
                    skill_id.as_deref(),
                    title_from_first_user,
                )?;
                return Ok(());
            }

            let assistant_message = assistant_api_message_for_tool_calls(&message, &tool_calls);
            runtime_messages.push(assistant_message);
            generated_api_messages.push(runtime_messages.last().cloned().unwrap_or(Value::Null));
            for tool_call in tool_calls {
                let Some(tool) = match_tool_call(&tools, &tool_call.function_name) else {
                    let error = format!("Unknown tool requested: {}", tool_call.function_name);
                    let record = unknown_tool_record(&tool_call, round + 1, error);
                    emit_chat_tool_record(
                        app,
                        &conversation.id,
                        &run_id,
                        &assistant_message_id,
                        &record,
                    );
                    let tool_message = serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_call.id,
                        "content": record.error.clone().unwrap_or_default(),
                    });
                    runtime_messages.push(tool_message.clone());
                    generated_api_messages.push(tool_message);
                    tool_records.push(record);
                    continue;
                };
                let tool_call_id = tool_call.id.clone();
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
        if !provider_tools_unsupported {
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
                    conversation,
                    assistant_message_id,
                    stored_content,
                    reasoning,
                    tool_records,
                    generated_api_messages,
                    skill_id.as_deref(),
                    title_from_first_user,
                )?;
            }
            return Err("cancelled".to_string());
        }
        let final_reasoning_for_api = stream.reasoning.clone();
        let reasoning = merge_reasoning(&planning_reasoning_parts, stream.reasoning);
        let response = sanitize_assistant_text_response(&stream.content);
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
            &message
                .get("content")
                .and_then(|content| content.as_str())
                .unwrap_or_default(),
        );
        let reasoning = merge_reasoning(
            &planning_reasoning_parts,
            extract_reasoning_content(&message),
        );
        if !response.is_empty() {
            emit_chat_stream_delta(
                app,
                &conversation.id,
                &run_id,
                &assistant_message_id,
                &response,
                None,
            );
        }
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
        conversation,
        assistant_message_id,
        response,
        reasoning,
        tool_records,
        generated_api_messages,
        skill_id.as_deref(),
        title_from_first_user,
    )?;
    Ok(())
}

fn push_assistant_message(
    app: &AppHandle,
    conversation: &mut Conversation,
    message_id: String,
    content: String,
    reasoning: Option<String>,
    tool_calls: Vec<ToolCallRecord>,
    api_messages: Vec<Value>,
    active_skill_id: Option<&str>,
    title_from_first_user: Option<&str>,
) -> Result<(), String> {
    conversation.messages.push(ChatMessage {
        id: message_id,
        role: "assistant".to_string(),
        content,
        attachments: vec![],
        reasoning,
        tool_calls,
        api_messages,
        active_skill_id: active_skill_id.map(|id| id.to_string()),
        timestamp: chrono::Local::now().timestamp(),
    });

    if let Some(user_content) = title_from_first_user {
        if conversation.messages.len() == 2 && conversation.title == "新对话" {
            conversation.title = generate_title(user_content);
        }
    }

    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(app, conversation)?;
    Ok(())
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
    active_skill_id: Option<&str>,
    active_skill_detail: Option<&skills::SkillDetail>,
    custom_system_prompt: &str,
) -> String {
    let mut prompt = if custom_system_prompt.trim().is_empty() {
        default_chat_system_prompt(language, has_image)
    } else {
        custom_system_prompt.trim().to_string()
    };
    prompt.push_str(&crate::settings::chat_current_datetime_context(language));

    if tools_available {
        prompt.push_str("\n\nYou have access to tools (functions). When the user's request requires action—such as activating a skill, reading a file, running a script, or searching the web—YOU MUST call the appropriate tool instead of describing what to do. Never say \"I cannot run commands\" or \"you can do it yourself\" when a tool is available for that action.");
        if language.starts_with("zh") {
            prompt.push_str(
                " 若用户只问今天/明天/星期几等可由上文「当前本地时间」直接推算的日期问题，直接回答，不要调用 skill_activate 或 web_search。",
            );
        } else {
            prompt.push_str(
                " If the user only asks for today/tomorrow/weekday derivable from the system local time above, answer directly without skill_activate or web_search.",
            );
        }
        append_native_tools_prompt(&mut prompt, chat_tools, language);
    }

    let include_catalog = chat_tools.skill_auto_match
        || active_skill_id.is_some()
        || chat_tools.skill_fallback_mode != "legacy_full_body";
    if include_catalog {
        let catalog = skills::format_catalog(
            registry,
            active_skill_id,
            tools_available,
            |skill_id| crate::settings::is_skill_enabled(chat_tools, skill_id),
        );
        if !catalog.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&catalog);
        }
    }

    if !chat_tools.skill_auto_match {
        prompt.push_str(
            "\n\nOnly activate skills that are enabled in Settings (listed in the catalog below).",
        );
    }

    let fallback = chat_tools.skill_fallback_mode.as_str();
    if let Some(skill_id) = active_skill_id.filter(|id| !id.trim().is_empty()) {
        prompt.push_str("\n\nUser pinned skill for this message: ");
        prompt.push_str(skill_id);
        if tools_available {
            prompt.push_str(
                ". Call skill_activate with this name only because the user pinned it; otherwise prefer Kivio built-in tools when they fit.",
            );
        } else if matches!(fallback, "skill_md_only" | "legacy_full_body") {
            prompt.push_str(". Follow the Active Skill instructions below.");
        } else {
            prompt.push_str(
                ". Progressive skill loading requires tool support; switch provider or set fallback to SKILL.md only.",
            );
        }
    } else if tools_available && chat_tools.skill_auto_match {
        if language.starts_with("zh") {
            prompt.push_str(
                "\n\nSkill 目录仅供参考：仅当用户明确需要某个 Skill 的能力（或点名 Skill 名称）时才 skill_activate。泛泛的「用 Python」「写代码」「搜一下」等应优先使用 Kivio 内置工具（如 run_python、read_file、web_search、web_fetch），不要只因 Skill 描述里提到 Python/脚本就激活无关 Skill。",
            );
        } else {
            prompt.push_str(
                "\n\nThe skill catalog is optional: call skill_activate only when the user clearly needs that skill (or names it). For generic requests like \"use Python\", coding, or web lookup, prefer Kivio built-in tools (run_python, read_file, web_search, web_fetch) instead of activating an unrelated skill just because its description mentions Python or scripts.",
            );
        }
    }

    if matches!(fallback, "skill_md_only" | "legacy_full_body") {
        if let Some(skill) = active_skill_detail {
            if !skill.body.trim().is_empty() {
                prompt.push_str("\n\nActive Skill:\n");
                prompt.push_str(&skill.body);
            }
        }
    }

    if !thinking_enabled && !tools_available {
        prompt.push_str(chat_no_think_instruction(language));
    }
    prompt
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

/// Kivio 内置工具（Skill 三件套 + 只读联网）始终自动执行，不走审批弹窗。
fn builtin_tool_bypasses_approval(tool: &ChatToolDefinition) -> bool {
    (tool.source == "skill" && is_native_skill_tool_name(&tool.name))
        || (tool.source == "native"
            && matches!(tool.name.as_str(), "web_search" | "web_fetch"))
}

fn append_native_tools_prompt(prompt: &mut String, chat_tools: &crate::settings::ChatToolsConfig, language: &str) {
    let native = &chat_tools.native_tools;
    let mut lines: Vec<&str> = Vec::new();
    if native.read_file {
        lines.push("read_file");
    }
    if native.write_file {
        lines.push("write_file");
    }
    if native.edit_file {
        lines.push("edit_file");
    }
    if native.run_command {
        lines.push("run_command");
    }
    if native.run_python {
        lines.push("run_python");
    }
    if native.web_search {
        lines.push("web_search");
    }
    if native.web_fetch {
        lines.push("web_fetch");
    }
    if lines.is_empty() {
        return;
    }
    let list = lines.join(", ");
    if language.starts_with("zh") {
        prompt.push_str("\n\nKivio 内置工具（已启用）：");
        prompt.push_str(&list);
        prompt.push_str(
            "。文件路径须在用户主目录内；可选工作区根目录进一步收紧。write_file、edit_file、run_command 会请求用户确认；run_python 在 Pyodide 沙盒中运行。用户要用 Python 跑代码/计算时优先 run_python，不要用 skill_run_script，除非用户点名某个 Skill。",
        );
    } else {
        prompt.push_str("\n\nKivio built-in tools enabled: ");
        prompt.push_str(&list);
        prompt.push_str(
            ". Paths must stay under the user home directory (optional workspace roots further restrict). write_file, edit_file, and run_command require user approval; run_python runs in a Pyodide sandbox. For generic Python requests, use run_python—not skill_run_script—unless the user named a specific skill.",
        );
    }
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
        active_skill_id,
        active_skill_detail,
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

    for (idx, message) in conversation.messages.iter().enumerate() {
        let content = if Some(idx) == last_user_idx {
            last_user_api_content.unwrap_or(message.content.as_str())
        } else {
            message.content.as_str()
        };
        if Some(idx) == last_user_idx && !last_user_image_paths.is_empty() {
            let mut parts = last_user_image_paths
                .iter()
                .map(image_content_part)
                .collect::<Result<Vec<_>, _>>()?;
            parts.push(serde_json::json!({ "type": "text", "text": content }));
            messages.push(serde_json::json!({
                "role": message.role,
                "content": parts,
            }));
        } else {
            messages.push(serde_json::json!({
                "role": message.role,
                "content": content,
            }));
        }
        if message.role == "assistant" && !message.api_messages.is_empty() {
            messages.pop();
            messages.extend(message.api_messages.iter().cloned());
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
    for (idx, message) in conversation.messages.iter().enumerate() {
        let role = match message.role.as_str() {
            "assistant" => "Assistant",
            _ => "User",
        };
        let content = if Some(idx) == last_user_idx {
            last_user_api_content.unwrap_or(message.content.as_str())
        } else {
            message.content.as_str()
        };
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
    if provider.api_format == "anthropic" {
        return call_anthropic_message(state, provider, model, messages, tools, retry_attempts, label).await;
    }

    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": 0.7,
        "max_tokens": 2000,
    });
    if let Some(tools) = tools.filter(|tools| !tools.is_empty()) {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(ChatToolDefinition::to_openai_tool)
                .collect(),
        );
    }
    if !thinking_enabled && utils::provider_supports_thinking_field(&provider.base_url) {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    }

    let response = send_with_failover(
        state,
        label,
        retry_attempts,
        &provider.id,
        &provider.api_keys,
        |key| {
            state
                .http
                .post(url.clone())
                .bearer_auth(key)
                .json(&body)
                .send()
        },
    )
    .await?;
    let raw = response
        .text()
        .await
        .map_err(|err| format!("{label} read body: {err}"))?;
    let value: Value = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "{label} parse JSON: {} (body: {})",
            err,
            raw.chars().take(500).collect::<String>()
        )
    })?;
    value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .cloned()
        .ok_or_else(|| {
            format!(
                "Invalid {label} response: {}",
                raw.chars().take(500).collect::<String>()
            )
        })
}

/// Anthropic Messages API 非流式调用（用于工具规划轮次）。
/// 将 OpenAI 格式的消息和工具转换为 Anthropic 格式，发送请求，再将响应转回 OpenAI 兼容格式。
async fn call_anthropic_message(
    state: &State<'_, AppState>,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    label: &str,
) -> Result<Value, String> {
    use crate::anthropic_adapter;

    let (system_prompt, anthropic_messages) = anthropic_adapter::convert_messages_to_anthropic(&messages);

    let url = anthropic_adapter::build_anthropic_url(&provider.base_url);
    let mut body = serde_json::json!({
        "model": model,
        "messages": anthropic_messages,
        "max_tokens": 2000,
    });
    if !system_prompt.is_empty() {
        body["system"] = Value::String(system_prompt);
    }
    if let Some(tools) = tools.filter(|t| !t.is_empty()) {
        let openai_tools: Vec<Value> = tools.iter().map(ChatToolDefinition::to_openai_tool).collect();
        let anthropic_tools = anthropic_adapter::convert_tools_to_anthropic(&openai_tools);
        if !anthropic_tools.is_empty() {
            body["tools"] = Value::Array(anthropic_tools);
        }
    }

    let response = send_with_failover(
        state,
        label,
        retry_attempts,
        &provider.id,
        &provider.api_keys,
        |key| {
            let headers = anthropic_adapter::build_anthropic_headers(key)
                .unwrap_or_default();
            state
                .http
                .post(url.clone())
                .headers(headers)
                .json(&body)
                .send()
        },
    )
    .await?;

    let raw = response
        .text()
        .await
        .map_err(|err| format!("{label} read body: {err}"))?;
    let anthropic_response: Value = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "{label} parse JSON: {} (body: {})",
            err,
            raw.chars().take(500).collect::<String>()
        )
    })?;

    // 检查 Anthropic 错误
    if let Some(error) = anthropic_response.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown Anthropic error");
        return Err(format!("{label}: {msg}"));
    }

    // 将 Anthropic 响应转换为 OpenAI 兼容格式
    let parsed = anthropic_adapter::parse_anthropic_response(&anthropic_response);

    let mut message = serde_json::json!({
        "role": "assistant",
        "content": if parsed.content.is_empty() { Value::Null } else { Value::String(parsed.content) },
        "finish_reason": parsed.finish_reason,
    });
    if let Some(reasoning) = parsed.reasoning {
        message["reasoning_content"] = Value::String(reasoning);
    }
    if !parsed.tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(
            parsed
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.function_name,
                            "arguments": tc.arguments_raw,
                        }
                    })
                })
                .collect(),
        );
    }

    Ok(message)
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
    if provider.api_format == "anthropic" {
        return stream_anthropic_completion(app, state, provider, model, messages, retry_attempts, conversation_id, run_id, message_id, generation).await;
    }

    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": 0.7,
        "max_tokens": 2000,
        "stream": true,
    });
    if !thinking_enabled && utils::provider_supports_thinking_field(&provider.base_url) {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    }
    let mut response = send_with_failover(
        state,
        "Chat stream",
        retry_attempts,
        &provider.id,
        &provider.api_keys,
        |key| {
            state
                .http
                .post(url.clone())
                .bearer_auth(key)
                .json(&body)
                .send()
        },
    )
    .await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "Chat stream HTTP {}: {}",
            status.as_u16(),
            text.chars().take(500).collect::<String>()
        ));
    }

    let mut buffer = String::new();
    let mut full = String::new();
    let mut reasoning_full = String::new();
    loop {
        if !state.is_chat_generation_active(conversation_id, generation) {
            emit_chat_stream_done(
                app,
                conversation_id,
                run_id,
                message_id,
                "cancelled",
                full.trim(),
            );
            return Ok(ChatStreamOutput::new(
                full.trim().to_string(),
                reasoning_full.trim().to_string(),
                true,
            ));
        }
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(err) => {
                emit_chat_stream_done(
                    app,
                    conversation_id,
                    run_id,
                    message_id,
                    "error",
                    full.trim(),
                );
                return Err(err.to_string());
            }
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buffer.find('\n') {
            let line: String = buffer.drain(..=pos).collect();
            let line = line.trim();
            if !line.starts_with("data:") {
                continue;
            }
            let data = line.trim_start_matches("data:").trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                let cleaned = sanitize_assistant_text_response(full.trim());
                emit_chat_stream_done(
                    app,
                    conversation_id,
                    run_id,
                    message_id,
                    "done",
                    &cleaned,
                );
                return Ok(ChatStreamOutput::new(
                    cleaned,
                    reasoning_full.trim().to_string(),
                    false,
                ));
            }
            let value: Value = match serde_json::from_str(data) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let delta = value
                .get("choices")
                .and_then(|choices| choices.get(0))
                .and_then(|choice| choice.get("delta"));
            if let Some(reasoning) = delta
                .and_then(|delta| {
                    delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning"))
                })
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
            {
                reasoning_full.push_str(reasoning);
                emit_chat_stream_delta(
                    app,
                    conversation_id,
                    run_id,
                    message_id,
                    "",
                    Some(reasoning),
                );
            }
            if let Some(content) = delta
                .and_then(|delta| delta.get("content"))
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
            {
                full.push_str(content);
                emit_chat_stream_delta(app, conversation_id, run_id, message_id, content, None);
            }
        }
    }
    let cleaned = sanitize_assistant_text_response(full.trim());
    emit_chat_stream_done(
        app,
        conversation_id,
        run_id,
        message_id,
        "done",
        &cleaned,
    );
    Ok(ChatStreamOutput::new(
        cleaned,
        reasoning_full.trim().to_string(),
        false,
    ))
}

/// Anthropic Messages API 流式调用（用于最终回复）。
/// 解析 Anthropic SSE 事件格式，转换为 OpenAI 兼容的流式输出。
#[allow(clippy::too_many_arguments)]
async fn stream_anthropic_completion(
    app: &AppHandle,
    state: &State<'_, AppState>,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    retry_attempts: usize,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
) -> Result<ChatStreamOutput, String> {
    use crate::anthropic_adapter;

    let (system_prompt, anthropic_messages) = anthropic_adapter::convert_messages_to_anthropic(&messages);

    let url = anthropic_adapter::build_anthropic_url(&provider.base_url);
    let mut body = serde_json::json!({
        "model": model,
        "messages": anthropic_messages,
        "max_tokens": 2000,
        "stream": true,
    });
    if !system_prompt.is_empty() {
        body["system"] = Value::String(system_prompt);
    }

    let mut response = send_with_failover(
        state,
        "Anthropic stream",
        retry_attempts,
        &provider.id,
        &provider.api_keys,
        |key| {
            let headers = anthropic_adapter::build_anthropic_headers(key)
                .unwrap_or_default();
            state
                .http
                .post(url.clone())
                .headers(headers)
                .json(&body)
                .send()
        },
    )
    .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "Anthropic stream HTTP {}: {}",
            status.as_u16(),
            text.chars().take(500).collect::<String>()
        ));
    }

    let mut buffer = String::new();
    let mut full = String::new();
    let mut reasoning_full = String::new();
    // 用于追踪当前 tool_use 块
    let mut current_tool_id = String::new();
    let mut current_tool_name = String::new();
    let mut current_tool_input_parts: Vec<String> = Vec::new();

    loop {
        if !state.is_chat_generation_active(conversation_id, generation) {
            emit_chat_stream_done(app, conversation_id, run_id, message_id, "cancelled", full.trim());
            return Ok(ChatStreamOutput::new(full.trim().to_string(), reasoning_full.trim().to_string(), true));
        }

        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(err) => {
                emit_chat_stream_done(app, conversation_id, run_id, message_id, "error", full.trim());
                return Err(err.to_string());
            }
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let line: String = buffer.drain(..=pos).collect();
            let event = anthropic_adapter::parse_anthropic_sse_event(&line);
            match event {
                Some(anthropic_adapter::AnthropicSseEvent::TextDelta(text)) => {
                    full.push_str(&text);
                    emit_chat_stream_delta(app, conversation_id, run_id, message_id, &text, None);
                }
                Some(anthropic_adapter::AnthropicSseEvent::ThinkingDelta(thinking)) => {
                    reasoning_full.push_str(&thinking);
                    emit_chat_stream_delta(app, conversation_id, run_id, message_id, "", Some(&thinking));
                }
                Some(anthropic_adapter::AnthropicSseEvent::ToolUseStart { id, name }) => {
                    current_tool_id = id;
                    current_tool_name = name;
                    current_tool_input_parts.clear();
                }
                Some(anthropic_adapter::AnthropicSseEvent::ToolInputDelta(json)) => {
                    current_tool_input_parts.push(json);
                }
                Some(anthropic_adapter::AnthropicSseEvent::ContentBlockStop) => {
                    // tool_use 块结束，但我们不在流式中处理 tool calls
                    // tool calls 在非流式 planning 路径中处理
                    current_tool_id.clear();
                    current_tool_name.clear();
                    current_tool_input_parts.clear();
                }
                Some(anthropic_adapter::AnthropicSseEvent::MessageStop) => {
                    emit_chat_stream_done(app, conversation_id, run_id, message_id, "done", full.trim());
                    return Ok(ChatStreamOutput::new(full.trim().to_string(), reasoning_full.trim().to_string(), false));
                }
                Some(anthropic_adapter::AnthropicSseEvent::MessageStopWithReason(_)) => {
                    emit_chat_stream_done(app, conversation_id, run_id, message_id, "done", full.trim());
                    return Ok(ChatStreamOutput::new(full.trim().to_string(), reasoning_full.trim().to_string(), false));
                }
                Some(anthropic_adapter::AnthropicSseEvent::Error(err)) => {
                    emit_chat_stream_done(app, conversation_id, run_id, message_id, "error", full.trim());
                    return Err(format!("Anthropic stream error: {err}"));
                }
                None => {}
            }
        }
    }

    emit_chat_stream_done(app, conversation_id, run_id, message_id, "done", full.trim());
    Ok(ChatStreamOutput::new(full.trim().to_string(), reasoning_full.trim().to_string(), false))
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

#[derive(Debug, Clone)]
pub(crate) struct PendingToolCall {
    pub(crate) id: String,
    pub(crate) function_name: String,
    pub(crate) arguments: Value,
    pub(crate) arguments_raw: String,
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
    message
        .get("tool_calls")
        .and_then(|value| value.as_array())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let function = call.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();
                    let arguments_raw = function
                        .get("arguments")
                        .and_then(|value| value.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    let arguments = serde_json::from_str(&arguments_raw).unwrap_or(Value::Null);
                    Some(PendingToolCall {
                        id: call
                            .get("id")
                            .and_then(|value| value.as_str())
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| format!("tool_{}", Uuid::new_v4())),
                        function_name: name,
                        arguments,
                        arguments_raw,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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
            }
        })
        .collect()
}

fn assistant_api_message_for_tool_calls(
    message: &Value,
    tool_calls: &[PendingToolCall],
) -> Value {
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
    let timeout_ms = settings.chat_tools.tool_timeout_ms;
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
    let tool_content = match result {
        Ok(Ok(output)) if !output.is_error => {
            record.status = ToolCallStatus::Success;
            record.result_preview = Some(truncate_chars(
                &format_tool_result_preview(&output.content),
                settings.chat_tools.max_tool_output_chars,
            ));
            output.content
        }
        Ok(Ok(output)) => {
            record.status = ToolCallStatus::Error;
            record.error = Some(truncate_chars(&output.content, 1000));
            output.content
        }
        Ok(Err(err)) => {
            record.status = ToolCallStatus::Error;
            record.error = Some(err.clone());
            err
        }
        Err(_) => {
            record.status = ToolCallStatus::Error;
            let err = "Tool call timed out".to_string();
            record.error = Some(err.clone());
            err
        }
    };
    emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);
    (record, tool_content)
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
            "argumentsPreview": truncate_chars(&record.arguments, 800),
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
pub(crate) fn chat_update_message(
    app: AppHandle,
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

    conversation.messages[idx].content = trimmed.to_string();
    conversation.messages[idx].timestamp = chrono::Local::now().timestamp();
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

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
pub(crate) fn chat_delete_message(
    app: AppHandle,
    conversation_id: String,
    message_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let idx = find_message_index(&conversation, &message_id)?;
    if conversation.messages[idx].role != "assistant" {
        return Err("仅支持删除助手回复".to_string());
    }

    conversation.messages.remove(idx);
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;

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
    if let Some(skill_id) = active_skill_id {
        let trimmed = skill_id.trim();
        conversation.active_skill_id = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
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
                    active_skill_id: Some("doc".to_string()),
                    timestamp: 2,
                },
            ],
            active_skill_id: Some("doc".to_string()),
            created_at: 1,
            updated_at: 2,
            pinned: false,
            folder: None,
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
}
