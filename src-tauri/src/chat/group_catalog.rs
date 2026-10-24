//! Existing project/set catalog rules, independent of the window framework.
//! The caller supplies the conversations directory. Conversation-wide changes
//! (rename/delete membership) remain in the existing repository owner.
use super::conversation_data::ConversationPin;
use super::file_io::atomic_write;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

pub type ConversationPins = HashMap<String, Vec<ConversationPin>>;

pub fn load_conversation_pins(dir: &Path) -> Result<ConversationPins, String> {
    match fs::read_to_string(dir.join("conversation-pins.json")) {
        Ok(content) => Ok(serde_json::from_str(&content).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(format!("read conversation pins: {error}")),
    }
}

/// The caller serializes writes. Merge into fresh disk state so another group's pins survive.
pub fn set_conversation_pins(
    dir: &Path,
    group_id: &str,
    pins: Vec<ConversationPin>,
) -> Result<(), String> {
    let mut all = load_conversation_pins(dir)?;
    if pins.is_empty() {
        all.remove(group_id);
    } else {
        all.insert(group_id.to_string(), pins);
    }
    save_conversation_pins(dir, &all)
}

pub(crate) fn save_conversation_pins(dir: &Path, pins: &ConversationPins) -> Result<(), String> {
    let content = serde_json::to_string_pretty(pins)
        .map_err(|error| format!("serialize conversation pins: {error}"))?;
    atomic_write(
        &dir.join("conversation-pins.json"),
        &content,
        "conversation pins",
    )
}

/// ADR 0004: reserve pinned rows, then fill vacancies in the original time order.
/// Duplicates, missing ids and collisions never discard conversations.
pub fn apply_conversation_pins<T>(
    time_ordered: Vec<T>,
    pins: &[ConversationPin],
    id_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    let count = time_ordered.len();
    if count == 0 || pins.is_empty() {
        return time_ordered;
    }
    let by_id: HashMap<_, _> = time_ordered
        .iter()
        .enumerate()
        .map(|(index, item)| (id_of(item), index))
        .collect();
    let mut ordered_pins: Vec<_> = pins.iter().collect();
    ordered_pins.sort_by_key(|pin| pin.row);
    let mut slots = vec![None; count];
    let mut pinned = HashSet::new();
    for pin in ordered_pins {
        let Some(&index) = by_id.get(pin.id.as_str()) else {
            continue;
        };
        if pinned.contains(&index) {
            continue;
        }
        let start = (pin.row as usize).min(count - 1);
        if let Some(row) = (start..count).find(|row| slots[*row].is_none()) {
            slots[row] = Some(index);
            pinned.insert(index);
        }
    }
    let mut remaining = (0..count).filter(|index| !pinned.contains(index));
    for slot in &mut slots {
        if slot.is_none() {
            *slot = remaining.next();
        }
    }
    let mut items: Vec<_> = time_ordered.into_iter().map(Some).collect();
    slots
        .into_iter()
        .flatten()
        .filter_map(|index| items[index].take())
        .collect()
}

/// Chat 项目。`folder` 保留用于旧对话兼容，真实项目归属使用 `project_id`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatProject {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub root_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 项目索引文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatProjectIndex {
    pub projects: Vec<ChatProject>,
}

/// Chat 集(Set)：助手之上的人设分组。不带工作目录（区别于项目）；持有自己的系统提示词
/// 和默认助手。集名下的对话通过 `Conversation.set_id` 关联（与 project_id 互斥）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSet {
    pub id: String,
    pub name: String,
    /// 集级系统提示词，运行时实时注入集内对话（不冻结）。
    #[serde(default)]
    pub system_prompt: String,
    /// 集的默认助手 id；在集下新建对话且未显式指定助手时使用。
    #[serde(default)]
    pub default_assistant_id: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 集索引文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatSetIndex {
    pub sets: Vec<ChatSet>,
}

