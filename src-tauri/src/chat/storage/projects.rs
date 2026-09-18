use super::*;

pub fn load_project_index(app: &AppHandle) -> Result<ChatProjectIndex, String> {
    let path = projects_file_path(app)?;
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

pub fn save_project_index(app: &AppHandle, index: &ChatProjectIndex) -> Result<(), String> {
    let path = projects_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize projects: {e}"))?;
    atomic_write(&path, &content, "projects")
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
            root_path: None,
            created_at: now,
            updated_at: now,
        });
        changed = true;
    }

    // 这里刻意不排序：索引里的数组顺序就是侧栏顺序，由用户拖拽决定（集同理）。
    // 加回任何 sort 都会静默抹掉用户手排的顺序且不报错 —— 见 docs/adr/0004。
    if changed {
        save_project_index(app, &project_index)?;
    }

    Ok(project_index.projects)
}

/// 按给定 id 顺序重排。规则：
/// - `ids` 里认不出的 id 直接忽略（前端拿的是旧快照时会有）；
/// - 重复 id 只认第一次；
/// - `ids` **没提到**的项保持原有相对顺序，排在**最前面** —— 唯一会出现这种情况的
///   现实场景是「前端取列表之后别处又新建了一个」，而新建就是 insert(0)，放最前正好一致。
pub(super) fn reorder_by_ids<T>(
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

pub fn reorder_projects(app: &AppHandle, ids: &[String]) -> Result<Vec<ChatProject>, String> {
    // 只重排、不接收对象：整份写回会把别处（改名/改色）的改动冲掉。
    let mut index = load_project_index(app)?;
    index.projects = reorder_by_ids(index.projects, ids, |p| p.id.as_str());
    save_project_index(app, &index)?;
    Ok(index.projects)
}

pub fn reorder_sets(app: &AppHandle, ids: &[String]) -> Result<Vec<ChatSet>, String> {
    let mut index = load_set_index(app)?;
    index.sets = reorder_by_ids(index.sets, ids, |s| s.id.as_str());
    save_set_index(app, &index)?;
    Ok(index.sets)
}

pub fn create_project(app: &AppHandle, project: ChatProject) -> Result<ChatProject, String> {
    create_project_with_options(app, project, false)
}

/// `ensure_root_dir`：root_path 尚不存在时，在父目录下创建该文件夹（「新建空白项目」用）。
pub fn create_project_with_options(
    app: &AppHandle,
    mut project: ChatProject,
    ensure_root_dir: bool,
) -> Result<ChatProject, String> {
    validate_project_id(&project.id)?;
    project.name = normalize_project_name(&project.name)?;
    project.root_path = normalize_project_root_path(project.root_path, ensure_root_dir)?;
    let mut index = load_project_index(app)?;
    if index.projects.iter().any(|item| item.name == project.name) {
        return Err("项目名称已存在".to_string());
    }
    index.projects.insert(0, project.clone());
    save_project_index(app, &index)?;
    Ok(project)
}

pub async fn update_project(
    app: &AppHandle,
    project_id: &str,
    name: Option<String>,
    description: Option<String>,
    description_set: bool,
    color: Option<String>,
    color_set: bool,
    root_path: Option<String>,
    root_path_set: bool,
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
    save_project_index(app, &project_index)?;

    if project.name != old_name {
        move_project_conversations(app, &old_name, Some(&project.id), Some(&project.name)).await?;
    }

    Ok(project)
}

pub async fn delete_project(app: &AppHandle, project_id: &str) -> Result<(), String> {
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
    move_project_conversations(app, &project.name, Some(&project.id), None).await
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
                crate::utils::strip_windows_verbatim_prefix(path)
                    .to_string_lossy()
                    .to_string(),
            )
        })
        .map_err(|err| format!("解析项目文件夹失败：{err}"))
}

/// Canonicalize, name, and cap a conversation's additional directories.
/// Drops the primary working directory (relative paths already resolve there).
/// Duplicate paths keep the first entry. Missing paths fail instead of silently vanishing.
pub fn normalize_additional_directories(
    entries: Vec<AdditionalDirectory>,
    primary_root: Option<&str>,
) -> Result<Vec<AdditionalDirectory>, String> {
    if entries.len() > MAX_ADDITIONAL_DIRECTORIES {
        return Err(format!(
            "一条对话最多附加 {MAX_ADDITIONAL_DIRECTORIES} 个目录。"
        ));
    }
    let primary = primary_root
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .and_then(|path| canonicalize_existing_dir(path).ok());
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        let normalized = normalize_additional_directory_path(entry.path)?;
        if primary
            .as_ref()
            .is_some_and(|root| paths_equal(root, &normalized))
        {
            continue;
        }
        if !seen.insert(normalized.clone()) {
            continue;
        }
        let name = entry
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .or_else(|| {
                Path::new(&normalized)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_string())
            });
        out.push(AdditionalDirectory {
            path: normalized,
            name,
        });
    }
    if out.len() > MAX_ADDITIONAL_DIRECTORIES {
        return Err(format!(
            "一条对话最多附加 {MAX_ADDITIONAL_DIRECTORIES} 个目录。"
        ));
    }
    Ok(out)
}

fn canonicalize_existing_dir(raw: &str) -> Result<String, String> {
    let expanded = expand_home_prefix(raw.trim())?;
    let path = Path::new(&expanded);
    if !path.is_absolute() {
        return Err("附加目录必须是绝对路径。".to_string());
    }
    if !path.is_dir() {
        if path.exists() {
            return Err(format!("附加路径不是文件夹：{expanded}"));
        }
        return Err(format!("附加目录不存在：{expanded}"));
    }
    fs::canonicalize(path)
        .map(|path| {
            crate::utils::strip_windows_verbatim_prefix(path)
                .to_string_lossy()
                .to_string()
        })
        .map_err(|err| format!("解析附加目录失败：{err}"))
}

fn normalize_additional_directory_path(raw: String) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("附加目录路径不能为空。".to_string());
    }
    canonicalize_existing_dir(trimmed)
}

fn paths_equal(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        Path::new(left) == Path::new(right)
    }
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

pub fn find_project_by_id(app: &AppHandle, project_id: &str) -> Result<ChatProject, String> {
    validate_project_id(project_id)?;
    load_project_index(app)?
        .projects
        .into_iter()
        .find(|project| project.id == project_id)
        .ok_or_else(|| "项目不存在".to_string())
}

pub fn find_project_by_name(app: &AppHandle, name: &str) -> Result<Option<ChatProject>, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(load_project_index(app)?
        .projects
        .into_iter()
        .find(|project| project.name == trimmed))
}

async fn move_project_conversations(
    app: &AppHandle,
    old_name: &str,
    old_project_id: Option<&str>,
    next_name: Option<&str>,
) -> Result<(), String> {
    crate::chat::repository::repository(app)
        .bulk_mutate(app, |conversation| {
            let belongs_to_project = conversation.folder.as_deref() == Some(old_name)
                || old_project_id
                    .map(|project_id| conversation.project_id.as_deref() == Some(project_id))
                    .unwrap_or(false);
            if !belongs_to_project {
                return Ok(false);
            }
            conversation.folder = next_name.map(str::to_string);
            if next_name.is_none() {
                conversation.project_id = None;
            }
            Ok(true)
        })
        .await
        .map(|_| ())
        .map_err(crate::chat::repository::repository_error)
}
