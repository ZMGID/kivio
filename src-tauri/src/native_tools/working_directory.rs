//! Shared desktop workbench paths; resolution never creates user directories.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Shared working directory for ordinary chats. Resolving it never creates it.
pub fn default_working_directory(working_directory: &str) -> Result<PathBuf, String> {
    let configured = working_directory.trim();
    let fallback = crate::settings::default_chat_working_directory();
    let root = candidate_path(if configured.is_empty() {
        &fallback
    } else {
        configured
    })?;
    fs_canonicalize_existing_or_self(&root)
}

/// Keep existing per-chat files in place; new chats share the configured directory.
pub fn ordinary_workspace_directory(
    working_directory: &str,
    conversation_id: &str,
) -> Result<PathBuf, String> {
    let legacy = conversation_workspace_directory(working_directory, conversation_id)?;
    if legacy.exists() {
        Ok(legacy)
    } else {
        default_working_directory(working_directory)
    }
}

/// Legacy owned directory only: migration and deletion must never use the shared root.
pub fn conversation_workspace_directory(
    working_directory: &str,
    conversation_id: &str,
) -> Result<PathBuf, String> {
    let valid = conversation_id.starts_with("conv_")
        && conversation_id.len() > "conv_".len()
        && conversation_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return Err(format!("Invalid conversation id: {conversation_id}"));
    }
    let root = default_working_directory(working_directory)?;
    Ok(root.join(conversation_id))
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

/// Expands a leading `~` / `~/` / `~\` to the user home directory (shell-style).
pub(crate) fn expand_home_prefix(raw_path: &str) -> Result<String, String> {
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

pub(crate) fn candidate_path(raw_path: &str) -> Result<PathBuf, String> {
    let expanded = expand_home_prefix(raw_path)?;
    if expanded.is_empty() {
        return Err("路径为空，请提供要访问的文件或目录路径。".to_string());
    }

    let path = Path::new(&expanded);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        user_home_dir()?.join(path)
    })
}

pub(crate) fn fs_canonicalize_existing_or_self(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        fs::canonicalize(path).map_err(|err| format!("Resolve path failed: {err}"))
    } else {
        Ok(path.to_path_buf())
    }
}
