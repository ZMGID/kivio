use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::{
    AssistantDataConnector, AssistantKnowledgeSkill, AssistantQuickCommand, ChatAssistant,
    ChatAssistantIndex, ChatAssistantSnapshot, ChatProject, ChatProjectIndex, Conversation,
    ConversationIndex, ConversationListItem,
};

const WRITE_RETRY_ATTEMPTS: usize = 3;
const LEGACY_GENERAL_ASSISTANT_SYSTEM_PROMPT: &str =
    "你是 Kivio 的通用助手。回答要清晰、直接，并在信息不足时主动说明假设。";

fn validate_conversation_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("conv_")
        && id.len() > "conv_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid conversation id: {id}"))
    }
}

fn validate_project_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("proj_")
        && id.len() > "proj_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid project id: {id}"))
    }
}

fn validate_assistant_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("asst_")
        && id.len() > "asst_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid assistant id: {id}"))
    }
}

fn atomic_write(path: &Path, content: &str, label: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} path has no parent"))?;
    fs::create_dir_all(parent).map_err(|e| format!("create {label} dir: {e}"))?;

    for attempt in 0..WRITE_RETRY_ATTEMPTS {
        let tmp_path = parent.join(format!(
            ".{}.tmp.{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("conversation"),
            attempt
        ));

        let write_result = fs::write(&tmp_path, content).and_then(|_| {
            fs::rename(&tmp_path, path).or_else(|_| {
                if path.exists() {
                    fs::remove_file(path)?;
                }
                fs::rename(&tmp_path, path)
            })
        });

        match write_result {
            Ok(()) => return Ok(()),
            Err(e) if attempt + 1 < WRITE_RETRY_ATTEMPTS => {
                let _ = fs::remove_file(&tmp_path);
                thread::sleep(Duration::from_millis(20 * (attempt as u64 + 1)));
                if e.kind() == ErrorKind::NotFound {
                    fs::create_dir_all(parent).map_err(|e| format!("create {label} dir: {e}"))?;
                }
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!("write {label} file: {e}"));
            }
        }
    }

    Err(format!("write {label} file failed"))
}

fn read_conversation_file(path: &Path, id: &str) -> Result<Conversation, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("读取对话文件失败（{id}）：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("对话文件已损坏，无法加载（{id}）：{e}"))
}

fn load_conversation_list_from_files(app: &AppHandle) -> Result<Vec<ConversationListItem>, String> {
    let dir = conversations_dir(app)?;
    let entries = fs::read_dir(&dir).map_err(|e| format!("read conversations dir: {e}"))?;
    let mut conversations = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                eprintln!("skip unreadable conversation dir entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some("index.json")
            || path.extension().and_then(|ext| ext.to_str()) != Some("json")
        {
            continue;
        }

        let id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(id) if validate_conversation_id(id).is_ok() => id,
            _ => continue,
        };

        match read_conversation_file(&path, id) {
            Ok(conversation) => conversations.push(ConversationListItem::from(&conversation)),
            Err(e) => eprintln!("skip corrupt conversation file {id}: {e}"),
        }
    }

    Ok(conversations)
}

fn load_index_or_scan(app: &AppHandle) -> Result<ConversationIndex, String> {
    match load_index(app) {
        Ok(index) => Ok(index),
        Err(e) => {
            eprintln!("conversation index unavailable, rebuilding list from files: {e}");
            Ok(ConversationIndex {
                conversations: load_conversation_list_from_files(app)?,
            })
        }
    }
}

/// 获取对话存储根目录：{app_data_dir}/conversations/
pub fn conversations_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?;
    let dir = base.join("conversations");
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create conversations dir: {e}"))?;
    }
    Ok(dir)
}

/// 获取对话索引文件路径
pub fn index_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("index.json"))
}

/// 获取项目索引文件路径。项目与对话同属 Chat 数据域，保存在 conversations 下便于备份/迁移。
pub fn projects_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("projects.json"))
}

/// 获取助手索引文件路径。助手是 Chat 数据域的一部分，与对话一起备份/迁移。
pub fn assistants_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("assistants.json"))
}

/// 获取对话文件路径
pub fn conversation_file_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_conversation_id(id)?;
    Ok(conversations_dir(app)?.join(format!("{}.json", id)))
}

/// 获取对话附件目录
#[allow(dead_code)]
pub fn conversation_attachments_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_conversation_id(id)?;
    let dir = conversations_dir(app)?.join(format!("{}_attachments", id));
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create attachments dir: {e}"))?;
    }
    Ok(dir)
}

