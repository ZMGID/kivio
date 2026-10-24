//! Catalog-owned installation and downloaded Skill sync shared by desktop hosts.
use super::catalog::{catalog_plugin, CatalogPlugin};
use crate::proc::NoConsoleWindow;

pub async fn run_official(id: &str, emit: &(impl Fn(String) + Sync)) -> Result<(), String> {
    let catalog = catalog_plugin(id).ok_or_else(|| format!("unknown plugin: {id}"))?;
    let script = catalog
        .host_install_command()
        .ok_or("当前系统暂不支持自动安装该插件")?;
    if !catalog
        .install_commands
        .iter()
        .any(|entry| entry.command == script)
    {
        return Err("插件安装计划无效".into());
    }
    let (program, args) = official_install_program_args(script);
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    #[cfg(unix)]
    command.process_group(0);
    command
        .env("PATH", crate::path_env::command_path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .no_console_window();
    let child = command
        .spawn()
        .map_err(|error| format!("无法开始安装：{error}"))?;
    if crate::external_agents::installer::stream_install_child(child, emit, 600).await {
        Ok(())
    } else {
        Err("插件安装失败或超时，请查看安装日志后重试。".into())
    }
}

pub async fn download_skill(catalog: &CatalogPlugin, url: &str) -> Result<(), String> {
    let destination = super::state::plugin_dir(catalog.id)
        .ok_or("app data directory unavailable")?
        .join("skills");
    std::fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    crate::skills::download_skill_zip_into_checked(url, &destination, catalog.skill_ids).await?;
    if !catalog
        .skill_ids
        .iter()
        .all(|id| destination.join(id).join("SKILL.md").is_file())
    {
        return Err("下载的插件 Skill 与官方目录声明不匹配".into());
    }
    Ok(())
}

pub(super) fn official_install_program_args(script: &str) -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell.exe",
            vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-Command".into(),
                script.to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        ("bash", vec!["-lc".into(), script.to_string()])
    }
}
