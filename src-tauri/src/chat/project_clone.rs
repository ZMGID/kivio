//! 「克隆 Git 仓库」新建项目：校验地址与文件夹名，克隆到存放位置下的新文件夹，再登记为项目。
//! Git 仅写临时目录，成功后独占发布，绝不递归清理用户可见的目标路径。

use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::AppHandle;

use crate::chat::{
    storage::{projects_file_path, register_project, validate_folder_name, ProjectRoot},
    ChatProject,
};

const CLONE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 接受 https / http / ssh / git 地址、`user@host:path`、`github.com/owner/repo` 与 `owner/repo` 简写。
/// 其余协议（file、ext:: 等）一律拒绝。
pub(crate) fn normalize_clone_url(input: &str) -> Result<String, String> {
    let url = input.trim();
    if url.is_empty() {
        return Err("请输入仓库地址。".into());
    }
    if url.starts_with('-') || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("仓库地址无效。".into());
    }
    let lower = url.to_ascii_lowercase();
    if ["https://", "http://", "ssh://", "git://"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return Ok(url.to_string());
    }
    if lower.starts_with("github.com/") {
        return Ok(format!("https://{url}"));
    }
    if let Some((user_host, path)) = url.split_once(':') {
        if user_host.contains('@') && !user_host.contains('/') && !path.is_empty() {
            return Ok(url.to_string());
        }
    }
    let segment = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if let Some((owner, repo)) = url.split_once('/') {
        if segment(owner) && segment(repo) {
            return Ok(format!("https://github.com/{owner}/{repo}"));
        }
    }
    Err("无法识别的仓库地址：请填写 Git 地址或 GitHub 的 owner/repo。".into())
}

async fn run_git_clone(url: &str, target: &Path) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args([
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "protocol.file.allow=never",
        "clone",
        "--",
        url,
    ])
    .arg(target)
    .env("GIT_TERMINAL_PROMPT", "0")
    .kill_on_drop(true);
    // 没有终端可输入：SSH 需要交互（主机指纹、口令）时直接失败，而不是挂住。
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    #[cfg(windows)]
    {
        use crate::proc::NoConsoleWindow;
        cmd.no_console_window();
    }
    let output = tokio::time::timeout(CLONE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "克隆超时（10 分钟）。".to_string())?
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                "未找到 git，请先安装 Git。".to_string()
            } else {
                format!("启动 git 失败：{err}")
            }
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("Cloning into"))
        .last()
        .unwrap_or("git clone 返回错误");
    Err(format!("克隆失败：{reason}"))
}

pub(crate) async fn clone_project(
    app: &AppHandle,
    url: &str,
    parent_dir: &str,
    name: &str,
) -> Result<ChatProject, String> {
    let url = normalize_clone_url(url)?;
    let name = name.trim();
    validate_folder_name(name)?;
    // 克隆可能很久：先挡掉重名，免得下完才失败。
    if crate::chat::storage::find_project_by_name(app, name)?.is_some() {
        return Err("项目名称已存在".into());
    }
    let parent = PathBuf::from(parent_dir.trim());
    if !parent.is_absolute() || !parent.is_dir() {
        return Err("存放位置不存在或不是文件夹。".into());
    }
    let target = parent.join(name);
    if target.exists() {
        return Err(format!("存放位置下已有「{name}」，请换一个名称或位置。"));
    }
    let staging = tempfile::Builder::new()
        .prefix(".kivio-clone-")
        .tempdir_in(&parent)
        .map_err(|err| format!("创建临时克隆目录失败：{err}"))?;
    let staged = staging.path().join("repo");
    run_git_clone(&url, &staged).await?;
    let now = chrono::Local::now().timestamp();
    let project = ChatProject {
        id: format!("proj_{}", uuid::Uuid::new_v4()),
        name: name.to_string(),
        description: None,
        color: None,
        root_path: Some(target.to_string_lossy().to_string()),
        created_at: now,
        updated_at: now,
    };
    register_project(
        &projects_file_path(app)?,
        project,
        ProjectRoot::Cloned(&staged),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::rename_no_replace;

    #[test]
    fn accepts_common_git_addresses() {
        for url in [
            "https://github.com/owner/repo.git",
            "http://git.example.com/a/b",
            "ssh://git@host/a/b.git",
            "git://host/a.git",
            "git@github.com:owner/repo.git",
        ] {
            assert_eq!(normalize_clone_url(url).unwrap(), url);
        }
        assert_eq!(
            normalize_clone_url(" github.com/owner/repo ").unwrap(),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_clone_url("owner/repo.js").unwrap(),
            "https://github.com/owner/repo.js"
        );
    }

    #[test]
    fn rejects_unsafe_or_unknown_addresses() {
        for url in [
            "",
            "--upload-pack=touch /tmp/x",
            "ext::sh -c touch% /tmp/pwned",
            "file:///etc",
            "/local/path",
            "a/b/c",
            "../repo",
            "owner/repo name",
        ] {
            assert!(normalize_clone_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn a_late_clone_cannot_replace_an_empty_or_completed_destination() {
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("target");
        let first = parent.path().join("first");
        let second = parent.path().join("second");
        for path in [&first, &second, &target] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(first.join("result"), "first").unwrap();
        std::fs::write(second.join("result"), "second").unwrap();
        assert!(rename_no_replace(&first, &target).is_err());
        assert_eq!(target.read_dir().unwrap().count(), 0);
        std::fs::remove_dir(&target).unwrap();
        rename_no_replace(&first, &target).unwrap();
        assert!(rename_no_replace(&second, &target).is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("result")).unwrap(),
            "first"
        );
    }

    #[test]
    fn folder_name_cannot_escape_location() {
        assert!(validate_folder_name("my-repo").is_ok());
        assert!(validate_folder_name("新项目 2").is_ok());
        for name in ["", ".", "..", "a/b", "a\\b", "c:", "x."] {
            assert!(validate_folder_name(name).is_err(), "{name}");
        }
    }
}