/// 加载对话索引
pub fn load_index(app: &AppHandle) -> Result<ConversationIndex, String> {
    let path = index_file_path(app)?;
    if !path.exists() {
        return Ok(ConversationIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read index file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse index file: {e}"))
}

/// 保存对话索引
pub fn save_index(app: &AppHandle, index: &ConversationIndex) -> Result<(), String> {
    let path = index_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize index: {e}"))?;
    atomic_write(&path, &content, "index")
}

pub fn load_project_index(app: &AppHandle) -> Result<ChatProjectIndex, String> {
    let path = projects_file_path(app)?;
    if !path.exists() {
        return Ok(ChatProjectIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read projects file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse projects file: {e}"))
}

pub fn save_project_index(app: &AppHandle, index: &ChatProjectIndex) -> Result<(), String> {
    let path = projects_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize projects: {e}"))?;
    atomic_write(&path, &content, "projects")
}

pub fn load_assistant_index(app: &AppHandle) -> Result<ChatAssistantIndex, String> {
    let path = assistants_file_path(app)?;
    if !path.exists() {
        return Ok(ChatAssistantIndex {
            assistants: default_assistants(),
        });
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read assistants file: {e}"))?;
    let mut index: ChatAssistantIndex =
        serde_json::from_str(&content).map_err(|e| format!("parse assistants file: {e}"))?;
    if ensure_default_assistants(&mut index) {
        save_assistant_index(app, &index)?;
    }
    Ok(index)
}

pub fn save_assistant_index(app: &AppHandle, index: &ChatAssistantIndex) -> Result<(), String> {
    let path = assistants_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize assistants: {e}"))?;
    atomic_write(&path, &content, "assistants")
}

/// 加载对话详情
pub fn load_conversation(app: &AppHandle, id: &str) -> Result<Conversation, String> {
    let path = conversation_file_path(app, id)?;
    if !path.exists() {
        return Err(format!("对话不存在：{id}"));
    }

    read_conversation_file(&path, id)
}

/// 保存对话详情
pub fn save_conversation(app: &AppHandle, conversation: &Conversation) -> Result<(), String> {
    let path = conversation_file_path(app, &conversation.id)?;
    let content = serde_json::to_string_pretty(conversation)
        .map_err(|e| format!("serialize conversation: {e}"))?;
    atomic_write(&path, &content, "conversation")?;

    // 更新索引
    let mut index = load_index_or_scan(app)?;
    let list_item = ConversationListItem::from(conversation);

    if let Some(pos) = index
        .conversations
        .iter()
        .position(|c| c.id == conversation.id)
    {
        index.conversations[pos] = list_item;
    } else {
        index.conversations.insert(0, list_item);
    }

    save_index(app, &index)
}

pub fn save_conversation_without_index(
    app: &AppHandle,
    conversation: &Conversation,
) -> Result<(), String> {
    let path = conversation_file_path(app, &conversation.id)?;
    let content = serde_json::to_string_pretty(conversation)
        .map_err(|e| format!("serialize conversation: {e}"))?;
    atomic_write(&path, &content, "conversation")
}

/// 删除对话
pub fn delete_conversation(app: &AppHandle, id: &str) -> Result<(), String> {
    // 删除对话文件
    let path = conversation_file_path(app, id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("delete conversation file: {e}"))?;
    }

    // 删除附件目录
    let attachments_dir = conversations_dir(app)?.join(format!("{}_attachments", id));
    if attachments_dir.exists() {
        fs::remove_dir_all(&attachments_dir).map_err(|e| format!("delete attachments dir: {e}"))?;
    }

    // 更新索引
    let mut index = load_index_or_scan(app)?;
    index.conversations.retain(|c| c.id != id);
    save_index(app, &index)
}

/// 获取对话列表（分页）
pub fn get_conversations(
    app: &AppHandle,
    offset: usize,
    limit: usize,
    folder: Option<String>,
) -> Result<Vec<ConversationListItem>, String> {
    let mut index = load_index_or_scan(app)?;

    // 按 folder 筛选
    if let Some(folder_name) = folder {
        index
            .conversations
            .retain(|c| c.folder.as_deref() == Some(&folder_name));
    }

    // 按 updated_at 倒序排序（最新的在前）
    index
        .conversations
        .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    // 分页
    if offset >= index.conversations.len() {
        return Ok(vec![]);
    }
    let end = (offset + limit).min(index.conversations.len());
    Ok(index.conversations[offset..end].to_vec())
}

pub fn find_reusable_blank_conversation(
    app: &AppHandle,
    provider_id: &str,
    model: &str,
    folder: Option<&str>,
    assistant_id: Option<&str>,
) -> Result<Option<Conversation>, String> {
    let mut index = load_index_or_scan(app)?;
    index
        .conversations
        .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    for item in index.conversations {
        if item.message_count != 0 {
            continue;
        }
        if item.provider_id != provider_id || item.model != model {
            continue;
        }
        if item.folder.as_deref() != folder {
            continue;
        }
        if item.assistant_id.as_deref() != assistant_id {
            continue;
        }
        let conversation = match load_conversation(app, &item.id) {
            Ok(conversation) => conversation,
            Err(err) => {
                eprintln!("skip reusable blank conversation {}: {err}", item.id);
                continue;
            }
        };
        if conversation.messages.is_empty()
            && conversation.provider_id == provider_id
            && conversation.model == model
            && conversation.folder.as_deref() == folder
            && conversation.assistant_id.as_deref() == assistant_id
        {
            return Ok(Some(conversation));
        }
    }

    Ok(None)
}

pub fn get_projects(app: &AppHandle) -> Result<Vec<ChatProject>, String> {
    let mut project_index = load_project_index(app)?;
    let conversation_index = load_index_or_scan(app)?;
    let now = chrono::Local::now().timestamp();
    let mut changed = false;

    for folder in conversation_index
        .conversations
        .iter()
        .filter_map(|conversation| conversation.folder.as_deref())
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
    {
        if project_index
            .projects
            .iter()
            .any(|project| project.name == folder)
        {
            continue;
        }
        project_index.projects.push(ChatProject {
            id: format!("proj_{}", uuid::Uuid::new_v4()),
            name: folder.to_string(),
            description: None,
            color: None,
            created_at: now,
            updated_at: now,
        });
        changed = true;
    }

    project_index.projects.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.cmp(&b.name))
    });

    if changed {
        save_project_index(app, &project_index)?;
    }

    Ok(project_index.projects)
}

pub fn get_assistants(
    app: &AppHandle,
    include_archived: bool,
) -> Result<Vec<ChatAssistant>, String> {
    let index = load_assistant_index(app)?;
    let mut assistants = index.assistants;
    if !include_archived {
        assistants.retain(|assistant| !assistant.archived);
    }
    assistants.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(assistants)
}

pub fn get_assistant(app: &AppHandle, assistant_id: &str) -> Result<ChatAssistant, String> {
    validate_assistant_id(assistant_id)?;
    load_assistant_index(app)?
        .assistants
        .into_iter()
        .find(|assistant| assistant.id == assistant_id)
        .ok_or_else(|| "助手不存在".to_string())
}

pub fn create_assistant(
    app: &AppHandle,
    mut assistant: ChatAssistant,
) -> Result<ChatAssistant, String> {
    validate_assistant_id(&assistant.id)?;
    normalize_assistant(&mut assistant)?;
    let mut index = load_assistant_index(app)?;
    if index.assistants.iter().any(|item| item.id == assistant.id) {
        return Err("助手 ID 已存在".to_string());
    }
    if index
        .assistants
        .iter()
        .any(|item| !item.archived && item.name == assistant.name)
    {
        return Err("助手名称已存在".to_string());
    }
    index.assistants.insert(0, assistant.clone());
    save_assistant_index(app, &index)?;
    Ok(assistant)
}

pub fn update_assistant(
    app: &AppHandle,
    assistant: ChatAssistant,
) -> Result<ChatAssistant, String> {
    validate_assistant_id(&assistant.id)?;
    let mut next = assistant;
    normalize_assistant(&mut next)?;
    let mut index = load_assistant_index(app)?;
    let pos = index
        .assistants
        .iter()
        .position(|item| item.id == next.id)
        .ok_or_else(|| "助手不存在".to_string())?;
    if index
        .assistants
        .iter()
        .any(|item| item.id != next.id && !item.archived && item.name == next.name)
    {
        return Err("助手名称已存在".to_string());
    }
    next.built_in = index.assistants[pos].built_in;
    next.created_at = index.assistants[pos].created_at;
    index.assistants[pos] = next.clone();
    save_assistant_index(app, &index)?;
    Ok(next)
}

pub fn duplicate_assistant(app: &AppHandle, assistant_id: &str) -> Result<ChatAssistant, String> {
    let source = get_assistant(app, assistant_id)?;
    let now = chrono::Local::now().timestamp();
    let copy = ChatAssistant {
        id: format!("asst_{}", uuid::Uuid::new_v4()),
        name: unique_assistant_copy_name(app, &source.name)?,
        built_in: false,
        archived: false,
        created_at: now,
        updated_at: now,
        ..source
    };
    create_assistant(app, copy)
}

pub fn archive_assistant(app: &AppHandle, assistant_id: &str) -> Result<(), String> {
    validate_assistant_id(assistant_id)?;
    let mut index = load_assistant_index(app)?;
    let Some(pos) = index
        .assistants
        .iter()
        .position(|assistant| assistant.id == assistant_id)
    else {
        return Err("助手不存在".to_string());
    };
    index.assistants[pos].archived = true;
    index.assistants[pos].updated_at = chrono::Local::now().timestamp();
    save_assistant_index(app, &index)
}

pub fn create_project(app: &AppHandle, mut project: ChatProject) -> Result<ChatProject, String> {
    validate_project_id(&project.id)?;
    project.name = normalize_project_name(&project.name)?;
    let mut index = load_project_index(app)?;
    if index.projects.iter().any(|item| item.name == project.name) {
        return Err("项目名称已存在".to_string());
    }
    index.projects.insert(0, project.clone());
    save_project_index(app, &index)?;
    Ok(project)
}

pub fn update_project(
    app: &AppHandle,
    project_id: &str,
    name: Option<String>,
    description: Option<String>,
    color: Option<String>,
) -> Result<ChatProject, String> {
    validate_project_id(project_id)?;
    let mut project_index = load_project_index(app)?;
    let pos = project_index
        .projects
        .iter()
        .position(|project| project.id == project_id)
        .ok_or_else(|| "项目不存在".to_string())?;

    let old_name = project_index.projects[pos].name.clone();
    let new_name = match name {
        Some(name) => Some(normalize_project_name(&name)?),
        None => None,
    };
    if let Some(next_name) = new_name.as_deref() {
        if next_name != old_name
            && project_index
                .projects
                .iter()
                .any(|project| project.name == next_name)
        {
            return Err("项目名称已存在".to_string());
        }
    }

    if let Some(next_name) = new_name {
        project_index.projects[pos].name = next_name;
    }
    if description.is_some() {
        project_index.projects[pos].description = description;
    }
    if color.is_some() {
        project_index.projects[pos].color = color;
    }
    project_index.projects[pos].updated_at = chrono::Local::now().timestamp();
    let project = project_index.projects[pos].clone();
    save_project_index(app, &project_index)?;

    if project.name != old_name {
        move_project_conversations(app, &old_name, Some(&project.name))?;
    }

    Ok(project)
}

pub fn delete_project(app: &AppHandle, project_id: &str) -> Result<(), String> {
    validate_project_id(project_id)?;
    let mut project_index = load_project_index(app)?;
    let Some(pos) = project_index
        .projects
        .iter()
        .position(|project| project.id == project_id)
    else {
        return Err("项目不存在".to_string());
    };
    let project = project_index.projects.remove(pos);
    save_project_index(app, &project_index)?;
    move_project_conversations(app, &project.name, None)
}

fn normalize_project_name(name: &str) -> Result<String, String> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err("项目名称不能为空".to_string());
    }
    if normalized.chars().count() > 80 {
        return Err("项目名称不能超过 80 个字符".to_string());
    }
    Ok(normalized.to_string())
}

