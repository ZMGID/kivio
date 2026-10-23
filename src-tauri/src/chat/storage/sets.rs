use super::*;

// ===== Chat 集(Set) 存储：照搬 project 模式，去掉 root_path/folder 迁移，加 system_prompt/默认助手 =====

pub(super) use super::super::group_catalog::validate_set_id;

pub(super) use super::super::group_catalog::normalize_set_name;

pub fn sets_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("sets.json"))
}

pub fn load_set_index(app: &AppHandle) -> Result<ChatSetIndex, String> {
    super::super::group_catalog::load_set_index(&conversations_dir(app)?)
}

pub fn save_set_index(app: &AppHandle, index: &ChatSetIndex) -> Result<(), String> {
    super::super::group_catalog::save_set_index(&conversations_dir(app)?, index)
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

pub fn create_set(app: &AppHandle, set: ChatSet) -> Result<ChatSet, String> {
    let _catalog = catalog_mutation_lock();
    super::super::group_catalog::create_set(&conversations_dir(app)?, set)
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
    let _catalog = catalog_mutation_lock();
    super::super::group_catalog::update_set(
        &conversations_dir(app)?,
        set_id,
        name,
        system_prompt,
        system_prompt_set,
        default_assistant_id,
        default_assistant_id_set,
        color,
        color_set,
    )
}

pub async fn delete_set(app: &AppHandle, set_id: &str) -> Result<(), String> {
    {
        let _catalog = catalog_mutation_lock();
        validate_set_id(set_id)?;
        let mut index = load_set_index(app)?;
        let Some(pos) = index.sets.iter().position(|set| set.id == set_id) else {
            return Err("集不存在".to_string());
        };
        index.sets.remove(pos);
        save_set_index(app, &index)?;
    }
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
