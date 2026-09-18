use super::*;

// ===== Chat 集(Set) 存储：照搬 project 模式，去掉 root_path/folder 迁移，加 system_prompt/默认助手 =====

pub(super) fn validate_set_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("set_")
        && id.len() > "set_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid set id: {id}"))
    }
}

pub(super) fn normalize_set_name(name: &str) -> Result<String, String> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err("集名称不能为空".to_string());
    }
    Ok(normalized.chars().take(80).collect())
}

pub fn sets_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("sets.json"))
}

pub fn load_set_index(app: &AppHandle) -> Result<ChatSetIndex, String> {
    let path = sets_file_path(app)?;
    if !path.exists() {
        return Ok(ChatSetIndex::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read sets file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse sets file: {e}"))
}

pub fn save_set_index(app: &AppHandle, index: &ChatSetIndex) -> Result<(), String> {
    let path = sets_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize sets: {e}"))?;
    atomic_write(&path, &content, "sets")
}

pub fn get_sets(app: &AppHandle) -> Result<Vec<ChatSet>, String> {
    Ok(load_set_index(app)?.sets)
}

pub fn find_set_by_id(app: &AppHandle, set_id: &str) -> Result<ChatSet, String> {
    validate_set_id(set_id)?;
    load_set_index(app)?
        .sets
        .into_iter()
        .find(|set| set.id == set_id)
        .ok_or_else(|| "集不存在".to_string())
}

/// Live set-level system prompt for a conversation. Looked up by `set_id` on
/// every call (not frozen), so editing the set applies to in-set conversations
/// immediately. Missing set / empty prompt → `None`.
pub fn live_set_system_prompt(app: &AppHandle, conversation: &Conversation) -> Option<String> {
    conversation
        .set_id
        .as_deref()
        .and_then(|id| find_set_by_id(app, id).ok())
        .map(|set| set.system_prompt)
        .filter(|prompt| !prompt.trim().is_empty())
}

pub fn create_set(app: &AppHandle, mut set: ChatSet) -> Result<ChatSet, String> {
    validate_set_id(&set.id)?;
    set.name = normalize_set_name(&set.name)?;
    let mut index = load_set_index(app)?;
    if index.sets.iter().any(|item| item.name == set.name) {
        return Err("集名称已存在".to_string());
    }
    index.sets.insert(0, set.clone());
    save_set_index(app, &index)?;
    Ok(set)
}

#[allow(clippy::too_many_arguments)]
pub fn update_set(
    app: &AppHandle,
    set_id: &str,
    name: Option<String>,
    system_prompt: Option<String>,
    system_prompt_set: bool,
    default_assistant_id: Option<String>,
    default_assistant_id_set: bool,
    color: Option<String>,
    color_set: bool,
) -> Result<ChatSet, String> {
    validate_set_id(set_id)?;
    let mut index = load_set_index(app)?;
    let pos = index
        .sets
        .iter()
        .position(|set| set.id == set_id)
        .ok_or_else(|| "集不存在".to_string())?;

    let old_name = index.sets[pos].name.clone();
    if let Some(name) = name {
        let next_name = normalize_set_name(&name)?;
        if next_name != old_name && index.sets.iter().any(|set| set.name == next_name) {
            return Err("集名称已存在".to_string());
        }
        index.sets[pos].name = next_name;
    }
    if system_prompt_set {
        index.sets[pos].system_prompt = system_prompt.unwrap_or_default();
    }
    if default_assistant_id_set {
        index.sets[pos].default_assistant_id = default_assistant_id.and_then(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    }
    if color_set {
        index.sets[pos].color = color;
    }
    index.sets[pos].updated_at = chrono::Local::now().timestamp();
    let set = index.sets[pos].clone();
    save_set_index(app, &index)?;
    Ok(set)
}

pub async fn delete_set(app: &AppHandle, set_id: &str) -> Result<(), String> {
    validate_set_id(set_id)?;
    let mut index = load_set_index(app)?;
    let Some(pos) = index.sets.iter().position(|set| set.id == set_id) else {
        return Err("集不存在".to_string());
    };
    index.sets.remove(pos);
    save_set_index(app, &index)?;
    clear_set_from_conversations(app, set_id).await
}

/// 删除集后，把名下对话的 set_id 清空（对话回到散对话、不丢）。仿 move_project_conversations。
async fn clear_set_from_conversations(app: &AppHandle, set_id: &str) -> Result<(), String> {
    crate::chat::repository::repository(app)
        .bulk_mutate(app, |conversation| {
            if conversation.set_id.as_deref() != Some(set_id) {
                return Ok(false);
            }
            conversation.set_id = None;
            Ok(true)
        })
        .await
        .map(|_| ())
        .map_err(crate::chat::repository::repository_error)
}