pub fn assistant_snapshot(
    app: &AppHandle,
    assistant_id: &str,
) -> Result<ChatAssistantSnapshot, String> {
    let assistant = get_assistant(app, assistant_id)?;
    if assistant.archived || !assistant.enabled {
        return Err("助手不可用".to_string());
    }
    Ok(ChatAssistantSnapshot::from(&assistant))
}

fn normalize_assistant(assistant: &mut ChatAssistant) -> Result<(), String> {
    assistant.name = assistant.name.trim().to_string();
    if assistant.name.is_empty() {
        return Err("助手名称不能为空".to_string());
    }
    if assistant.name.chars().count() > 64 {
        return Err("助手名称不能超过 64 个字符".to_string());
    }
    assistant.description = assistant.description.trim().to_string();
    if assistant.description.chars().count() > 240 {
        return Err("助手描述不能超过 240 个字符".to_string());
    }
    assistant.icon = assistant.icon.trim().chars().take(8).collect();
    assistant.color = assistant.color.trim().chars().take(32).collect();
    assistant.source = normalize_assistant_source(&assistant.source, assistant.built_in);
    assistant.author = assistant.author.trim().chars().take(80).collect();
    if assistant.author.is_empty() && assistant.built_in {
        assistant.author = "Kivio".to_string();
    }
    assistant.version = normalize_assistant_version(&assistant.version);
    assistant.category = assistant.category.trim().chars().take(40).collect();
    assistant.tags = normalize_string_list(&assistant.tags, 8, 24);
    assistant.provider_id = assistant.provider_id.trim().to_string();
    assistant.model = assistant.model.trim().to_string();
    assistant.skill_id = assistant
        .skill_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    assistant.tool_preset = normalize_tool_preset(&assistant.tool_preset);
    assistant.conversation_starters = assistant
        .conversation_starters
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .take(6)
        .map(str::to_string)
        .collect();
    assistant.greeting = assistant.greeting.trim().to_string();
    assistant.quick_commands = assistant
        .quick_commands
        .clone()
        .into_iter()
        .enumerate()
        .filter_map(|(idx, command)| normalize_quick_command(command, idx))
        .take(12)
        .collect();
    assistant.data_connectors = assistant
        .data_connectors
        .clone()
        .into_iter()
        .enumerate()
        .filter_map(|(idx, connector)| normalize_data_connector(connector, idx))
        .take(12)
        .collect();
    assistant.knowledge_skills = assistant
        .knowledge_skills
        .clone()
        .into_iter()
        .enumerate()
        .filter_map(|(idx, skill)| normalize_knowledge_skill(skill, idx))
        .take(12)
        .collect();
    Ok(())
}

