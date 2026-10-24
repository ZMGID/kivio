//! Native host for the built-in ego lite plugin; package plugins keep their own owner.
use super::{PluginMeta, catalog_plugin, probe_version, read_meta, resolve_binary, write_meta};
use crate::native_preferences::{self, Change, FieldEdit};
use std::path::Path;
static OPERATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const ID: &str = "ego-lite";
#[derive(Clone, Default)]
pub struct Status {
    pub installed: bool,
    pub enabled: bool,
    pub skill_ready: bool,
    pub effective: bool,
    pub version: Option<String>,
}
pub fn status(root: &Path) -> Result<Status, String> {
    super::configure_directory(root)?;
    let binary = resolve_binary(ID);
    let installed = binary.is_some();
    let enabled = installed && super::is_enabled(ID);
    let skill_ready = catalog_plugin(ID).is_some_and(|catalog| {
        catalog
            .skill_ids
            .iter()
            .all(|id| super::skill_dir(ID, id).is_some_and(|path| path.join("SKILL.md").is_file()))
    });
    let settings = native_preferences::load(root)?;
    let tools = &settings.chat_tools;
    let effective = enabled
        && skill_ready
        && tools.enabled
        && tools.native_tools.skill_runtime
        && tools.native_tools.run_command
        && crate::settings::skill_globally_available(tools, "ego-browser");
    Ok(Status {
        installed,
        enabled,
        skill_ready,
        effective,
        version: binary.as_deref().and_then(probe_version),
    })
}
pub async fn install(root: &Path, emit: &(impl Fn(String) + Sync)) -> Result<Status, String> {
    let _guard = OPERATION.try_lock().map_err(|_| "插件操作正在进行")?;
    super::configure_directory(root)?;
    super::install_core::run_official(ID, emit).await?;
    status(root)
}
pub async fn set_enabled(root: &Path, enabled: bool) -> Result<(Status, Option<String>), String> {
    let _guard = OPERATION.try_lock().map_err(|_| "插件操作正在进行")?;
    super::configure_directory(root)?;
    let catalog = catalog_plugin(ID).ok_or("插件目录缺失")?;
    let binary = resolve_binary(ID);
    if enabled && binary.is_none() {
        return Err("尚未检测到 ego lite，请先安装并完成首次设置。".into());
    }
    let mut meta = read_meta(ID).unwrap_or_else(|| PluginMeta {
        id: ID.into(),
        ..Default::default()
    });
    if enabled {
        // Commit the original enablement prerequisites through the settings owner.
        let settings = native_preferences::load(root)?;
        let mut edits: Vec<FieldEdit> = [
            ("/chatTools/enabled", settings.chat_tools.enabled),
            (
                "/chatTools/nativeTools/skillRuntime",
                settings.chat_tools.native_tools.skill_runtime,
            ),
            (
                "/chatTools/nativeTools/runCommand",
                settings.chat_tools.native_tools.run_command,
            ),
        ]
        .into_iter()
        .map(|(path, before)| FieldEdit {
            path: path.into(),
            before: before.into(),
            after: true.into(),
        })
        .collect();
        let mut disabled = settings.chat_tools.disabled_skill_ids.clone();
        disabled.retain(|id| id != "ego-browser");
        edits.push(FieldEdit {
            path: "/chatTools/disabledSkillIds".into(),
            before: serde_json::json!(settings.chat_tools.disabled_skill_ids),
            after: serde_json::json!(disabled),
        });
        native_preferences::save(root, Change::Fields(edits))?;
        if meta.installed_at.is_none() {
            meta.installed_at = Some(chrono::Utc::now().to_rfc3339());
        }
        meta.version = binary.as_deref().and_then(probe_version);
    }
    meta.enabled = enabled;
    meta.id = ID.into();
    write_meta(&meta).map_err(|error| {
        if enabled {
            format!("工具开关已保存，但插件启用状态保存失败，请重试：{error}")
        } else {
            error
        }
    })?;
    crate::native_tool_runtime::invalidate_resource_catalogs();
    let warning = if enabled {
        if let Some(url) = catalog.skill_download_url {
            match super::install_core::download_skill(catalog, url).await {
                Ok(()) => None,
                Err(error) => Some(format!("插件已启用，但官方 Skill 尚未就绪：{error}")),
            }
        } else {
            None
        }
    } else {
        None
    };
    crate::native_tool_runtime::invalidate_resource_catalogs();
    Ok((status(root)?, warning))
}
