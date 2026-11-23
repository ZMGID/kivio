use std::{ffi::OsString, io, path::Path, path::PathBuf};

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
    key: Option<String>,
    email: Option<String>,
    user_id: Option<String>,
    principal_type: Option<String>,
    principal_id: Option<String>,
    team_id: Option<String>,
}

pub(super) fn email(command: &Command, binary: &Path, cwd: &Path) -> Option<String> {
    let home = profile_home(command, binary, cwd)?;
    let login = read_login(&home, super::command_env(command, "GROK_AUTH"))?;
    login
        .email
        .map(|email| email.trim().to_owned())
        .filter(|email| !email.is_empty())
}

pub(super) fn profile_home(command: &Command, binary: &Path, cwd: &Path) -> Option<PathBuf> {
    if super::super::wsl::is_wsl_target(binary) {
        return None;
    }
    let env = |key| super::command_env(command, key);
    if [
        "XAI_API_KEY",
        "GROK_API_KEY",
        "GROK_DEPLOYMENT_KEY",
        "GROK_OIDC_ISSUER",
        "GROK_OIDC_CLIENT_ID",
        "GROK_OAUTH2_ISSUER",
        "GROK_OAUTH2_CLIENT_ID",
        "GROK_OAUTH2_PRINCIPAL_TYPE",
        "GROK_OAUTH2_PRINCIPAL_ID",
        "GROK_AUTH_PROVIDER_COMMAND",
        "GROK_AUTH_PATH",
        "GROK_FORCE_LOGIN_TEAM_ID",
        "GROK_CODE_XAI_API_KEY",
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
    Some(cwd.join(home))
}

pub(super) fn quota_owner(home: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    if super::persisted::managed_preferences("ai.x.grok", &["requirements_toml_base64"]) {
        return None;
    }
    let login = read_login(home, None)?;
    // The subscription scope can change inside the same account when switching teams.
    if login.auth_mode.as_deref() != Some("oidc") {
        return None;
    }
    login.key.as_deref().filter(|key| !key.trim().is_empty())?;
    let user_id = login
        .user_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())?;
    let principal = login
        .principal_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())?;
    let principal_type = login
        .principal_type
        .as_deref()
        .filter(|kind| !kind.trim().is_empty())?;
    super::persisted::fingerprint(&(home, user_id, principal_type, principal, login.team_id))
}

fn read_login(home: &Path, inline: Option<OsString>) -> Option<Login> {
    for path in [
        home.join("config.toml"),
        home.join("managed_config.toml"),
        home.join("requirements.toml"),
        PathBuf::from("/etc/grok/managed_config.toml"),
        PathBuf::from("/etc/grok/requirements.toml"),
    ] {
        match super::persisted::read_small(&path) {
            Ok(config) => {
                let config: toml::Table =
                    toml::from_str(std::str::from_utf8(&config).ok()?).ok()?;
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
    let cache = match inline {
        Some(value) => value.into_string().ok()?.into_bytes(),
        None => super::persisted::read_small(&home.join("auth.json")).ok()?,
    };
    let cache: AuthCache = serde_json::from_slice(&cache).ok()?;
    let login = cache.oidc.or(cache.legacy)?;
    if login.auth_mode.as_deref() == Some("api_key") {
        return None;
    }
    Some(login)
}