fn normalize_assistant_source(source: &str, built_in: bool) -> String {
    match source.trim() {
        "builtin" | "user" | "imported" => source.trim().to_string(),
        _ if built_in => "builtin".to_string(),
        _ => "user".to_string(),
    }
}

fn normalize_assistant_version(version: &str) -> String {
    let trimmed = version.trim().trim_start_matches('v');
    if trimmed.is_empty() {
        "1.0.0".to_string()
    } else {
        trimmed.chars().take(24).collect()
    }
}

fn normalize_string_list(values: &[String], limit: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let item: String = value.trim().chars().take(max_chars).collect();
        if item.is_empty() || out.iter().any(|existing| existing == &item) {
            continue;
        }
        out.push(item);
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn normalize_quick_command(
    mut command: AssistantQuickCommand,
    idx: usize,
) -> Option<AssistantQuickCommand> {
    command.name = command.name.trim().chars().take(40).collect();
    command.slash = normalize_slash_command(&command.slash, &command.name);
    if command.name.is_empty() || command.slash.is_empty() {
        return None;
    }
    command.id = normalize_local_id(&command.id, "cmd", idx);
    command.description = command.description.trim().chars().take(160).collect();
    command.placeholder = command.placeholder.trim().chars().take(160).collect();
    command.prompt = command.prompt.trim().chars().take(4000).collect();
    command.starter_text = command.starter_text.trim().chars().take(400).collect();
    Some(command)
}

fn normalize_slash_command(value: &str, fallback_name: &str) -> String {
    let source = if value.trim().is_empty() {
        fallback_name.trim()
    } else {
        value.trim()
    };
    if source.is_empty() {
        return String::new();
    }
    let compact: String = source
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .take(32)
        .collect();
    if compact.starts_with('/') {
        compact
    } else {
        format!("/{compact}")
    }
}

fn normalize_data_connector(
    mut connector: AssistantDataConnector,
    idx: usize,
) -> Option<AssistantDataConnector> {
    connector.name = connector.name.trim().chars().take(60).collect();
    if connector.name.is_empty() {
        return None;
    }
    connector.id = normalize_local_id(&connector.id, "conn", idx);
    connector.kind = match connector.kind.trim() {
        "builtin_tool" | "mcp" | "skill_tool" | "memory" | "file" | "web" => {
            connector.kind.trim().to_string()
        }
        _ => "builtin_tool".to_string(),
    };
    connector.description = connector.description.trim().chars().take(180).collect();
    connector.tool_ids = normalize_string_list(&connector.tool_ids, 12, 80);
    connector.server_id = connector
        .server_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(80).collect());
    Some(connector)
}

