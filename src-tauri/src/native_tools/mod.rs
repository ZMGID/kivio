mod fetch;
mod files;
mod sandbox_exports;
mod shell;

pub use fetch::web_fetch;
pub use files::{
    edit_file, glob_files, list_dir, read_file, search_files, write_file, FileMutationResult,
    ReadFileResult, ReadFileState,
};
pub use sandbox_exports::{
    cleanup_stale_sandbox_exports, export_sandbox_artifacts, format_export_error,
    format_exported_paths, remove_sandbox_exports_for_conversation, resolve_sandbox_export_file_path,
    SandboxExportContext,
};
pub use shell::run_command;

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub const MAX_READ_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Block writes/edits under these path segments (relative to home).
const WRITE_BLOCKLIST_SEGMENTS: &[&str] = &[
    ".ssh",
    ".gnupg",
    "Library/Keychains",
    "Library/Application Support/Keychain",
];

#[derive(Debug, Clone)]
pub struct ProjectWorkspaceContext {
    pub project_id: String,
    pub project_name: String,
    pub root_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct NativeToolWorkspace {
    pub project: Option<ProjectWorkspaceContext>,
    pub workspace_roots: Vec<String>,
}

impl NativeToolWorkspace {
    pub fn global(workspace_roots: &[String]) -> Self {
        Self {
            project: None,
            workspace_roots: workspace_roots.to_vec(),
        }
    }

    pub fn project(project_id: String, project_name: String, root_path: Option<String>) -> Self {
        Self {
            project: Some(ProjectWorkspaceContext {
                project_id,
                project_name,
                root_path: root_path.map(PathBuf::from),
            }),
            workspace_roots: Vec::new(),
        }
    }

    pub fn has_project(&self) -> bool {
        self.project.is_some()
    }
}

pub fn user_home_dir() -> Result<PathBuf, String> {
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

pub fn resolve_workspace_path(
    raw_path: &str,
    workspace_roots: &[String],
) -> Result<PathBuf, String> {
    let home = user_home_dir()?;
    let home_canon = fs_canonicalize_existing_or_self(&home)?;
    let candidate = candidate_path(raw_path)?;
    let canonical = fs_canonicalize_existing_or_self(&candidate)?;
    if !canonical.starts_with(&home_canon) {
        return Err("路径不在允许范围内：只能访问用户主目录下的文件。".to_string());
    }

    if !workspace_roots.is_empty() {
        let allowed = workspace_roots.iter().any(|root| {
            let expanded = match expand_home_prefix(root.trim()) {
                Ok(path) => path,
                Err(_) => return false,
            };
            if expanded.is_empty() {
                return false;
            }
            let root_path = Path::new(&expanded);
            let root_canon = fs_canonicalize_existing_or_self(root_path).ok();
            root_canon
                .map(|root_canon| {
                    root_canon.starts_with(&home_canon) && canonical.starts_with(&root_canon)
                })
                .unwrap_or(false)
        });
        if !allowed {
            return Err(
                "路径不在允许的工作区根目录内，请到设置 > MCP > Kivio 内置工具检查 workspaceRoots。"
                    .to_string(),
            );
        }
    }

    Ok(canonical)
}

pub fn resolve_tool_read_path(
    workspace: &NativeToolWorkspace,
    raw_path: &str,
) -> Result<PathBuf, String> {
    if workspace.has_project() {
        // An explicit absolute or ~/ path may leave the project root for reads,
        // matching non-project conversations (reads are unrestricted there).
        let expanded = expand_home_prefix(raw_path)?;
        if Path::new(&expanded).is_absolute() {
            return resolve_read_path(raw_path);
        }
        return resolve_project_path(workspace, raw_path, false);
    }
    resolve_read_path(raw_path)
}

pub fn resolve_tool_write_path(
    workspace: &NativeToolWorkspace,
    raw_path: &str,
) -> Result<PathBuf, String> {
    if workspace.has_project() {
        if let Some(path) = resolve_project_escape_write_path(workspace, raw_path, false)? {
            return Ok(path);
        }
        return resolve_project_path(workspace, raw_path, true);
    }
    let path = resolve_workspace_path(raw_path, &workspace.workspace_roots)?;
    assert_writable_path(&path)?;
    Ok(path)
}

/// In a project conversation, an explicit absolute or ~/ path that resolves
/// outside the project root falls back to the global write rules (home-bounded,
/// blocklist, workspace_roots) instead of being rejected. Relative paths never
/// escape. Returns Ok(None) when the path belongs to the normal project flow.
fn resolve_project_escape_write_path(
    workspace: &NativeToolWorkspace,
    raw_path: &str,
    entry: bool,
) -> Result<Option<PathBuf>, String> {
    let expanded = expand_home_prefix(raw_path)?;
    let raw = Path::new(&expanded);
    if !raw.is_absolute() {
        return Ok(None);
    }
    if let Ok(root) = project_root_required(workspace) {
        // Entry semantics must not follow a final symlink, or an in-root link
        // pointing outside would be misclassified as an escape and the global
        // flow would delete the link target instead of the link itself.
        let probed = if entry {
            canonicalize_entry_or_missing(raw)?
        } else {
            canonicalize_existing_or_missing(raw, true)?
        };
        if probed.starts_with(&root) {
            return Ok(None);
        }
    }
    let path = resolve_workspace_path(raw_path, &workspace.workspace_roots)?;
    assert_writable_path(&path)?;
    Ok(Some(path))
}

pub fn resolve_tool_existing_dir(
    workspace: &NativeToolWorkspace,
    raw_path: Option<&str>,
) -> Result<PathBuf, String> {
    if workspace.has_project() {
        let path = match raw_path.map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) => resolve_project_path(workspace, path, false)?,
            None => project_root_required(workspace)?,
        };
        if !path.is_dir() {
            return Err(format!(
                "Working directory is not a directory: {}",
                path.display()
            ));
        }
        return Ok(path);
    }

