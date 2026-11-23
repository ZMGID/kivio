use std::{ffi::OsString, fs, io, path::Path, path::PathBuf};

use serde::Deserialize;
use tokio::process::Command;

// Grok's official OAuth scopes, also used by the T3 Code adapter. Do not select
// an arbitrary entry from the CLI's cache, which can contain other deployments.
#[derive(Deserialize)]
struct AuthCache {
    #[serde(rename = "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828")]
    oidc: Option<Login>,
    #[serde(rename = "https://accounts.x.ai/sign-in")]
    legacy: Option<Login>,
}

#[derive(Deserialize)]
struct Login {
    auth_mode: Option<String>,
    email: Option<String>,
}

pub(super) fn email(command: &Command, binary: &Path, cwd: &Path) -> Option<String> {
    if super::super::wsl::is_wsl_target(binary) {
        return None;
    }
    let env = |key: &str| -> Option<OsString> {
        command
            .as_std()
            .get_envs()
            .find(|(name, _)| {
                if cfg!(windows) {
                    name.to_string_lossy().eq_ignore_ascii_case(key)
                } else {
                    *name == key
                }
            })
            .map_or_else(|| std::env::var_os(key), |(_, value)| value.map(Into::into))
            .filter(|value| !value.to_string_lossy().trim().is_empty())
    };
    if [
        "XAI_API_KEY",
        "GROK_API_KEY",
        "GROK_OIDC_ISSUER",
        "GROK_OIDC_CLIENT_ID",
        "GROK_OAUTH2_ISSUER",
        "GROK_OAUTH2_CLIENT_ID",
        "GROK_OAUTH2_PRINCIPAL_TYPE",
        "GROK_OAUTH2_PRINCIPAL_ID",
        "GROK_AUTH_PROVIDER_COMMAND",
        "GROK_LOCAL_AUTH",
        "GROK_CLI_CHAT_PROXY_BASE_URL",
        "GROK_MODELS_BASE_URL",
        "GROK_CONFIG",
        "GROK_CONFIG_PATH",
    ]
    .iter()
    .any(|key| env(key).is_some())
    {
        return None;
    }
    let home = env("GROK_HOME").map(PathBuf::from).or_else(|| {
        env(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .or_else(std::env::home_dir)
            .map(|home| home.join(".grok"))
    })?;
    let home = cwd.join(home);
    for path in [
        home.join("config.toml"),
        home.join("managed_config.toml"),
        home.join("requirements.toml"),
        PathBuf::from("/etc/grok/managed_config.toml"),
        PathBuf::from("/etc/grok/requirements.toml"),
    ] {
        match fs::read_to_string(path) {
            Ok(config) => {
                let config: toml::Table = toml::from_str(&config).ok()?;
                if ["auth", "grok_com_config", "endpoints"]
                    .iter()
                    .any(|key| config.contains_key(*key))
                {
                    return None;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    let cache = match env("GROK_AUTH") {
        Some(value) => value.into_string().ok()?,
        None => fs::read_to_string(home.join("auth.json")).ok()?,
    };
    let cache: AuthCache = serde_json::from_str(&cache).ok()?;
    let login = cache.oidc.or(cache.legacy)?;
    if login.auth_mode.as_deref() == Some("api_key") {
        return None;
    }
    login
        .email
        .map(|email| email.trim().to_owned())
        .filter(|email| !email.is_empty())
}