fn normalize_knowledge_skill(
    mut skill: AssistantKnowledgeSkill,
    idx: usize,
) -> Option<AssistantKnowledgeSkill> {
    skill.name = skill.name.trim().chars().take(60).collect();
    if skill.name.is_empty() {
        return None;
    }
    skill.id = normalize_local_id(&skill.id, "ks", idx);
    skill.description = skill.description.trim().chars().take(360).collect();
    skill.trigger_phrases = normalize_string_list(&skill.trigger_phrases, 16, 40);
    skill.skill_id = skill
        .skill_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    skill.prompt = skill.prompt.trim().chars().take(5000).collect();
    skill.recommended_tools = normalize_string_list(&skill.recommended_tools, 12, 80);
    skill.requires_connectors = normalize_string_list(&skill.requires_connectors, 12, 80);
    Some(skill)
}

fn normalize_local_id(id: &str, prefix: &str, idx: usize) -> String {
    let trimmed = id.trim();
    let compact: String = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .take(80)
        .collect();
    if !compact.is_empty() {
        compact
    } else if trimmed.is_empty() {
        format!("{prefix}_{idx}")
    } else {
        format!("{prefix}_{:016x}", stable_label_hash(trimmed))
    }
}

fn stable_label_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn normalize_tool_preset(value: &str) -> String {
    match value.trim() {
        "none" | "skills" | "all" => value.trim().to_string(),
        _ => "inherit".to_string(),
    }
}

fn unique_assistant_copy_name(app: &AppHandle, base_name: &str) -> Result<String, String> {
    let index = load_assistant_index(app)?;
    let base = format!("{base_name} 副本");
    if !index
        .assistants
        .iter()
        .any(|assistant| !assistant.archived && assistant.name == base)
    {
        return Ok(base);
    }
    for i in 2..100 {
        let candidate = format!("{base} {i}");
        if !index
            .assistants
            .iter()
            .any(|assistant| !assistant.archived && assistant.name == candidate)
        {
            return Ok(candidate);
        }
    }
    Ok(format!("{base} {}", chrono::Local::now().timestamp()))
}

fn ensure_default_assistants(index: &mut ChatAssistantIndex) -> bool {
    let mut changed = false;
    for assistant in default_assistants() {
        if let Some(existing) = index
            .assistants
            .iter_mut()
            .find(|item| item.id == assistant.id)
        {
            changed |= hydrate_builtin_assistant(existing, &assistant);
        } else {
            index.assistants.push(assistant);
            changed = true;
        }
    }
    if changed {
        index.assistants.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.name.cmp(&b.name))
        });
    }
    changed
}

