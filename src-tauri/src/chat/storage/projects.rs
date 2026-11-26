use super::*;

pub fn load_project_index(app: &AppHandle) -> Result<ChatProjectIndex, String> {
    read_project_index(&projects_file_path(app)?)
}

fn read_project_index(path: &Path) -> Result<ChatProjectIndex, String> {
    if !path.exists() {
        return Ok(ChatProjectIndex::default());
    }

    let content = fs::read_to_string(path).map_err(|e| format!("read projects file: {e}"))?;
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
    let _catalog = catalog_mutation_lock();
    write_project_index(&projects_file_path(app)?, index)
}

fn write_project_index(path: &Path, index: &ChatProjectIndex) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize projects: {e}"))?;
    atomic_write(path, &content, "projects")
}

pub fn get_projects(app: &AppHandle) -> Result<Vec<ChatProject>, String> {
    let _catalog = catalog_mutation_lock();
    let path = projects_file_path(app)?;
    let mut project_index = read_project_index(&path)?;
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
        write_project_index(&path, &project_index)?;
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
    let _catalog = catalog_mutation_lock();
    let path = projects_file_path(app)?;
    let mut index = read_project_index(&path)?;
    index.projects = reorder_by_ids(index.projects, ids, |p| p.id.as_str());
    write_project_index(&path, &index)?;
    Ok(index.projects)
}

pub fn reorder_sets(app: &AppHandle, ids: &[String]) -> Result<Vec<ChatSet>, String> {
    let _catalog = catalog_mutation_lock();
    let mut index = load_set_index(app)?;
    index.sets = reorder_by_ids(index.sets, ids, |s| s.id.as_str());
    save_set_index(app, &index)?;
    Ok(index.sets)
}

pub fn create_project(app: &AppHandle, project: ChatProject) -> Result<ChatProject, String> {
    create_project_with_options(app, project, false)
}

pub(crate) enum ProjectRoot<'a> {
    Existing,
    NewFolder,
    Cloned(&'a Path),
}

pub fn create_project_with_options(
    app: &AppHandle,
    project: ChatProject,
    ensure_root_dir: bool,
) -> Result<ChatProject, String> {
    let root = if ensure_root_dir {
        ProjectRoot::NewFolder
    } else {
        ProjectRoot::Existing
    };
    register_project(&projects_file_path(app)?, project, root)
}

pub(crate) fn register_project(
    index_path: &Path,
    mut project: ChatProject,
    root: ProjectRoot<'_>,
) -> Result<ChatProject, String> {
    validate_project_id(&project.id)?;
    project.name = normalize_project_name(&project.name)?;
    let _catalog = catalog_mutation_lock();
    let mut index = read_project_index(index_path)?;
    if index.projects.iter().any(|item| item.name == project.name) {
        return Err("项目名称已存在".into());
    }
    project.root_path = match root {
        ProjectRoot::Existing => normalize_project_root_path(project.root_path)?,
        ProjectRoot::NewFolder | ProjectRoot::Cloned(_) => {
            validate_folder_name(&project.name)?;
            let raw = project
                .root_path
                .as_deref()
                .ok_or("请选择项目文件夹的存放位置。")?;
            let expanded = expand_home_prefix(raw.trim())?;
            let path = Path::new(&expanded);
            if !path.is_absolute() || path.file_name() != Some(project.name.as_ref()) {
                return Err("项目必须建在所选位置下的同名文件夹中。".into());
            }
            let parent = fs::canonicalize(path.parent().ok_or("请选择项目文件夹的存放位置。")?)
                .map_err(|err| format!("解析存放位置失败：{err}"))?;
            let target = parent.join(&project.name);
            let result = match root {
                ProjectRoot::Cloned(staged) => crate::utils::rename_no_replace(staged, &target),
                _ => fs::create_dir(&target),
            };
            result.map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    format!("存放位置下已有「{}」，请换一个名称或位置。", project.name)
                } else {
                    format!("创建项目文件夹失败：{err}")
                }
            })?;
            Some(
                crate::utils::strip_windows_verbatim_prefix(target)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    };
    index.projects.insert(0, project.clone());
    if let Err(err) = write_project_index(index_path, &index) {
        if let Some(path) = &project.root_path {
            match root {
                ProjectRoot::NewFolder => {
                    let _ = fs::remove_dir(path);
                }
                ProjectRoot::Cloned(_) => {
                    return Err(format!("项目登记失败，已克隆的文件保存在 {path}：{err}"))
                }
                ProjectRoot::Existing => {}
            }
        }
        return Err(err);
    }
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
    let (project, old_name) = {
        let _catalog = catalog_mutation_lock();
        let path = projects_file_path(app)?;
        let mut project_index = read_project_index(&path)?;
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
            project_index.projects[pos].root_path = normalize_project_root_path(root_path)?;
        }
        project_index.projects[pos].updated_at = chrono::Local::now().timestamp();
        let project = project_index.projects[pos].clone();
        write_project_index(&path, &project_index)?;
        (project, old_name)
    };

    if project.name != old_name {
        move_project_conversations(app, &old_name, Some(&project.id), Some(&project.name)).await?;
    }

    Ok(project)
}

