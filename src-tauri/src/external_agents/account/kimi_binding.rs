use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use super::{
    command_env, explicit_command_env,
    persisted::{fingerprint, read_small},
};

pub(super) fn profile_home(command: &Command) -> Option<PathBuf> {
    let env =
        |key: &str| explicit_command_env(command, key).unwrap_or_else(|| std::env::var_os(key));
    if [
        "KIMI_CODE_BASE_URL",
        "KIMI_CODE_OAUTH_HOST",
        "KIMI_OAUTH_HOST",
        "KIMI_CODE_CUSTOM_HEADERS",
    ]
    .into_iter()
    .any(|key| env(key).is_some())
    {
        return None;
    }
    let home = env("KIMI_CODE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            command_env(command, if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .or_else(std::env::home_dir)
                .map(|home| home.join(".kimi-code"))
        })?;
    home.is_absolute().then_some(home)
}

pub(super) fn quota_owner(home: &Path) -> Option<String> {
    let config = read_small(&home.join("config.toml")).ok()?;
    let config: toml::Table = toml::from_str(std::str::from_utf8(&config).ok()?).ok()?;
    let provider = config
        .get("providers")?
        .get("managed:kimi-code")?
        .as_table()?;
    let oauth = provider.get("oauth")?.as_table()?;
    // Respect only the credential slots created by the CLI's own managed login.
    if provider.get("type")?.as_str()? != "kimi"
        || oauth.get("storage")?.as_str()? != "file"
        || [
            "baseUrl",
            "api_key_env",
            "apiKey",
            "apiKeyEnv",
            "custom_headers",
            "customHeaders",
        ]
        .iter()
        .any(|key| provider.contains_key(*key))
        || oauth.contains_key("oauthHost")
        || provider
            .get("api_key")
            .is_some_and(|key| key.as_str() != Some(""))
    {
        return None;
    }
    let base_url = provider.get("base_url")?.as_str()?;
    let host = oauth
        .get("oauth_host")
        .map(toml::Value::as_str)
        .unwrap_or(Some("https://auth.kimi.com"))?;
    let credentials = home.join("credentials");
    let key = match (base_url, host) {
        ("https://api.kimi.com/coding/v1", "https://auth.kimi.com") => "kimi-code".to_owned(),
        ("https://api.kimi.ai/coding/v1", "https://auth.kimi.ai") => {
            // The CLI scopes a non-default login slot to this exact host and base URL.
            // If parsing falls back to defaults, a leftover mainland token is ambiguous.
            if !fs::symlink_metadata(credentials.join("kimi-code.json"))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                return None;
            }
            let pair = format!(r#"{{"oauthHost":"{host}","baseUrl":"{base_url}"}}"#);
            let digest = format!("{:x}", Sha256::digest(pair));
            format!("kimi-code-env-{}", &digest[..16])
        }
        _ => return None,
    };
    if oauth.get("key")?.as_str()? != format!("oauth/{key}") {
        return None;
    }
    let token: Value =
        serde_json::from_slice(&read_small(&credentials.join(format!("{key}.json"))).ok()?).ok()?;
    let access = super::display_text(&token["access_token"])?;
    let refresh = super::display_text(&token["refresh_token"])?;
    // Kimi's file has no stable account id. A different or rotated credential must
    // establish a fresh reading before it can reuse the disk cache.
    fingerprint(&(home, base_url, key, access, refresh))
}
