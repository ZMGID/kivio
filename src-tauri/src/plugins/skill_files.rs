//! Shared installed skill files and discovery (no installer execution).
use std::path::PathBuf;
use super::catalog::{CatalogPlugin, OFFICECLI_DOMAIN_SKILLS};
use super::state::{is_enabled, is_installed, skill_dir};
pub(super) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// 官方 `officecli skills install` 把领域 skill 写到 `~/.agents/skills/<folder>`，
/// folder 是 `pptx` 而 catalog 里的 skill_id 是 `officecli-pptx`。
pub(super) fn officecli_skill_folder(skill_id: &str) -> &str {
    OFFICECLI_DOMAIN_SKILLS
        .iter()
        .find(|(_, id)| *id == skill_id)
        .map(|(folder, _)| *folder)
        .unwrap_or(skill_id)
}

pub(super) fn skill_folder_names(skill_id: &str) -> Vec<&str> {
    let alias = officecli_skill_folder(skill_id);
    if alias == skill_id {
        vec![skill_id]
    } else {
        vec![skill_id, alias]
    }
}

pub(super) fn home_agent_skill_parents() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    vec![
        home.join(".agents").join("skills"),
        home.join(crate::app_data::user_home_name()).join("skills"),
        home.join(".claude").join("skills"),
        home.join(".cursor").join("skills"),
        home.join(".codex").join("skills"),
        home.join(".copilot").join("skills"),
        home.join(".hermes").join("skills"),
        home.join(".openclaw").join("skills"),
        home.join(".opencode").join("skills"),
    ]
}

/// 官方安装器 / `skills install` 写入的 skill 目录（含 `SKILL.md`）。
pub(crate) fn official_skill_dir(skill_id: &str) -> Option<PathBuf> {
    for folder in skill_folder_names(skill_id) {
        for parent in home_agent_skill_parents() {
            let dir = parent.join(folder);
            if dir.join("SKILL.md").is_file() {
                return Some(dir);
            }
        }
    }
    None
}


pub(crate) fn write_skill_files(catalog: &CatalogPlugin) -> Result<(), String> {
    if catalog.skill_ids.is_empty() || catalog.uses_shared_skill_dirs() {
        return Ok(());
    }
    if catalog.skill_md.trim().is_empty() {
        return Ok(());
    }
    for skill_id in catalog.skill_ids {
        let dir = skill_dir(catalog.id, skill_id)
            .ok_or_else(|| "app data directory unavailable".to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("create skill dir: {e}"))?;
        std::fs::write(dir.join("SKILL.md"), catalog.skill_md)
            .map_err(|e| format!("write skill: {e}"))?;
    }
    Ok(())
}


pub fn enabled_skill_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for plugin in super::catalog::PLUGIN_CATALOG {
        if !is_enabled(plugin.id) || !is_installed(plugin.id) {
            continue;
        }
        if plugin.uses_shared_skill_dirs() {
            for skill_id in plugin.skill_ids {
                if let Some(dir) = skill_dir(plugin.id, skill_id) {
                    if dir.join("SKILL.md").is_file() {
                        roots.push(dir);
                    }
                }
                if let Some(dir) = official_skill_dir(skill_id) {
                    if !roots.iter().any(|existing| existing == &dir) {
                        roots.push(dir);
                    }
                }
            }
            continue;
        }
        if !plugin_skill_files_complete(plugin) {
            if let Err(err) = write_skill_files(plugin) {
                eprintln!("[plugins] sync skills for {}: {err}", plugin.id);
            }
        }
        for skill_id in plugin.skill_ids {
            if let Some(dir) = skill_dir(plugin.id, skill_id) {
                if dir.join("SKILL.md").is_file() {
                    roots.push(dir);
                }
            }
        }
    }
    roots
}

pub(super) fn plugin_skill_files_complete(plugin: &super::catalog::CatalogPlugin) -> bool {
    if plugin.skill_ids.is_empty() {
        return true;
    }
    plugin.skill_ids.iter().all(|skill_id| {
        skill_dir(plugin.id, skill_id)
            .map(|d| d.join("SKILL.md").is_file())
            .unwrap_or(false)
    })
}
