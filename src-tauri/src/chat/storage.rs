use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::{
    ChatAssistant, ChatAssistantIndex, ChatAssistantSnapshot, ChatProject, ChatProjectIndex,
    Conversation, ConversationIndex, ConversationListItem,
};

const WRITE_RETRY_ATTEMPTS: usize = 3;

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
    ensure_default_assistants(&mut index);
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
    Ok(())
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

fn ensure_default_assistants(index: &mut ChatAssistantIndex) {
    let mut changed = false;
    for assistant in default_assistants() {
        if index.assistants.iter().any(|item| item.id == assistant.id) {
            continue;
        }
        index.assistants.push(assistant);
        changed = true;
    }
    if changed {
        index.assistants.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.name.cmp(&b.name))
        });
    }
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
            system_prompt: "你是 Kivio 的通用助手。回答要清晰、直接，并在信息不足时主动说明假设。".to_string(),
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
            enabled: true,
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
            enabled: true,
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
            enabled: true,
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
            enabled: true,
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
            enabled: true,
            archived: false,
            built_in: true,
            created_at: now,
            updated_at: now + 1,
        },
    ]
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
