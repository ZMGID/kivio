use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::chat::storage::{
    conversations_dir, find_project_by_id, load_conversation, resolve_conversation_project,
};
use crate::chat::types::ChatProject;
use crate::external_agents::types::RuntimeAgentDef;
use crate::utils::strip_windows_verbatim_prefix;

pub fn resolve_effective_cwd(
    app: &AppHandle,
    conversation_id: &str,
    project_id: Option<&str>,
) -> Result<PathBuf, String> {
    let project = project_cli_root_for_conversation(app, conversation_id, project_id)?;
    if let Some(original) = bound_original_cwd(app, conversation_id)? {
        return validate_bound_cwd(&original, project.as_deref());
    }
    if let Some(path) = project {
        return Ok(path);
    }

    // Native sessions keep their original cwd when resumed, even if the default changes.
    if let Some(handle) = crate::external_agents::session::load_live_handle(app, conversation_id) {
        if !handle.cwd.trim().is_empty() {
            return Ok(strip_windows_verbatim_prefix(PathBuf::from(handle.cwd)));
        }
    }
    let legacy = conversation_workspace_path(app, conversation_id)?;
    if legacy.exists() {
        return Ok(legacy);
    }
    let settings = crate::settings::load_settings(app);
    crate::native_tools::ordinary_workspace_directory(
        &settings.chat_tools.native_tools.working_directory,
        conversation_id,
    )
    .map(strip_windows_verbatim_prefix)
}

pub(crate) fn bound_original_cwd(app: &AppHandle, id: &str) -> Result<Option<String>, String> {
    if let Some(record) = crate::external_agents::import::load_import_record(app, id) {
        return crate::external_agents::import::original_import_cwd(&record)
            .or_else(|| {
                crate::external_agents::session::load_live_handle(app, id).map(|handle| handle.cwd)
            })
            .map(Some)
            .ok_or_else(|| "无法确认导入会话的原工作目录，请重新导入".into());
    }
    if let Some(cwd) = load_conversation(app, id)
        .ok()
        .and_then(|conversation| conversation.external_cwd)
    {
        return Ok(Some(cwd));
    }
    if let Some(handle) = crate::external_agents::session::load_live_handle(app, id) {
        return Ok(Some(handle.cwd));
    }
    if let Some(binding) = crate::external_agents::session::load_session(app, id) {
        return crate::external_agents::import::native_session_original_cwd(
            &binding.agent_id,
            &binding.session_id,
        )
        .map(Some)
        .ok_or_else(|| "无法确认原生会话的原工作目录".into());
    }
    Ok(None)
}

pub(crate) fn validate_bound_cwd(
    original: &str,
    requested: Option<&Path>,
) -> Result<PathBuf, String> {
    let path = cli_dir_if_exists(PathBuf::from(original))
        .or_else(|| {
            requested
                .filter(|path| {
                    crate::external_agents::import::paths_match(original, &path.to_string_lossy())
                })
                .and_then(|path| cli_dir_if_exists(path.to_path_buf()))
        })
        .ok_or_else(|| format!("原工作目录不存在：{original}"))?;
    if requested.is_some_and(|requested| {
        !requested.is_dir()
            || !crate::external_agents::import::paths_match(original, &requested.to_string_lossy())
    }) {
        return Err(format!("此会话只能在原工作目录续聊：{original}"));
    }
    Ok(path)
}

pub(crate) fn validate_working_directory_override(
    app: &AppHandle,
    id: &str,
    path: &Path,
) -> Result<(), String> {
    if let Some(original) = bound_original_cwd(app, id)? {
        resolve_effective_cwd(app, id, None)?;
        validate_bound_cwd(&original, Some(path))?;
    }
    Ok(())
}

fn project_cli_root_for_conversation(
    app: &AppHandle,
    conversation_id: &str,
    project_id: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    if let Ok(conversation) = load_conversation(app, conversation_id) {
        // 选了独立 worktree 的对话，外部 CLI 也在 worktree 里运行。
        if let Some(worktree) = conversation.worktree.as_ref() {
            return cli_dir_if_exists(PathBuf::from(&worktree.path))
                .map(Some)
                .ok_or_else(|| format!("worktree 目录不存在：{}", worktree.path));
        }
        if let Some(project) = resolve_conversation_project(app, &conversation)? {
            return project_cli_root(project).map(Some);
        }
        return Ok(None);
    }
    let Some(project_id) = project_id.filter(|id| !id.trim().is_empty()) else {
        return Ok(None);
    };
    project_cli_root(find_project_by_id(app, project_id)?).map(Some)
}

pub(crate) fn project_cli_root(project: ChatProject) -> Result<PathBuf, String> {
    let root = project
        .root_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            format!(
                "Project {} has no working directory configured",
                project.name
            )
        })?;
    cli_dir_if_exists(PathBuf::from(root)).ok_or_else(|| {
        format!(
            "Project {} working directory does not exist or is not a directory: {root}",
            project.name
        )
    })
}

/// Project roots are stored via `fs::canonicalize` (`\\?\E:\...` on Windows).
/// External CLIs (Node `dsh` especially) need the non-verbatim spelling.
pub fn cli_dir_if_exists(path: PathBuf) -> Option<PathBuf> {
    let stripped = strip_windows_verbatim_prefix(path);
    stripped.is_dir().then_some(stripped)
}