pub fn load_project_index(dir: &Path) -> Result<ChatProjectIndex, String> {
    let path = dir.join("projects.json");
    if !path.exists() {
        return Ok(ChatProjectIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read projects file: {e}"))?;
    let mut index: ChatProjectIndex =
        serde_json::from_str(&content).map_err(|e| format!("parse projects file: {e}"))?;
    for project in &mut index.projects {
        project.root_path = project.root_path.as_ref().and_then(|path| {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    }
    Ok(index)
}

pub fn save_project_index(dir: &Path, index: &ChatProjectIndex) -> Result<(), String> {
    let path = dir.join("projects.json");
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize projects: {e}"))?;
    atomic_write(&path, &content, "projects")
}

pub(crate) fn reorder_by_ids<T>(
    items: Vec<T>,
    ids: &[String],
    id_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut rank: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, id) in ids.iter().enumerate() {
        rank.entry(id.as_str()).or_insert(i);
    }
    let mut untouched = Vec::new();
    let mut ranked = Vec::new();
    for item in items {
        match rank.get(id_of(&item)) {
            Some(&i) => ranked.push((i, item)),
            None => untouched.push(item),
        }
    }
    ranked.sort_by_key(|(i, _)| *i);
    untouched.extend(ranked.into_iter().map(|(_, item)| item));
    untouched
}

pub fn reorder_projects(dir: &Path, ids: &[String]) -> Result<Vec<ChatProject>, String> {
    // 只重排、不接收对象：整份写回会把别处（改名/改色）的改动冲掉。
    let mut index = load_project_index(dir)?;
    index.projects = reorder_by_ids(index.projects, ids, |p| p.id.as_str());
    save_project_index(dir, &index)?;
    Ok(index.projects)
}

pub fn create_project_with_options(
    dir: &Path,
    mut project: ChatProject,
    ensure_root_dir: bool,
) -> Result<ChatProject, String> {
    validate_project_id(&project.id)?;
    project.name = normalize_project_name(&project.name)?;
    project.root_path = normalize_project_root_path(project.root_path, ensure_root_dir)?;
    let mut index = load_project_index(dir)?;
    if index.projects.iter().any(|item| item.name == project.name) {
        return Err("项目名称已存在".to_string());
    }
    index.projects.insert(0, project.clone());
    save_project_index(dir, &index)?;
    Ok(project)
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

fn normalize_project_root_path(
    root_path: Option<String>,
    ensure_dir: bool,
) -> Result<Option<String>, String> {
    let Some(root_path) = root_path else {
        return Ok(None);
    };
    let trimmed = root_path.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let expanded = expand_home_prefix(trimmed)?;
    let path = Path::new(&expanded);
    if !path.is_absolute() {
        return Err("项目文件夹必须是绝对路径。".to_string());
    }
    if path.is_dir() {
        // ok
    } else if path.exists() {
        return Err("项目路径已存在，但不是文件夹。".to_string());
    } else if ensure_dir {
        // 空白项目：在已存在的父目录下创建新文件夹。
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| "无法解析项目文件夹的父目录。".to_string())?;
        if !parent.is_dir() {
            return Err("项目文件夹的父目录不存在。".to_string());
        }
        fs::create_dir(path).map_err(|err| format!("创建项目文件夹失败：{err}"))?;
    } else {
        return Err("项目文件夹不存在或不是文件夹。".to_string());
    }
    fs::canonicalize(path)
        .map(|path| {
            Some(
                strip_windows_verbatim_prefix(path)
                    .to_string_lossy()
                    .to_string(),
            )
        })
        .map_err(|err| format!("解析项目文件夹失败：{err}"))
}

fn expand_home_prefix(raw_path: &str) -> Result<String, String> {
    if raw_path == "~" {
        return user_home_dir().map(|path| path.to_string_lossy().to_string());
    }
    if let Some(rest) = raw_path.strip_prefix("~/") {
        return user_home_dir().map(|home| home.join(rest).to_string_lossy().to_string());
    }
    #[cfg(target_os = "windows")]
    if let Some(rest) = raw_path.strip_prefix("~\\") {
        return user_home_dir().map(|home| home.join(rest).to_string_lossy().to_string());
    }
    Ok(raw_path.to_string())
}

fn user_home_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .map_err(|_| "USERPROFILE is not set".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "HOME is not set".to_string())
    }
}

pub(crate) fn validate_set_id(id: &str) -> Result<(), String> {
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

pub(crate) fn normalize_set_name(name: &str) -> Result<String, String> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err("集名称不能为空".to_string());
    }
    Ok(normalized.chars().take(80).collect())
}