    if let Some(cwd_arg) = raw_path.map(str::trim).filter(|path| !path.is_empty()) {
        let path = resolve_read_path(cwd_arg)?;
        if !path.is_dir() {
            return Err(format!(
                "Working directory is not a directory: {}",
                path.display()
            ));
        }
        return Ok(path);
    }

    if let Some(root) = workspace
        .workspace_roots
        .iter()
        .map(|root| root.trim())
        .find(|root| !root.is_empty())
    {
        let path = resolve_read_path(root)?;
        if !path.is_dir() {
            return Err(format!(
                "Working directory is not a directory: {}",
                path.display()
            ));
        }
        return Ok(path);
    }

    user_home_dir()
}

pub fn workspace_display_path(workspace: &NativeToolWorkspace, path: &Path) -> String {
    if let Some(project) = &workspace.project {
        if let Some(root) = project.root_path.as_ref() {
            if let Ok(root_canon) = fs_canonicalize_existing_or_self(root) {
                if let Ok(relative) = path.strip_prefix(&root_canon) {
                    let rel = relative.to_string_lossy();
                    return if rel.is_empty() {
                        ".".to_string()
                    } else {
                        rel.to_string()
                    };
                }
            }
        }
    }
    path.display().to_string()
}

pub fn resolve_read_path(raw_path: &str) -> Result<PathBuf, String> {
    let candidate = candidate_path(raw_path)?;
    fs_canonicalize_existing_or_self(&candidate)
}

