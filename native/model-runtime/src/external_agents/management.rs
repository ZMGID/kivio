//! Settings-only CLI discovery. Protocol support remains with the runtime drivers.
use super::{management_catalog, spawn};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct InstalledCli {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}
pub async fn inspect(root: &Path, id: &str) -> Result<InstalledCli, String> {
    let settings = crate::native_preferences::load(root)?;
    inspect_snapshot(&settings, id).await
}
async fn inspect_snapshot(
    settings: &crate::settings_config::Settings,
    id: &str,
) -> Result<InstalledCli, String> {
    let def = management_catalog::get(id).ok_or("未知 CLI")?;
    let custom = settings
        .chat
        .external_cli_agents
        .get(id)
        .map(|config| config.path.trim())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let path = spawn::resolve_probe(def, custom.clone()).await;
    if path.is_none() && custom.is_some() {
        return Err("指定的 CLI 路径无法启动；请检查路径或执行权限。".into());
    }
    let version = path.as_deref().and_then(spawn::cached_cli_version);
    Ok(InstalledCli { path, version })
}
pub async fn account(root: &Path, id: &str) -> Result<serde_json::Value, String> {
    let def = management_catalog::get(id).ok_or("未知 CLI")?;
    let settings = crate::native_preferences::load(root)?;
    if settings
        .chat
        .external_cli_agents
        .get(id)
        .is_some_and(|config| !config.current_provider.trim().is_empty())
    {
        return Err("此供应商 profile 的账号查询尚未接入；未改查默认账号。".into());
    }
    let cli = inspect_snapshot(&settings, id).await?;
    let path = cli.path.ok_or("未检测到 CLI，请先安装或设置路径。")?;
    if super::wsl::is_wsl_target(&path) {
        return Err("原生版本的 WSL 账号身份映射尚未接入；未查询其他账号。".into());
    }
    let mut command = spawn::cli_probe_command(&path);
    for (key, value) in def.env {
        command.env(key, value);
    }
    if let Some(config) = settings.chat.external_cli_agents.get(id) {
        for entry in &config.env {
            command.env(&entry.key, &entry.value);
        }
    }
    let cwd = super::native::workspace(root, "conv_cli_settings", None)?;
    super::account::probe_with_command(id, command, &cwd).await
}

pub async fn info(root: &Path, id: &str) -> Result<super::installer::InstallInfo, String> {
    let def = management_catalog::get(id).ok_or("未知 CLI")?;
    let settings = crate::native_preferences::load(root)?;
    let cli = inspect_snapshot(&settings, id).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    Ok(super::installer::management_info(&client, def, cli.path, cli.version).await)
}

pub async fn install(
    root: &Path,
    id: &str,
    emit: &(impl Fn(String) + Sync),
) -> Result<InstalledCli, String> {
    let def = management_catalog::get(id).ok_or("未知 CLI")?;
    if matches!(
        def.install.post_install,
        super::types::PostInstallStrategy::DshProfile
    ) {
        return Err("dsh 的 Kivio profile 初始化尚未接入原生客户端，请按官方文档安装。".into());
    }
    let settings = crate::native_preferences::load(root)?;
    let before = inspect_snapshot(&settings, id).await?;
    super::installer::run_install_command(&def.install, before.path.as_deref(), emit).await?;
    spawn::clear_probe_cache();
    let installed = inspect_snapshot(&settings, id).await?;
    let path = installed
        .path
        .as_deref()
        .ok_or("安装命令结束，但尚未检测到可执行文件，请重新检测。")?;
    let version = super::installer::verify_installed_cli(
        def,
        spawn::cli_probe_command(path),
        std::time::Duration::from_secs(20),
    )
    .await?;
    emit(format!("{} {version}", def.name));
    Ok(InstalledCli {
        version: Some(version),
        ..installed
    })
}