fn hydrate_builtin_assistant(existing: &mut ChatAssistant, default: &ChatAssistant) -> bool {
    if !existing.built_in && existing.source != "builtin" {
        return false;
    }
    let mut changed = false;
    if existing.id == "asst_builtin_general"
        && existing.system_prompt.trim() == LEGACY_GENERAL_ASSISTANT_SYSTEM_PROMPT
    {
        existing.system_prompt.clear();
        changed = true;
    }
    if existing.source.trim().is_empty() {
        existing.source = default.source.clone();
        changed = true;
    }
    if existing.author.trim().is_empty() {
        existing.author = default.author.clone();
        changed = true;
    }
    if existing.version.trim().is_empty() {
        existing.version = default.version.clone();
        changed = true;
    }
    if existing.category.trim().is_empty() {
        existing.category = default.category.clone();
        changed = true;
    }
    if existing.tags.is_empty() {
        existing.tags = default.tags.clone();
        changed = true;
    }
    if existing.icon.trim().is_empty() {
        existing.icon = default.icon.clone();
        changed = true;
    }
    if existing.color.trim().is_empty() {
        existing.color = default.color.clone();
        changed = true;
    }
    if existing.quick_commands.is_empty() {
        existing.quick_commands = default.quick_commands.clone();
        changed = true;
    }
    if existing.data_connectors.is_empty() {
        existing.data_connectors = default.data_connectors.clone();
        changed = true;
    }
    if existing.knowledge_skills.is_empty() {
        existing.knowledge_skills = default.knowledge_skills.clone();
        changed = true;
    }
    changed
}