/// 会话工作区路径，**不创建目录**。
///
/// 目录要不要建取决于调用方：真的要往里写（spawn CLI）才建，只是想知道路径
/// （dock 展示 / 上下文统计）就别建。原来这里无条件 `create_dir_all`，于是每开一个
/// 新会话都留一个空壳——实测 110 个 conv 目录里 105 个是空的。
fn conversation_workspace_path(app: &AppHandle, conversation_id: &str) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?
        .parent()
        .ok_or_else(|| "chat data root unavailable".to_string())?
        .join("chat-workspaces")
        .join(conversation_id))
}

/// `resolve_effective_cwd` + 确保目录存在。**只给真的要在里面跑进程/写文件的路径用**
/// （spawn 外部 CLI、压缩）。项目根不会被创建（配错路径不该被悄悄建成空目录）。
pub fn ensure_effective_cwd(
    app: &AppHandle,
    conversation_id: &str,
    project_id: Option<&str>,
) -> Result<PathBuf, String> {
    let path = resolve_effective_cwd(app, conversation_id, project_id)?;
    // 项目根由用户配置，存在性已在 resolve 里校验过（is_dir 才返回）；只有会话工作区
    // 这条分支需要按需创建。
    if !path.is_dir() {
        std::fs::create_dir_all(&path).map_err(|e| format!("create workspace: {e}"))?;
    }
    Ok(path)
}

/// 模型/斜杠命令探测按项目根或默认目录缓存，普通对话共用稳定 scope。
/// 与新对话的执行目录一致，以便发现该目录下的 CLI 配置。
pub fn resolve_detection_cwd(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(conversation_id) = conversation_id.filter(|id| !id.trim().is_empty()) {
        if let Some(path) = project_cli_root_for_conversation(app, conversation_id, None)? {
            return Ok(path);
        }
    }
    let settings = crate::settings::load_settings(app);
    let base = crate::native_tools::default_working_directory(
        &settings.chat_tools.native_tools.working_directory,
    )?;
    let base = strip_windows_verbatim_prefix(base);
    std::fs::create_dir_all(&base).map_err(|e| format!("create detection workspace: {e}"))?;
    Ok(base)
}

pub fn extra_allowed_dirs_for_agent(
    def: &RuntimeAgentDef,
    skill_scan_paths: &[String],
) -> Vec<String> {
    if def.id == "codex" {
        return Vec::new();
    }
    skill_scan_paths
        .iter()
        .filter(|p| !p.trim().is_empty() && Path::new(p).is_dir())
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn bound_directory_rejects_relocation_and_missing_original() {
        let original = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert!(
            validate_bound_cwd(&original.path().to_string_lossy(), Some(other.path())).is_err()
        );
        assert!(
            validate_bound_cwd(&original.path().join("missing").to_string_lossy(), None).is_err()
        );
        assert!(
            validate_bound_cwd(&original.path().to_string_lossy(), Some(original.path())).is_ok()
        );
    }

    #[test]
    #[cfg(windows)]
    fn bound_directory_accepts_windows_junction() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        let alias = directory.path().join("alias");
        std::fs::create_dir(&original).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(&original)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(validate_bound_cwd(&original.to_string_lossy(), Some(&alias)).is_ok());
        std::fs::remove_dir(alias).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn bound_directory_accepts_posix_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        let alias = directory.path().join("alias");
        std::fs::create_dir(&original).unwrap();
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        assert!(validate_bound_cwd(&original.to_string_lossy(), Some(&alias)).is_ok());
    }

    #[test]
    fn bound_project_requires_an_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "not a directory").unwrap();
        let project = |root: Option<String>| ChatProject {
            id: "project_test".into(),
            name: "Test".into(),
            description: None,
            color: None,
            root_path: root,
            created_at: 0,
            updated_at: 0,
        };
        for root in [
            None,
            Some(" ".into()),
            Some(dir.path().join("missing").display().to_string()),
            Some(file.display().to_string()),
        ] {
            assert!(
                project_cli_root(project(root.clone())).is_err(),
                "invalid bound project must fail instead of falling back: {root:?}"
            );
        }
        assert_eq!(
            project_cli_root(project(Some(dir.path().display().to_string()))).unwrap(),
            dir.path().to_path_buf()
        );
    }

    #[test]
    fn cli_dir_if_exists_rejects_missing_paths() {
        assert_eq!(
            cli_dir_if_exists(PathBuf::from(
                r"\\?\E:\this-folder-does-not-exist-kivio-cwd-test"
            )),
            None
        );
    }

    #[test]
    fn cli_dir_if_exists_accepts_a_real_directory() {
        let cwd = std::env::current_dir().expect("cwd");
        #[cfg(windows)]
        let verbatim = PathBuf::from(format!(r"\\?\{}", cwd.display()));
        #[cfg(not(windows))]
        let verbatim = cwd.clone();
        let resolved = cli_dir_if_exists(verbatim).expect("cwd is a directory");
        assert!(resolved.is_dir());
        #[cfg(windows)]
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "cli cwd must not keep a verbatim prefix: {}",
            resolved.display()
        );
    }
}
