//! Obsidian 本地笔记库路径：读取客户端配置、列出已登记 vault。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianVault {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
struct ObsidianConfig {
    #[serde(default)]
    vaults: HashMap<String, ObsidianVaultEntry>,
}

#[derive(Debug, Deserialize)]
struct ObsidianVaultEntry {
    path: String,
    #[serde(default)]
    name: Option<String>,
}

fn obsidian_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        directories::BaseDirs::new().map(|dirs| {
            dirs.home_dir()
                .join("Library/Application Support/obsidian/obsidian.json")
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("obsidian").join("obsidian.json"))
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".config/obsidian/obsidian.json"))
    }
}

fn vault_display_name(path: &Path, explicit: Option<&str>) -> String {
    if let Some(name) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Vault")
        .to_string()
}

pub fn list_obsidian_vaults() -> Result<Vec<ObsidianVault>, String> {
    let config_path =
        obsidian_config_path().ok_or_else(|| "Obsidian config path unavailable".to_string())?;
    if !config_path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("read {}: {e}", config_path.display()))?;
    let config: ObsidianConfig =
        serde_json::from_str(&raw).map_err(|e| format!("parse obsidian.json: {e}"))?;

    let mut vaults: Vec<ObsidianVault> = config
        .vaults
        .into_values()
        .filter_map(|entry| {
            let path = entry.path.trim();
            if path.is_empty() {
                return None;
            }
            let path_buf = PathBuf::from(path);
            if !path_buf.is_dir() {
                return None;
            }
            Some(ObsidianVault {
                name: vault_display_name(&path_buf, entry.name.as_deref()),
                path: path_buf.to_string_lossy().to_string(),
            })
        })
        .collect();

    vaults.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(vaults)
}

#[tauri::command]
pub fn list_obsidian_vaults_cmd() -> Result<Vec<ObsidianVault>, String> {
    list_obsidian_vaults()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_display_name_prefers_explicit() {
        let path = Path::new("/Users/me/Documents/MyVault");
        assert_eq!(
            vault_display_name(path, Some("personal home")),
            "personal home"
        );
        assert_eq!(vault_display_name(path, None), "MyVault");
    }
}