fn project_root_required(workspace: &NativeToolWorkspace) -> Result<PathBuf, String> {
    let Some(project) = &workspace.project else {
        return Err("No active project workspace.".to_string());
    };
    let Some(root_path) = project.root_path.as_ref() else {
        return Err(format!(
            "项目「{}」尚未绑定本地文件夹。请先在项目菜单中绑定文件夹，再使用文件或命令工具。",
            project.project_name
        ));
    };
    if !root_path.is_dir() {
        return Err(format!(
            "项目「{}」绑定的文件夹不存在或不可访问：{}",
            project.project_name,
            root_path.display()
        ));
    }
    fs::canonicalize(root_path).map_err(|err| format!("Resolve project root failed: {err}"))
}

fn resolve_project_path(
    workspace: &NativeToolWorkspace,
    raw_path: &str,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    let root = project_root_required(workspace)?;
    let expanded = expand_home_prefix(raw_path)?;
    if expanded.trim().is_empty() {
        return Err("路径为空，请提供项目内的文件或目录路径。".to_string());
    }

    let raw = Path::new(&expanded);
    for component in raw.components() {
        if matches!(component, Component::ParentDir) {
            return Err("项目路径不能包含 '..'，请使用项目内的明确相对路径。".to_string());
        }
    }

    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    let resolved = canonicalize_existing_or_missing(&candidate, allow_missing)?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "路径不在当前项目根目录内。项目「{}」根目录：{}",
            workspace
                .project
                .as_ref()
                .map(|project| project.project_name.as_str())
                .unwrap_or(""),
            root.display()
        ));
    }
    Ok(resolved)
}

fn resolve_project_entry_path(
    workspace: &NativeToolWorkspace,
    raw_path: &str,
) -> Result<PathBuf, String> {
    let root = project_root_required(workspace)?;
    let expanded = expand_home_prefix(raw_path)?;
    if expanded.trim().is_empty() {
        return Err("路径为空，请提供项目内的文件或目录路径。".to_string());
    }

    let raw = Path::new(&expanded);
    for component in raw.components() {
        if matches!(component, Component::ParentDir) {
            return Err("项目路径不能包含 '..'，请使用项目内的明确相对路径。".to_string());
        }
    }

    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    let resolved = canonicalize_entry_or_missing(&candidate)?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "路径不在当前项目根目录内。项目「{}」根目录：{}",
            workspace
                .project
                .as_ref()
                .map(|project| project.project_name.as_str())
                .unwrap_or(""),
            root.display()
        ));
    }
    Ok(resolved)
}

fn canonicalize_existing_or_missing(path: &Path, _allow_missing: bool) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|err| format!("Resolve path failed: {err}"));
    }

    let mut missing = Vec::new();
    let mut current = path;
    while !current.exists() {
        let name = current
            .file_name()
            .ok_or_else(|| "Invalid path".to_string())?;
        missing.push(name.to_os_string());
        current = current.parent().ok_or_else(|| "Invalid path".to_string())?;
    }

    let mut resolved =
        fs::canonicalize(current).map_err(|err| format!("Resolve path failed: {err}"))?;
    for name in missing.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn canonicalize_entry_or_missing(path: &Path) -> Result<PathBuf, String> {
    if fs::symlink_metadata(path).is_ok() {
        let parent = path.parent().ok_or_else(|| "Invalid path".to_string())?;
        let parent =
            fs::canonicalize(parent).map_err(|err| format!("Resolve path failed: {err}"))?;
        let name = path.file_name().ok_or_else(|| "Invalid path".to_string())?;
        return Ok(parent.join(name));
    }
    canonicalize_existing_or_missing(path, true)
}