pub fn load_set_index(dir: &Path) -> Result<ChatSetIndex, String> {
    let path = dir.join("sets.json");
    if !path.exists() {
        return Ok(ChatSetIndex::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read sets file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse sets file: {e}"))
}

pub fn save_set_index(dir: &Path, index: &ChatSetIndex) -> Result<(), String> {
    let path = dir.join("sets.json");
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize sets: {e}"))?;
    atomic_write(&path, &content, "sets")
}

pub fn create_set(dir: &Path, mut set: ChatSet) -> Result<ChatSet, String> {
    validate_set_id(&set.id)?;
    set.name = normalize_set_name(&set.name)?;
    let mut index = load_set_index(dir)?;
    if index.sets.iter().any(|item| item.name == set.name) {
        return Err("集名称已存在".to_string());
    }
    index.sets.insert(0, set.clone());
    save_set_index(dir, &index)?;
    Ok(set)
}

#[allow(clippy::too_many_arguments)]
pub fn update_set(
    dir: &Path,
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
    let mut index = load_set_index(dir)?;
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
    save_set_index(dir, &index)?;
    Ok(set)
}

#[allow(clippy::too_many_arguments)]
pub fn update_project(
    dir: &Path,
    project_id: &str,
    name: Option<String>,
    description: Option<String>,
    description_set: bool,
    color: Option<String>,
    color_set: bool,
    root_path: Option<String>,
    root_path_set: bool,
) -> Result<(ChatProject, String), String> {
    validate_project_id(project_id)?;
    let mut project_index = load_project_index(dir)?;
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
    if new_name.as_deref().is_some_and(|next_name| {
        next_name != old_name
            && project_index
                .projects
                .iter()
                .any(|project| project.name == next_name)
    }) {
        return Err("项目名称已存在".to_string());
    }

    if let Some(next_name) = new_name {
        project_index.projects[pos].name = next_name;
    }
    if description_set {
        project_index.projects[pos].description = description;
    }
    if color_set {
        project_index.projects[pos].color = color;
    }
    if root_path_set {
        project_index.projects[pos].root_path = normalize_project_root_path(root_path, false)?;
    }
    project_index.projects[pos].updated_at = chrono::Local::now().timestamp();
    let project = project_index.projects[pos].clone();
    save_project_index(dir, &project_index)?;

    Ok((project, old_name))
}

pub(super) fn validate_project_id(id: &str) -> Result<(), String> {
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

pub fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        const VERBATIM_UNC: &str = r"\\?\UNC\";
        const VERBATIM: &str = r"\\?\";
        let raw = path.to_string_lossy();
        if let Some(rest) = raw.strip_prefix(VERBATIM_UNC) {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = raw.strip_prefix(VERBATIM) {
            return PathBuf::from(rest);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both desktop hosts use this catalog. Protect durable values and failed-save
    // semantics through actual filesystem operations, independently of the UI.
    #[test]
    fn catalog_reopens_without_losing_fields_and_failed_updates_keep_previous_data() {
        let dir = std::env::temp_dir().join(format!("kivio-catalog-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let original = ChatSet {
            id: "set_existing".into(),
            name: "研究".into(),
            system_prompt: "保留原有提示词".into(),
            default_assistant_id: Some("asst_writer".into()),
            color: Some("blue".into()),
            created_at: 123,
            updated_at: 123,
        };
        create_set(&dir, original).unwrap();
        update_set(
            &dir,
            "set_existing",
            Some("新名称".into()),
            None,
            false,
            None,
            false,
            None,
            false,
        )
        .unwrap();
        let restored = load_set_index(&dir).unwrap().sets.remove(0);
        assert_eq!(restored.name, "新名称");
        assert_eq!(restored.system_prompt, "保留原有提示词");
        assert_eq!(
            restored.default_assistant_id.as_deref(),
            Some("asst_writer")
        );
        assert_eq!(restored.color.as_deref(), Some("blue"));
        assert_eq!(restored.created_at, 123);

        super::super::file_io::fail_atomic_replace_for_current_test(true);
        let failure = update_set(
            &dir,
            "set_existing",
            Some("不应保存".into()),
            None,
            false,
            None,
            false,
            None,
            false,
        );
        super::super::file_io::fail_atomic_replace_for_current_test(false);
        assert!(failure.is_err());
        assert_eq!(load_set_index(&dir).unwrap().sets[0].name, "新名称");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}

pub(crate) fn validate_assistant_id(id: &str) -> Result<(), String> {
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
