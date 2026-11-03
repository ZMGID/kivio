use super::*;

pub fn load_assistant_index(app: &AppHandle) -> Result<ChatAssistantIndex, String> {
    let path = assistants_file_path(app)?;
    if !path.exists() {
        // 内置专家已下线，启动为空，由用户自建。
        return Ok(ChatAssistantIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read assistants file: {e}"))?;
    let mut index: ChatAssistantIndex =
        serde_json::from_str(&content).map_err(|e| format!("parse assistants file: {e}"))?;
    for assistant in &mut index.assistants {
        retire_cua_mcp_server_ids(&mut assistant.mcp_server_ids);
    }
    Ok(index)
}

pub fn save_assistant_index(app: &AppHandle, index: &ChatAssistantIndex) -> Result<(), String> {
    let path = assistants_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize assistants: {e}"))?;
    atomic_write(&path, &content, "assistants")
}

/// 删除随应用附带的内置专家（`built_in == true`）。
/// 用户自建专家保留；集里指向已不存在助手的默认助手一并清空，避免留下悬空引用。
/// 幂等，启动时运行；旧备份恢复后重新出现的条目也会再次被清理。
pub fn remove_builtin_assistants(app: &AppHandle) -> Result<usize, String> {
    let _catalog = catalog_mutation_lock();
    let mut index = load_assistant_index(app)?;
    let removed = remove_builtin_assistants_from(&mut index.assistants);
    if removed > 0 {
        save_assistant_index(app, &index)?;
    }
    clear_missing_default_assistants(app, &index.assistants)?;
    Ok(removed)
}

fn remove_builtin_assistants_from(assistants: &mut Vec<ChatAssistant>) -> usize {
    let before = assistants.len();
    assistants.retain(|assistant| !assistant.built_in);
    before - assistants.len()
}

fn clear_missing_default_assistants(
    app: &AppHandle,
    assistants: &[ChatAssistant],
) -> Result<(), String> {
    let mut sets = super::sets::load_set_index(app)?;
    if clear_missing_default_assistant_ids(&mut sets.sets, assistants) {
        super::sets::save_set_index(app, &sets)?;
    }
    Ok(())
}

fn clear_missing_default_assistant_ids(sets: &mut [ChatSet], assistants: &[ChatAssistant]) -> bool {
    let known: std::collections::HashSet<&str> = assistants
        .iter()
        .map(|assistant| assistant.id.as_str())
        .collect();
    let mut changed = false;
    for set in sets {
        if let Some(id) = set.default_assistant_id.as_deref() {
            if !known.contains(id) {
                set.default_assistant_id = None;
                changed = true;
            }
        }
    }
    changed
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
    let _catalog = catalog_mutation_lock();
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
    let _catalog = catalog_mutation_lock();
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
    let _catalog = catalog_mutation_lock();
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

pub fn assistant_snapshot(
    app: &AppHandle,
    assistant_id: &str,
) -> Result<ChatAssistantSnapshot, String> {
    let assistant = get_assistant(app, assistant_id)?;
    if !assistant_is_available(&assistant) {
        return Err("助手不可用".to_string());
    }
    Ok(ChatAssistantSnapshot::from(&assistant))
}

/// `enabled` 是旧版助手中心留下的兼容字段。新版以「常用 / installed」控制选择器展示，
/// 不再提供启停入口；若继续把旧的 `enabled=false` 当作运行时禁用，用户会看到助手却无法使用，
/// 也没有任何地方能重新启用。因此运行时可用性只由归档状态决定。
pub(super) fn assistant_is_available(assistant: &ChatAssistant) -> bool {
    !assistant.archived
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
    assistant.system_prompt = assistant.system_prompt.trim().to_string();
    assistant.provider_id = assistant.provider_id.trim().to_string();
    assistant.model = assistant.model.trim().to_string();
    assistant.mcp_server_ids = normalize_string_list(&assistant.mcp_server_ids, 64, 200);
    retire_cua_mcp_server_ids(&mut assistant.mcp_server_ids);
    assistant.skill_ids = normalize_string_list(&assistant.skill_ids, 64, 200);
    Ok(())
}

pub(super) fn retire_cua_mcp_server_ids(ids: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| {
        !crate::computer_control::is_cua_mcp_server_id(id.trim()) && seen.insert(id.clone())
    });
}

fn normalize_assistant_source(source: &str, built_in: bool) -> String {
    match source.trim() {
        "builtin" | "user" | "imported" => source.trim().to_string(),
        _ if built_in => "builtin".to_string(),
        _ => "user".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(id: &str, built_in: bool) -> ChatAssistant {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": id,
            "built_in": built_in,
            "created_at": 1,
            "updated_at": 1,
        }))
        .unwrap()
    }

    fn set(id: &str, default_assistant_id: Option<&str>) -> ChatSet {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": id,
            "default_assistant_id": default_assistant_id,
            "created_at": 1,
            "updated_at": 2,
        }))
        .unwrap()
    }

    #[test]
    fn removes_only_built_in_assistants() {
        let mut assistants = vec![
            assistant("asst_builtin_writer", true),
            assistant("asst_user_custom", false),
        ];

        assert_eq!(remove_builtin_assistants_from(&mut assistants), 1);
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0].id, "asst_user_custom");
    }

    #[test]
    fn clears_set_defaults_that_no_longer_resolve() {
        let assistants = vec![assistant("asst_user_custom", false)];
        let mut sets = vec![
            set("set_keep", Some("asst_user_custom")),
            set("set_drop", Some("asst_builtin_writer")),
            set("set_none", None),
        ];

        assert!(clear_missing_default_assistant_ids(&mut sets, &assistants));
        assert_eq!(
            sets[0].default_assistant_id.as_deref(),
            Some("asst_user_custom")
        );
        assert_eq!(sets[1].default_assistant_id, None);
        assert_eq!(sets[2].default_assistant_id, None);
        assert!(!clear_missing_default_assistant_ids(&mut sets, &assistants));
    }
}