pub fn assert_writable_path(path: &Path) -> Result<(), String> {
    let home = user_home_dir()?;
    let home_canon = fs_canonicalize_existing_or_self(&home)?;
    let canonical = if path.exists() {
        fs::canonicalize(path).map_err(|err| format!("Resolve path failed: {err}"))?
    } else if let Some(parent) = path.parent() {
        let parent_canon = fs_canonicalize_existing_or_self(parent)?;
        parent_canon.join(path.file_name().ok_or_else(|| "Invalid path".to_string())?)
    } else {
        return Err("Invalid path".to_string());
    };

    if !canonical.starts_with(&home_canon) {
        return Err("路径不在允许范围内：只能写入用户主目录下的文件。".to_string());
    }

    let relative = canonical
        .strip_prefix(&home_canon)
        .map_err(|_| "Invalid path".to_string())?;
    let rel = relative.to_string_lossy();
    for blocked in WRITE_BLOCKLIST_SEGMENTS {
        if rel.as_ref() == *blocked || rel.starts_with(&format!("{blocked}/")) {
            return Err(format!("出于安全策略，禁止写入 {blocked} 目录。"));
        }
    }
    Ok(())
}

/// Expands a leading `~` / `~/` / `~\` to the user home directory (shell-style).
fn expand_home_prefix(raw_path: &str) -> Result<String, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed == "~" {
        return Ok(user_home_dir()?.to_string_lossy().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = user_home_dir()?;
        return Ok(home.join(rest).to_string_lossy().to_string());
    }
    #[cfg(target_os = "windows")]
    if let Some(rest) = trimmed.strip_prefix("~\\") {
        let home = user_home_dir()?;
        return Ok(home.join(rest).to_string_lossy().to_string());
    }
    Ok(trimmed.to_string())
}

fn candidate_path(raw_path: &str) -> Result<PathBuf, String> {
    let expanded = expand_home_prefix(raw_path)?;
    if expanded.is_empty() {
        return Err("路径为空，请提供要访问的文件或目录路径。".to_string());
    }

    let candidate = {
        let path = Path::new(&expanded);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            user_home_dir()?.join(path)
        }
    };

    for component in candidate.components() {
        if matches!(component, Component::ParentDir) {
            return Err("路径不能包含 '..'，请使用明确的文件路径。".to_string());
        }
    }

    Ok(candidate)
}

fn fs_canonicalize_existing_or_self(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        fs::canonicalize(path).map_err(|err| format!("Resolve path failed: {err}"))
    } else {
        Ok(path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_dir_traversal() {
        let err = resolve_workspace_path("../etc/passwd", &[]).unwrap_err();
        assert!(err.contains(".."));
    }

    #[test]
    fn resolve_read_path_allows_temp_paths() {
        let file = std::env::temp_dir().join(format!("kivio_read_{}.txt", uuid::Uuid::new_v4()));
        fs::write(&file, "hello").expect("write temp file");

        let resolved = resolve_read_path(&file.to_string_lossy()).expect("resolve read path");
        assert_eq!(
            resolved,
            fs::canonicalize(&file).expect("canonical temp file")
        );

        let _ = fs::remove_file(file);
    }

    #[test]
    fn resolve_read_path_expands_tilde_home_prefix() {
        let home = user_home_dir().expect("home");
        let dir = home.join(format!(".kivio_tilde_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("sample.csv");
        fs::write(&file, "alpha,beta").expect("write");

        let rel = dir
            .strip_prefix(&home)
            .expect("dir under home")
            .to_string_lossy();
        let tilde_path = format!("~/{}/sample.csv", rel);
        let resolved = resolve_read_path(&tilde_path).expect("resolve tilde path");
        assert_eq!(resolved, fs::canonicalize(&file).expect("canonical file"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_workspace_path_expands_tilde_prefix() {
        let home = user_home_dir().expect("home");
        let dir = home.join(format!(".kivio_tilde_ws_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("note.txt");
        fs::write(&file, "ok").expect("write");

        let rel = dir
            .strip_prefix(&home)
            .expect("dir under home")
            .to_string_lossy();
        let tilde_root = format!("~/{rel}");
        let resolved =
            resolve_workspace_path(&format!("~/{rel}/note.txt"), &[tilde_root]).expect("resolve");
        assert_eq!(resolved, fs::canonicalize(&file).expect("canonical file"));

        let _ = fs::remove_dir_all(&dir);
    }
}