fn default_assistants() -> Vec<ChatAssistant> {
    let now = 1_700_000_000;
    vec![
        ChatAssistant {
            id: "asst_builtin_general".to_string(),
            name: "通用助手".to_string(),
            description: "适合日常问答、梳理想法和处理轻量任务。".to_string(),
            icon: "sparkles".to_string(),
            color: "#6A8FBD".to_string(),
            source: "builtin".to_string(),
            author: "Kivio".to_string(),
            version: "1.0.0".to_string(),
            category: "general".to_string(),
            tags: vec!["通用".to_string(), "效率".to_string()],
            system_prompt: String::new(),
            provider_id: String::new(),
            model: String::new(),
            skill_id: None,
            tool_preset: "inherit".to_string(),
            conversation_starters: vec![
                "帮我整理一下这个想法".to_string(),
                "把这段内容改得更清楚".to_string(),
                "给我一个可执行的下一步计划".to_string(),
            ],
            greeting: "我可以帮你整理、分析、写作和处理日常 AI 任务。".to_string(),
            quick_commands: vec![
                quick_command("整理想法", "/整理", "把零散想法整理成结构化要点", "请把用户输入整理成背景、关键点、风险和下一步。"),
                quick_command("改清楚", "/改清楚", "让表达更直接、更易读", "在保留原意的前提下改写用户内容，使其更清晰、自然、紧凑。"),
                quick_command("下一步", "/下一步", "给出可执行计划", "把用户目标拆成具体下一步，优先给出今天就能执行的动作。"),
            ],
            data_connectors: vec![
                data_connector("memory", "记忆", "memory", "读取和维护用户长期偏好与流程。", vec!["memory_read", "memory_modify"]),
            ],
            knowledge_skills: vec![
                knowledge_skill("日常任务拆解", "把模糊问题拆成目标、约束、方案和行动。", vec!["整理", "计划", "下一步"], None, ""),
            ],
            enabled: true,
            installed: true,
            archived: false,
            built_in: true,
            created_at: now,
            updated_at: now + 5,
        },
        ChatAssistant {
            id: "asst_builtin_translate_polish".to_string(),
            name: "翻译润色助手".to_string(),
            description: "面向翻译、改写、语气调整和双语表达。".to_string(),
            icon: "languages".to_string(),
            color: "#C56646".to_string(),
            source: "builtin".to_string(),
            author: "Kivio".to_string(),
            version: "1.0.0".to_string(),
            category: "language".to_string(),
            tags: vec!["翻译".to_string(), "润色".to_string()],
            system_prompt: "你是翻译与润色助手。优先保留原意，输出自然、准确、适合目标语境的表达；必要时给出简短改动说明。".to_string(),
            provider_id: String::new(),
            model: String::new(),
            skill_id: None,
            tool_preset: "inherit".to_string(),
            conversation_starters: vec![
                "把这段中文翻译成自然英文".to_string(),
                "帮我润色这段邮件".to_string(),
                "给我三个不同语气的版本".to_string(),
            ],
            greeting: "贴文本给我，我会帮你翻译、润色或改成指定语气。".to_string(),
            quick_commands: vec![
                quick_command("翻译", "/翻译", "翻译为指定语言", "把用户内容翻译成目标语言；未指定目标语言时，中文默认译成英文，其他语言默认译成中文。"),
                quick_command("润色", "/润色", "改善措辞和流畅度", "保留原意，提升表达自然度、专业度和可读性。"),
                quick_command("语气调整", "/语气", "按指定语气改写", "按用户指定的正式、友好、简洁、礼貌等语气改写内容。"),
            ],
            data_connectors: Vec::new(),
            knowledge_skills: vec![
                knowledge_skill("双语表达", "保持含义准确，同时让目标语言读起来自然。", vec!["翻译", "双语", "英文"], None, ""),
                knowledge_skill("表达润色", "针对邮件、产品文案、说明文字做语气和清晰度优化。", vec!["润色", "改写", "语气"], None, ""),
            ],
            enabled: true,
            installed: true,
            archived: false,
            built_in: true,
            created_at: now,
            updated_at: now + 4,
        },
        ChatAssistant {
            id: "asst_builtin_screenshot_analyst".to_string(),
            name: "截图分析助手".to_string(),
            description: "适合分析截图、界面、报错和视觉信息。".to_string(),
            icon: "scan".to_string(),
            color: "#8A6FBD".to_string(),
            source: "builtin".to_string(),
            author: "Kivio".to_string(),
            version: "1.0.0".to_string(),
            category: "vision".to_string(),
            tags: vec!["截图".to_string(), "视觉".to_string()],
            system_prompt: "你是截图分析助手。看到图片时先描述关键信息，再回答用户问题；如果是界面或报错，优先指出可能原因和下一步操作。".to_string(),
            provider_id: String::new(),
            model: String::new(),
            skill_id: None,
            tool_preset: "inherit".to_string(),
            conversation_starters: vec![
                "这张截图里发生了什么？".to_string(),
                "帮我分析这个报错".to_string(),
                "这个界面可以怎么优化？".to_string(),
            ],
            greeting: "发截图或图片给我，我会帮你识别重点并分析问题。".to_string(),
            quick_commands: vec![
                quick_command("分析截图", "/截图分析", "解释截图里的关键信息", "结合截图回答用户问题，先识别画面中的关键对象、文本和状态。"),
                quick_command("报错排查", "/报错", "定位错误原因和下一步", "读取截图或文本中的报错信息，给出可能原因、验证方法和修复步骤。"),
                quick_command("界面建议", "/界面建议", "分析 UI 可用性", "从信息层级、交互效率、视觉一致性和可读性角度分析界面。"),
            ],
            data_connectors: vec![
                data_connector("vision", "图片附件", "file", "读取当前对话中的截图和图片附件。", Vec::new()),
            ],
            knowledge_skills: vec![
                knowledge_skill("截图信息提取", "从截图中提取文字、状态、按钮、报错和上下文线索。", vec!["截图", "界面", "报错"], None, ""),
            ],
            enabled: true,
            installed: true,
            archived: false,
            built_in: true,
            created_at: now,
            updated_at: now + 3,
        },
        ChatAssistant {
            id: "asst_builtin_code_data".to_string(),
            name: "编程/数据助手".to_string(),
            description: "适合代码解释、调试、脚本和数据分析。".to_string(),
            icon: "code".to_string(),
            color: "#4F9D7A".to_string(),
            source: "builtin".to_string(),
            author: "Kivio".to_string(),
            version: "1.0.0".to_string(),
            category: "technical".to_string(),
            tags: vec!["代码".to_string(), "数据".to_string()],
            system_prompt: "你是编程和数据助手。回答要具体，优先给出可运行的步骤、代码或排查路径；涉及不确定信息时说明验证方式。".to_string(),
            provider_id: String::new(),
            model: String::new(),
            skill_id: None,
            tool_preset: "all".to_string(),
            conversation_starters: vec![
                "解释这段代码".to_string(),
                "帮我定位这个 bug".to_string(),
                "用数据分析这个问题".to_string(),
            ],
            greeting: "把代码、错误信息或数据问题发给我，我会帮你拆解和验证。".to_string(),
            quick_commands: vec![
                quick_command("解释代码", "/解释代码", "解释代码行为和结构", "解释用户提供代码的目的、关键路径、输入输出和潜在风险。"),
                quick_command("调试", "/调试", "定位 bug 或报错", "根据代码、日志或报错，提出排查路径、可能原因和修复建议。"),
                quick_command("数据分析", "/数据分析", "分析数据或生成图表", "优先使用可用的数据/代码工具验证结论，并给出可复现步骤。"),
            ],
            data_connectors: vec![
                data_connector("python", "Python 沙盒", "builtin_tool", "运行 Python 做数据计算、图表和文件分析。", vec!["run_python"]),
                data_connector("filesystem", "文件读取", "builtin_tool", "读取用户提供的本地文本文件。", vec!["read_file"]),
            ],
            knowledge_skills: vec![
                knowledge_skill("代码调试", "把问题拆成复现、定位、验证、修复四步。", vec!["bug", "报错", "调试"], None, ""),
                knowledge_skill("数据分析", "用数据处理和统计方法回答问题，并说明假设。", vec!["数据", "统计", "图表"], Some("xlsx"), ""),
            ],
            enabled: true,
            installed: true,
            archived: false,
            built_in: true,
            created_at: now,
            updated_at: now + 2,
        },
        ChatAssistant {
            id: "asst_builtin_writing".to_string(),
            name: "写作助手".to_string(),
            description: "适合文章、文案、提纲、总结和表达优化。".to_string(),
            icon: "pen".to_string(),
            color: "#BD8A3E".to_string(),
            source: "builtin".to_string(),
            author: "Kivio".to_string(),
            version: "1.0.0".to_string(),
            category: "writing".to_string(),
            tags: vec!["写作".to_string(), "总结".to_string()],
            system_prompt: "你是写作助手。先理解目标读者和用途，输出结构清晰、语言自然的内容；需要时给出多个可选版本。".to_string(),
            provider_id: String::new(),
            model: String::new(),
            skill_id: None,
            tool_preset: "inherit".to_string(),
            conversation_starters: vec![
                "帮我写一个提纲".to_string(),
                "把这段话改得更有说服力".to_string(),
                "总结这段内容".to_string(),
            ],
            greeting: "告诉我写作目标和受众，我会帮你起草、改写或总结。".to_string(),
            quick_commands: vec![
                quick_command("写提纲", "/提纲", "生成文章或方案提纲", "根据用户主题生成层次清楚、可继续扩展的提纲。"),
                quick_command("写文案", "/文案", "生成产品或传播文案", "围绕目标受众、场景和行动目标生成简洁有力的文案。"),
                quick_command("总结", "/总结", "提炼重点", "把用户内容总结成重点、结论和可行动事项。"),
            ],
            data_connectors: Vec::new(),
            knowledge_skills: vec![
                knowledge_skill("结构化写作", "先确定读者、目的、结构，再生成正文。", vec!["提纲", "文章", "文案"], None, ""),
                knowledge_skill("总结提炼", "压缩内容时保留结论、证据和行动项。", vec!["总结", "提炼", "摘要"], None, ""),
            ],
            enabled: true,
            installed: true,
            archived: false,
            built_in: true,
            created_at: now,
            updated_at: now + 1,
        },
    ]
}