pub async fn delete_project(app: &AppHandle, project_id: &str) -> Result<(), String> {
    validate_project_id(project_id)?;
    let project = {
        let _catalog = catalog_mutation_lock();
        let path = projects_file_path(app)?;
        let mut project_index = read_project_index(&path)?;
        let Some(pos) = project_index
            .projects
            .iter()
            .position(|project| project.id == project_id)
        else {
            return Err("项目不存在".to_string());
        };
        let project = project_index.projects.remove(pos);
        write_project_index(&path, &project_index)?;
        project
    };
    move_project_conversations(app, &project.name, Some(&project.id), None).await
}

pub(crate) fn validate_folder_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("项目名称不能为空".into());
    }
    if name.chars().count() > 80 {
        return Err("项目名称不能超过 80 个字符".into());
    }
    if name == "."
        || name == ".."
        || name.ends_with('.')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err("项目名称会用作文件夹名，不能包含 / \\ : * ? \" < > | 等字符。".into());
    }
    Ok(())
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

fn normalize_project_root_path(root_path: Option<String>) -> Result<Option<String>, String> {
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
    if !path.is_dir() {
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

#[cfg(test)]
mod registration_tests {
    use super::*;

    fn project(name: &str, path: &Path) -> ChatProject {
        ChatProject {
            id: format!("proj_{}", uuid::Uuid::new_v4()),
            name: name.into(),
            description: None,
            color: None,
            root_path: Some(path.to_string_lossy().into_owned()),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn blank_project_requires_an_unused_child_but_existing_projects_can_attach() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("projects.json");
        let existing = dir.path().join("existing");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("keep"), "original").unwrap();
        for name in ["existing", "..", "existing/sub"] {
            assert!(register_project(
                &index,
                project(name, &dir.path().join(name)),
                ProjectRoot::NewFolder
            )
            .is_err());
        }
        assert!(!index.exists());
        assert!(!existing.join("sub").exists());
        assert_eq!(
            fs::read_to_string(existing.join("keep")).unwrap(),
            "original"
        );
        register_project(&index, project("linked", &existing), ProjectRoot::Existing).unwrap();
        let new = dir.path().join("new");
        register_project(&index, project("new", &new), ProjectRoot::NewFolder).unwrap();
        assert!(new.is_dir());
    }

    #[test]
    fn index_write_failure_removes_an_empty_folder_and_preserves_a_published_clone() {
        let parent = tempfile::tempdir().unwrap();
        let index = parent.path().join("projects.json");
        let blank = parent.path().join("blank");
        let cloned = parent.path().join("cloned");
        let staging = tempfile::tempdir_in(parent.path()).unwrap();
        let staged = staging.path().join("repo");
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("README"), "complete").unwrap();
        fail_atomic_replace_for_current_test(true);
        let blank_result =
            register_project(&index, project("blank", &blank), ProjectRoot::NewFolder);
        let clone_result = register_project(
            &index,
            project("cloned", &cloned),
            ProjectRoot::Cloned(&staged),
        );
        fail_atomic_replace_for_current_test(false);
        assert!(blank_result.is_err());
        assert!(!blank.exists());
        assert!(!index.exists());
        let target = fs::canonicalize(&cloned).unwrap();
        let target = crate::utils::strip_windows_verbatim_prefix(target);
        assert!(clone_result.unwrap_err().contains(target.to_str().unwrap()));
        assert_eq!(
            fs::read_to_string(cloned.join("README")).unwrap(),
            "complete"
        );
    }

    #[test]
    fn concurrent_registrations_keep_every_project_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("projects.json");
        let start = std::sync::Barrier::new(12);
        std::thread::scope(|scope| {
            for n in 0..12 {
                let index = &index;
                let start = &start;
                let name = format!("project-{n}");
                let project = project(&name, &dir.path().join(&name));
                scope.spawn(move || {
                    start.wait();
                    register_project(index, project, ProjectRoot::NewFolder).unwrap();
                });
            }
        });
        let saved = read_project_index(&index).unwrap();
        assert_eq!(saved.projects.len(), 12);
        for p in saved.projects {
            assert!(Path::new(p.root_path.as_deref().unwrap()).is_dir());
        }
    }
}
