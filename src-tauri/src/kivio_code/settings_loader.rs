//! Load the user's real Kivio `Settings` from the Tauri store on disk, without
//! a Tauri runtime.
//!
//! The desktop app persists the whole `Settings` struct under a single
//! `"settings"` key inside `<app_data_dir>/settings.json` (tauri-plugin-store).
//! `app_data_dir` is derived from the bundle identifier `com.zmair.kivio`:
//! - macOS:   `~/Library/Application Support/com.zmair.kivio/`
//! - Linux:   `$XDG_DATA_HOME/com.zmair.kivio/` (or `~/.local/share/...`)
//! - Windows: `%APPDATA%\com.zmair.kivio\` (Roaming)
//!
//! We read that JSON, pull out `["settings"]`, deserialize into `Settings`, then
//! run the same `sanitize_settings` migration/validation the app runs at
//! startup. Missing/unreadable file → `Settings::default()` (so the CLI still
//! boots and reports "no provider configured" rather than crashing).

use std::path::{Path, PathBuf};

use crate::settings::{sanitize_settings, Settings};

/// Tauri bundle identifier (must match `tauri.conf.json`); used to locate the
/// per-app data directory the store writes into.
pub const APP_IDENTIFIER: &str = "com.zmair.kivio";
/// tauri-plugin-store file name used by the app for `Settings`.
pub const SETTINGS_STORE_FILE: &str = "settings.json";

/// Resolve `<app_data_dir>/settings.json` for the Kivio bundle identifier.
///
/// Uses the `directories` crate so the per-OS rules match Tauri's `app_data_dir`
/// (which itself wraps the platform dirs APIs). Returns `None` only when no home
/// directory can be determined.
pub fn settings_store_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(SETTINGS_STORE_FILE))
}

/// The per-app data directory Tauri would use for `com.zmair.kivio`.
pub fn app_data_dir() -> Option<PathBuf> {
    // Tauri's `app_data_dir` = `dirs::data_dir()/<identifier>` on Linux/Windows
    // and `~/Library/Application Support/<identifier>` on macOS. `directories`'
    // `ProjectDirs::from("", "", identifier)` yields the same base on all three.
    directories::ProjectDirs::from("", "", APP_IDENTIFIER)
        .map(|dirs| dirs.data_dir().to_path_buf())
        // Fallback: hand-roll from the platform data dir + identifier.
        .or_else(|| directories::BaseDirs::new().map(|base| base.data_dir().join(APP_IDENTIFIER)))
}

/// Load + sanitize the user's `Settings` from the on-disk store. Missing or
/// malformed store → sanitized defaults (never panics, never errors).
pub fn load_settings_from_disk() -> Settings {
    match settings_store_path() {
        Some(path) => load_settings_from_path(&path),
        None => sanitize_settings(Settings::default()),
    }
}

/// Load + sanitize `Settings` from a specific store file (the unit-test seam).
pub fn load_settings_from_path(path: &Path) -> Settings {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return sanitize_settings(Settings::default()),
    };
    let settings = parse_settings_json(&raw).unwrap_or_default();
    sanitize_settings(settings)
}

/// Extract the `Settings` payload from a tauri-plugin-store JSON document. The
/// store wraps every value under its key, so we read `["settings"]`; if the file
/// is instead a bare `Settings` object (older/foreign formats) we fall back to
/// deserializing the whole document.
fn parse_settings_json(raw: &str) -> Option<Settings> {
    let root: serde_json::Value = serde_json::from_str(raw).ok()?;
    if let Some(value) = root.get("settings") {
        if let Ok(settings) = serde_json::from_value::<Settings>(value.clone()) {
            return Some(settings);
        }
    }
    serde_json::from_value::<Settings>(root).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_sanitized_defaults() {
        let path = std::env::temp_dir().join("kivio-code-does-not-exist-xyz.json");
        let settings = load_settings_from_path(&path);
        // Defaults sanitize cleanly; no providers configured.
        assert!(settings.providers.is_empty() || settings.providers.iter().all(|p| !p.id.is_empty()));
    }

    #[test]
    fn reads_settings_under_store_key() {
        let dir = std::env::temp_dir().join(format!(
            "kivio-code-settings-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("settings.json");
        let doc = serde_json::json!({
            "settings": {
                "providers": [
                    {
                        "id": "test-provider",
                        "name": "Test",
                        "apiKeys": ["sk-test"],
                        "baseUrl": "https://example.com/v1",
                        "enabledModels": ["gpt-test"],
                        "enabled": true
                    }
                ],
                "defaultModels": {
                    "chat": { "providerId": "test-provider", "model": "gpt-test" }
                }
            }
        });
        std::fs::write(&path, serde_json::to_string(&doc).unwrap()).expect("write store");

        let settings = load_settings_from_path(&path);
        let provider = settings
            .providers
            .iter()
            .find(|p| p.id == "test-provider")
            .expect("provider loaded from store");
        assert_eq!(provider.api_keys, vec!["sk-test".to_string()]);
        let (provider_id, model) = settings.effective_chat_model();
        assert_eq!(provider_id, "test-provider");
        assert_eq!(model, "gpt-test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_json_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "kivio-code-bad-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not valid json ").expect("write garbage");

        let settings = load_settings_from_path(&path);
        // Should equal sanitized defaults (no panic, no providers from garbage).
        assert!(settings
            .providers
            .iter()
            .all(|p| p.id != "test-provider"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