fn quick_command(
    name: &str,
    slash: &str,
    description: &str,
    prompt: &str,
) -> AssistantQuickCommand {
    AssistantQuickCommand {
        id: normalize_local_id(slash, "cmd", 0),
        name: name.to_string(),
        slash: slash.to_string(),
        description: description.to_string(),
        placeholder: String::new(),
        prompt: prompt.to_string(),
        starter_text: String::new(),
        requires_suite_enabled: true,
        enabled: true,
    }
}

fn data_connector(
    id: &str,
    name: &str,
    kind: &str,
    description: &str,
    tool_ids: Vec<&str>,
) -> AssistantDataConnector {
    AssistantDataConnector {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        description: description.to_string(),
        tool_ids: tool_ids.into_iter().map(str::to_string).collect(),
        server_id: None,
        required: false,
        enabled: true,
        configured: true,
    }
}

fn knowledge_skill(
    name: &str,
    description: &str,
    trigger_phrases: Vec<&str>,
    skill_id: Option<&str>,
    prompt: &str,
) -> AssistantKnowledgeSkill {
    AssistantKnowledgeSkill {
        id: normalize_local_id(name, "ks", 0),
        name: name.to_string(),
        description: description.to_string(),
        trigger_phrases: trigger_phrases.into_iter().map(str::to_string).collect(),
        skill_id: skill_id.map(str::to_string),
        prompt: prompt.to_string(),
        recommended_tools: Vec::new(),
        requires_connectors: Vec::new(),
        enabled: true,
    }
}

fn move_project_conversations(
    app: &AppHandle,
    old_name: &str,
    next_name: Option<&str>,
) -> Result<(), String> {
    let mut index = load_index_or_scan(app)?;
    let mut changed = false;
    for item in &mut index.conversations {
        if item.folder.as_deref() != Some(old_name) {
            continue;
        }
        let mut conversation = load_conversation(app, &item.id)?;
        conversation.folder = next_name.map(str::to_string);
        conversation.updated_at = chrono::Local::now().timestamp();
        save_conversation_without_index(app, &conversation)?;
        *item = ConversationListItem::from(&conversation);
        changed = true;
    }
    if changed {
        save_index(app, &index)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_local_id_hashes_non_ascii_labels() {
        let first = normalize_local_id("/整理", "cmd", 0);
        let second = normalize_local_id("/总结", "cmd", 0);

        assert!(first.starts_with("cmd_"));
        assert!(second.starts_with("cmd_"));
        assert_ne!(first, "cmd_0");
        assert_ne!(first, second);
    }

    #[test]
    fn hydrate_builtin_assistant_fills_missing_suite_fields_without_overwriting_existing() {
        let defaults = default_assistants();
        let default = defaults
            .iter()
            .find(|assistant| assistant.id == "asst_builtin_code_data")
            .expect("default code/data assistant exists");
        let mut existing = default.clone();
        existing.quick_commands = Vec::new();
        existing.data_connectors = Vec::new();
        existing.knowledge_skills = Vec::new();
        existing.author.clear();
        existing.color = "#123456".to_string();

        let changed = hydrate_builtin_assistant(&mut existing, default);

        assert!(changed);
        assert_eq!(existing.quick_commands.len(), default.quick_commands.len());
        assert_eq!(
            existing.data_connectors.len(),
            default.data_connectors.len()
        );
        assert_eq!(
            existing.knowledge_skills.len(),
            default.knowledge_skills.len()
        );
        assert_eq!(existing.author, "Kivio");
        assert_eq!(existing.color, "#123456");
    }
}
