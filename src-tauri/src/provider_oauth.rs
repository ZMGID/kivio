//! Device authorization based on Hermes Agent (MIT) and Kimi CLI (Apache-2.0).
//! See docs/provider-oauth.md and docs/licenses/. No upstream credential files are modified.
use crate::{settings::ModelProvider, state::AppState};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::OnceLock, time::Duration};
use tokio::sync::Mutex;
pub mod account;
pub mod antigravity;
pub mod usage;

const CODEX_CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const KIMI_CLIENT: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub const CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex";
pub const KIMI_BASE: &str = "https://api.kimi.com/coding/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthConfig {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// Runtime-only account project, loaded from the local JSON credential store.
    #[serde(skip)]
    pub project_id: Option<String>,
}

// Deliberately no Debug: these values must never reach logs or the webview.
#[derive(Serialize, Deserialize)]
struct Tokens {
    provider: String,
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
}

#[derive(Clone)]
struct Pending {
    provider: String,
    device_code: String,
    user_code: String,
    expires_at: i64,
    next_poll: i64,
    interval: i64,
    use_system_proxy: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    login_id: String,
    user_code: String,
    verification_url: String,
    interval: i64,
    expires_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollResult {
    status: &'static str,
    interval: i64,
    auth: Option<OAuthConfig>,
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}
fn refresh_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Default::default)
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn client_id(provider: &str) -> Result<&'static str, String> {
    match provider {
        "codex" => Ok(CODEX_CLIENT),
        "kimi" => Ok(KIMI_CLIENT),
        "antigravity" => Ok(antigravity::CLIENT),
        _ => Err("Unsupported OAuth provider".into()),
    }
}
fn token_url(provider: &str) -> &'static str {
    if provider == "antigravity" {
        return antigravity::TOKEN_URL;
    }
    if provider == "codex" {
        "https://auth.openai.com/oauth/token"
    } else {
        "https://auth.kimi.com/api/oauth/token"
    }
}
fn http(use_system_proxy: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("Kivio/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30));
    if !use_system_proxy {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|_| "Cannot create OAuth HTTP client".into())
}
fn common_headers(request: reqwest::RequestBuilder, provider: &str) -> reqwest::RequestBuilder {
    if provider == "kimi" {
        request
            .header("X-Msh-Platform", "kimi_cli")
            .header("X-Msh-Version", "1.0.0")
            .header("X-Msh-Device-Name", "Kivio")
            .header("X-Msh-Device-Model", std::env::consts::ARCH)
            .header("X-Msh-Os-Version", std::env::consts::OS)
            .header("X-Msh-Device-Id", device_id())
    } else {
        request
    }
}
fn device_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let id = uuid::Uuid::new_v4().to_string();
        let Ok(root) = credential_root() else {
            return id;
        };
        let path = root.join("device-id.json");
        if let Ok(data) = std::fs::read(&path) {
            if let Ok(saved) = serde_json::from_slice::<String>(&data) {
                return saved;
            }
        }
        let saved = keyring::Entry::new(crate::app_data::oauth_service(), "device-id")
            .ok()
            .and_then(|entry| entry.get_password().ok())
            .unwrap_or(id);
        if std::fs::create_dir_all(&root).is_ok() {
            if let Ok(json) = serde_json::to_string(&saved) {
                let _ = crate::chat::storage::atomic_write(&path, &json, "OAuth device ID");
            }
        }
        saved
    })
}

/// OAuth credentials are never forwarded through HTTP redirects, including same-host redirects.
pub fn inference_client(use_system_proxy: bool) -> &'static reqwest::Client {
    static PROXY: OnceLock<reqwest::Client> = OnceLock::new();
    static DIRECT: OnceLock<reqwest::Client> = OnceLock::new();
    let slot = if use_system_proxy { &PROXY } else { &DIRECT };
    slot.get_or_init(|| {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(300));
        if !use_system_proxy {
            builder = builder.no_proxy();
        }
        builder.build().expect("OAuth TLS client initialization")
    })
}
async fn response_json(request: reqwest::RequestBuilder) -> Result<(u16, Value), String> {
    let response = request
        .send()
        .await
        .map_err(|_| "OAuth network request failed; check your connection or proxy".to_string())?;
    let status = response.status().as_u16();
    let value = response
        .json()
        .await
        .map_err(|_| format!("OAuth returned an invalid response (HTTP {status})"))?;
    Ok((status, value))
}
fn field(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("OAuth response is missing {key}"))
}
fn number(value: &Value, key: &str, default: i64) -> i64 {
    value[key]
        .as_i64()
        .or_else(|| value[key].as_str()?.parse().ok())
        .unwrap_or(default)
}
fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}
fn tokens_from(value: &Value, provider: &str, old_refresh: &str) -> Result<Tokens, String> {
    let access_token = field(value, "access_token")?;
    let refresh_token = value["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(old_refresh)
        .to_string();
    if refresh_token.is_empty() {
        return Err("OAuth response is missing refresh_token".into());
    }
    let expires_at = claims(&access_token)["exp"]
        .as_i64()
        .unwrap_or_else(|| now() + number(value, "expires_in", 3600).max(1));
    Ok(Tokens {
        provider: provider.into(),
        access_token,
        refresh_token,
        expires_at,
        project_id: None,
    })
}
fn entry(id: &str) -> Result<keyring::Entry, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid OAuth credential reference".to_string())?;
    keyring::Entry::new(crate::app_data::oauth_service(), id)
        .map_err(|_| "System credential store is unavailable".into())
}
#[derive(Serialize, Deserialize)]
struct CredentialManifest {
    version: String,
    chunks: usize,
}
fn chunk_entry(id: &str, version: &str, index: usize) -> Result<keyring::Entry, String> {
    uuid::Uuid::parse_str(version).map_err(|_| "Invalid credential version".to_string())?;
    keyring::Entry::new(
        crate::app_data::oauth_service(),
        &format!("{id}/{version}/{index}"),
    )
    .map_err(|_| "System credential store is unavailable".into())
}
fn manifest(id: &str) -> Result<CredentialManifest, String> {
    let data = entry(id)?.get_secret().map_err(|error| match error {
        keyring::Error::NoEntry => "OAuth session is missing; sign in again".to_string(),
        _ => "Cannot read legacy OAuth credential store".to_string(),
    })?;
    let m: CredentialManifest = serde_json::from_slice(&data)
        .map_err(|_| "Invalid OAuth credential metadata".to_string())?;
    if m.chunks == 0 || m.chunks > 64 {
        return Err("Invalid OAuth credential size".into());
    }
    Ok(m)
}
fn delete_chunks(id: &str, m: &CredentialManifest) {
    for i in 0..m.chunks {
        if let Ok(e) = chunk_entry(id, &m.version, i) {
            let _ = e.delete_credential();
        }
    }
}
fn credential_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Default::default)
}
fn credential_root() -> Result<std::path::PathBuf, String> {
    crate::app_data::app_data_dir()
        .map(|root| root.join("provider-oauth"))
        .ok_or_else(|| "OAuth data directory is unavailable".into())
}
fn credential_path(root: &std::path::Path, id: &str) -> Result<std::path::PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid OAuth credential reference".to_string())?;
    Ok(root.join(format!("{id}.json")))
}
fn save_tokens_in(root: &std::path::Path, id: &str, tokens: &Tokens) -> Result<(), String> {
    use std::io::Write;
    let path = credential_path(root, id)?;
    let data = serde_json::to_vec(tokens).map_err(|_| "Cannot encode OAuth credentials")?;
    std::fs::create_dir_all(root).map_err(|_| "Cannot create OAuth data directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot restrict OAuth directory permissions")?;
    }
    let temporary = root.join(format!(".{id}-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &path)?;
        #[cfg(unix)]
        std::fs::File::open(root)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err("Cannot save OAuth credentials to local JSON".into());
    }
    Ok(())
}
fn save_tokens(id: &str, tokens: &Tokens) -> Result<(), String> {
    let _guard = credential_lock()
        .lock()
        .map_err(|_| "OAuth credential store unavailable")?;
    save_tokens_in(&credential_root()?, id, tokens)
}
fn read_tokens_in(root: &std::path::Path, id: &str) -> Result<Tokens, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            credential_path(root, id)?,
            std::fs::Permissions::from_mode(0o600),
        )
        .map_err(|_| "Cannot restrict OAuth credential permissions")?;
    }
    let data = std::fs::read(credential_path(root, id)?).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "OAuth session is missing; sign in again"
        } else {
            "Cannot read OAuth credential JSON"
        }
    })?;
    serde_json::from_slice(&data).map_err(|_| "OAuth session is invalid; sign in again".into())
}
fn read_legacy_tokens(id: &str) -> Result<(Tokens, CredentialManifest), String> {
    let m = manifest(id)?;
    let mut data = Vec::new();
    for i in 0..m.chunks {
        data.extend(
            chunk_entry(id, &m.version, i)?
                .get_secret()
                .map_err(|_| "OAuth session is incomplete; sign in again")?,
        );
    }
    let tokens =
        serde_json::from_slice(&data).map_err(|_| "OAuth session is invalid; sign in again")?;
    Ok((tokens, m))
}
fn read_or_migrate_tokens_in(
    root: &std::path::Path,
    id: &str,
    legacy: impl FnOnce() -> Result<(Tokens, CredentialManifest), String>,
    cleanup: impl FnOnce(&CredentialManifest),
) -> Result<Tokens, String> {
    let _guard = credential_lock()
        .lock()
        .map_err(|_| "OAuth credential store unavailable")?;
    let path = credential_path(root, id)?;
    match std::fs::metadata(path) {
        Ok(_) => read_tokens_in(root, id),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let (tokens, previous) = legacy()?;
            save_tokens_in(root, id, &tokens)?;
            cleanup(&previous);
            Ok(tokens)
        }
        Err(_) => Err("Cannot access OAuth credential JSON".into()),
    }
}
fn read_tokens(id: &str) -> Result<Tokens, String> {
    read_or_migrate_tokens_in(
        &credential_root()?,
        id,
        || read_legacy_tokens(id),
        |previous| {
            if entry(id)
                .and_then(|entry| {
                    entry
                        .delete_credential()
                        .map_err(|_| "Cannot remove legacy credential".into())
                })
                .is_ok()
            {
                delete_chunks(id, previous);
            }
        },
    )
}

pub(crate) fn export_credentials(settings: &crate::settings::Settings) -> Result<Value, String> {
    let mut entries = serde_json::Map::new();
    for provider in &settings.providers {
        if let Some(id) = provider
            .request
            .oauth
            .as_ref()
            .and_then(|auth| auth.credential_id.as_ref())
        {
            let tokens = match read_tokens(id) {
                Ok(tokens) => tokens,
                Err(error) if error == "OAuth session is missing; sign in again" => continue,
                Err(error) => return Err(error),
            };
            entries.insert(
                id.clone(),
                serde_json::to_value(tokens).map_err(|_| "Cannot encode OAuth backup")?,
            );
        }
    }
    Ok(Value::Object(entries))
}
/// Import under fresh IDs: a failed settings commit cannot replace an existing account's token.
pub(crate) fn import_credentials(
    settings: &mut crate::settings::Settings,
    value: &Value,
) -> Result<Vec<String>, String> {
    import_credentials_in(&credential_root()?, settings, value)
}
fn import_credentials_in(
    root: &std::path::Path,
    settings: &mut crate::settings::Settings,
    value: &Value,
) -> Result<Vec<String>, String> {
    let _guard = credential_lock()
        .lock()
        .map_err(|_| "OAuth credential store unavailable")?;
    let entries: HashMap<String, Tokens> =
        serde_json::from_value(value.clone()).map_err(|_| "Invalid OAuth backup credentials")?;
    let mut imported = Vec::new();
    let mut remapped: HashMap<String, String> = HashMap::new();
    let result = (|| {
        for provider in &mut settings.providers {
            let Some(auth) = &mut provider.request.oauth else {
                continue;
            };
            let Some(old_id) = &auth.credential_id else {
                continue;
            };
            let Some(tokens) = entries.get(old_id) else {
                auth.credential_id = None;
                continue;
            };
            if tokens.provider != auth.provider {
                return Err("OAuth backup provider mismatch".into());
            }
            if let Some(id) = remapped.get(old_id) {
                auth.credential_id = Some(id.clone());
                continue;
            }
            let id = uuid::Uuid::new_v4().to_string();
            save_tokens_in(root, &id, tokens)?;
            remapped.insert(old_id.clone(), id.clone());
            imported.push(id.clone());
            auth.credential_id = Some(id);
        }
        Ok(())
    })();
    if let Err(error) = result {
        discard_credentials_in(root, &imported);
        return Err(error);
    }
    Ok(imported)
}
fn discard_credentials_in(root: &std::path::Path, ids: &[String]) {
    for id in ids {
        if let Ok(path) = credential_path(root, id) {
            let _ = std::fs::remove_file(path);
        }
    }
}
pub(crate) fn discard_imported_credentials(ids: &[String]) {
    if let Ok(root) = credential_root() {
        discard_credentials_in(&root, ids);
    }
}

#[tauri::command]
pub async fn provider_oauth_start(
    provider: String,
    use_system_proxy: bool,
) -> Result<Login, String> {
    if provider == "antigravity" {
        return antigravity::start(use_system_proxy).await;
    }
    let client_id = client_id(&provider)?;
    let client = http(use_system_proxy)?;
    let request = if provider == "codex" {
        client
            .post("https://auth.openai.com/api/accounts/deviceauth/usercode")
            .json(&json!({"client_id": client_id}))
    } else {
        common_headers(
            client
                .post("https://auth.kimi.com/api/oauth/device_authorization")
                .form(&[("client_id", client_id)]),
            &provider,
        )
    };
    let (status, value) = response_json(request).await?;
    if status != 200 {
        return Err(format!(
            "Cannot start OAuth login (HTTP {status}); try again later"
        ));
    }
    let user_code = field(&value, "user_code")?;
    let device_code = field(
        &value,
        if provider == "codex" {
            "device_auth_id"
        } else {
            "device_code"
        },
    )?;
    let verification_url = if provider == "codex" {
        "https://auth.openai.com/codex/device".into()
    } else {
        field(&value, "verification_uri_complete").or_else(|_| field(&value, "verification_uri"))?
    };
    let url =
        url::Url::parse(&verification_url).map_err(|_| "Invalid verification URL".to_string())?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("auth.openai.com" | "auth.kimi.com" | "www.kimi.com" | "kimi.com")
        )
    {
        return Err("Unrecognized OAuth verification URL".into());
    }
    let interval = number(&value, "interval", 5).clamp(3, 60);
    let expires_at = now() + number(&value, "expires_in", 900).clamp(1, 900);
    let login_id = uuid::Uuid::new_v4().to_string();
    let mut sessions = pending().lock().await;
    sessions.retain(|_, p| p.expires_at > now());
    if sessions.len() >= 8 {
        return Err("Too many pending logins; cancel an existing login first".into());
    }
    sessions.insert(
        login_id.clone(),
        Pending {
            provider,
            device_code,
            user_code: user_code.clone(),
            expires_at,
            next_poll: now() + interval,
            interval,
            use_system_proxy,
        },
    );
    Ok(Login {
        login_id,
        user_code,
        verification_url,
        interval,
        expires_at,
    })
}

#[tauri::command]
pub async fn provider_oauth_poll(login_id: String) -> Result<PollResult, String> {
    if let Some(result) = antigravity::poll(&login_id).await {
        return result;
    }
    let p = {
        let mut sessions = pending().lock().await;
        let p = sessions
            .get_mut(&login_id)
            .ok_or("Login was cancelled or expired")?;
        if p.expires_at <= now() {
            sessions.remove(&login_id);
            return Err("Login expired; start again".into());
        }
        if p.next_poll > now() {
            return Ok(PollResult {
                status: "pending",
                interval: p.next_poll - now(),
                auth: None,
            });
        }
        p.next_poll = now() + 60; // reserve this poll against concurrent invocations
        p.clone()
    };
    let client = http(p.use_system_proxy)?;
    let req = if p.provider == "codex" {
        client
            .post("https://auth.openai.com/api/accounts/deviceauth/token")
            .json(&json!({"device_auth_id": p.device_code, "user_code": p.user_code}))
    } else {
        common_headers(
            client.post(token_url(&p.provider)).form(&[
                ("client_id", KIMI_CLIENT),
                ("device_code", &p.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]),
            &p.provider,
        )
    };
    let (status, mut value) = response_json(req).await?;
    let waiting = (p.provider == "codex" && matches!(status, 403 | 404))
        || (p.provider == "kimi"
            && matches!(
                value["error"].as_str(),
                Some("authorization_pending" | "slow_down")
            ))
        || status == 429;
    if waiting {
        let interval = if status == 429 || value["error"] == "slow_down" {
            (p.interval + 5).min(60)
        } else {
            p.interval
        };
        if let Some(p) = pending().lock().await.get_mut(&login_id) {
            p.interval = interval;
            p.next_poll = now() + interval;
        }
        return Ok(PollResult {
            status: "pending",
            interval,
            auth: None,
        });
    }
    if status != 200 {
        pending().lock().await.remove(&login_id);
        return Err(format!(
            "OAuth authorization failed (HTTP {status}); start again"
        ));
    }
    if p.provider == "codex" {
        let code = field(&value, "authorization_code")?;
        let verifier = field(&value, "code_verifier")?;
        let (status, token_value) = response_json(client.post(token_url("codex")).form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CODEX_CLIENT),
            ("code", &code),
            ("code_verifier", &verifier),
            (
                "redirect_uri",
                "https://auth.openai.com/deviceauth/callback",
            ),
        ]))
        .await?;
        if status != 200 {
            pending().lock().await.remove(&login_id);
            return Err(format!(
                "OAuth token exchange failed (HTTP {status}); start again"
            ));
        }
        value = token_value;
    }
    let tokens = tokens_from(&value, &p.provider, "")?;
    let mut sessions = pending().lock().await;
    if !sessions.contains_key(&login_id) {
        return Err("Login cancelled".into());
    }
    save_tokens(&login_id, &tokens)?;
    sessions.remove(&login_id);
    Ok(PollResult {
        status: "authorized",
        interval: 0,
        auth: Some(OAuthConfig {
            provider: p.provider,
            credential_id: Some(login_id),
            project_id: None,
        }),
    })
}

#[tauri::command]
pub async fn provider_oauth_cancel(login_id: String) {
    antigravity::cancel(&login_id).await;
    pending().lock().await.remove(&login_id);
}

#[tauri::command]
pub async fn provider_oauth_disconnect(credential_id: String) -> Result<(), String> {
    let _guard = refresh_lock().lock().await;
    let _file_guard = credential_lock()
        .lock()
        .map_err(|_| "OAuth credential store unavailable")?;
    let root = credential_root()?;
    let path = credential_path(&root, &credential_id)?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Cannot remove OAuth credential JSON".into()),
    }
    // Best-effort cleanup of a pre-migration entry; JSON-only installations need no keyring.
    if let Ok(previous) = manifest(&credential_id) {
        entry(&credential_id)?
            .delete_credential()
            .map_err(|_| "Cannot remove legacy OAuth session")?;
        delete_chunks(&credential_id, &previous);
    }
    Ok(())
}

pub fn validate_provider(provider: &ModelProvider) -> Result<(), String> {
    let Some(auth) = &provider.request.oauth else {
        return Ok(());
    };
    let (base, format) = match auth.provider.as_str() {
        "codex" => (CODEX_BASE, "openai_responses"),
        "kimi" => (KIMI_BASE, "openai_chat"),
        "antigravity" => (antigravity::BASE, "gemini"),
        _ => return Err("Unsupported OAuth provider".into()),
    };
    if provider.base_url.trim_end_matches('/') != base || provider.api_format != format {
        return Err("OAuth requires the provider's original endpoint and protocol; restore them or switch to API Key".into());
    }
    Ok(())
}

/// Resolve only into an ephemeral clone, after validating the destination. Never save this clone.
pub async fn resolve_provider(
    state: &AppState,
    provider: &ModelProvider,
) -> Result<ModelProvider, String> {
    let mut resolved = provider.clone();
    let Some(auth) = &provider.request.oauth else {
        return Ok(resolved);
    };
    validate_provider(provider)?;
    let id = auth
        .credential_id
        .as_deref()
        .ok_or("Sign in to the model provider first")?;
    let _guard = refresh_lock().lock().await;
    let mut tokens = read_tokens(id)?;
    if tokens.provider != auth.provider {
        return Err("OAuth credential belongs to another provider".into());
    }
    if tokens.expires_at <= now() + 120 {
        let client = http(provider.request.use_system_proxy)?;
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id(&auth.provider)?),
            ("refresh_token", &tokens.refresh_token),
        ];
        if auth.provider == "antigravity" {
            form.push(("client_secret", antigravity::CLIENT_SECRET));
        }
        let request = common_headers(
            client.post(token_url(&auth.provider)).form(&form),
            &auth.provider,
        );
        let (status, value) = response_json(request).await?;
        if status != 200 {
            return Err(format!(
                "OAuth refresh failed (HTTP {status}); sign in again if the session was revoked"
            ));
        }
        let project_id = tokens.project_id.clone();
        tokens = tokens_from(&value, &auth.provider, &tokens.refresh_token)?;
        tokens.project_id = project_id;
        save_tokens(id, &tokens)?;
    }
    if auth.provider == "antigravity" && tokens.project_id.is_none() {
        return Err("Antigravity account project is missing; sign in again".into());
    }
    if let Some(auth) = resolved.request.oauth.as_mut() {
        auth.project_id = tokens.project_id;
    }
    resolved.api_keys = vec![tokens.access_token];
    resolved.active_key_index = 0;
    // This argument keeps parity with normal providers.
    let _ = state;
    Ok(resolved)
}

pub fn is_codex(provider: &ModelProvider) -> bool {
    provider
        .request
        .oauth
        .as_ref()
        .is_some_and(|a| a.provider == "codex")
}

pub fn header_pairs(provider: &ModelProvider) -> Vec<(String, String)> {
    let Some(auth) = &provider.request.oauth else {
        return vec![];
    };
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
    if is_codex(provider) {
        let mut pairs = vec![
            pair("originator", "kivio"),
            pair("User-Agent", concat!("Kivio/", env!("CARGO_PKG_VERSION"))),
            pair("OpenAI-Beta", "responses=experimental"),
        ];
        if let Some(token) = provider.preferred_api_key() {
            if let Some(id) =
                claims(token)["https://api.openai.com/auth"]["chatgpt_account_id"].as_str()
            {
                pairs.push(pair("ChatGPT-Account-Id", id));
            }
        }
        pairs
    } else if auth.provider == "antigravity" {
        let mut pairs = vec![pair("User-Agent", &antigravity::user_agent())];
        if let Some(token) = provider.preferred_api_key() {
            pairs.push(pair("Authorization", &format!("Bearer {token}")));
        }
        pairs
    } else if auth.provider == "kimi" {
        vec![
            pair("User-Agent", "KimiCLI/1.0.0"),
            pair("X-Msh-Platform", "kimi_cli"),
            pair("X-Msh-Version", "1.0.0"),
            pair("X-Msh-Device-Name", "Kivio"),
            pair("X-Msh-Device-Model", std::env::consts::ARCH),
            pair("X-Msh-Os-Version", std::env::consts::OS),
            pair("X-Msh-Device-Id", device_id()),
        ]
    } else {
        vec![]
    }
}

pub fn apply_headers(
    mut request: reqwest::RequestBuilder,
    provider: &ModelProvider,
) -> reqwest::RequestBuilder {
    for (name, value) in crate::provider_request::header_pairs(provider, None) {
        request = request.header(name, value);
    }
    request
}

pub fn codex_body(body: &mut Value) {
    body["stream"] = json!(true);
    body["store"] = json!(false);
    if !body["instructions"].is_string() {
        body["instructions"] = json!("");
    }
    if let Some(object) = body.as_object_mut() {
        for key in [
            "max_output_tokens",
            "temperature",
            "top_p",
            "prompt_cache_retention",
            "previous_response_id",
        ] {
            object.remove(key);
        }
    }
}

pub async fn models(state: &AppState, provider: &ModelProvider) -> Result<Vec<String>, String> {
    let provider = resolve_provider(state, provider).await?;
    if antigravity::is_provider(&provider) {
        return antigravity::models(&provider).await;
    }
    let client = http(provider.request.use_system_proxy)?;
    let url = if is_codex(&provider) {
        format!("{CODEX_BASE}/models?client_version=1.0.0")
    } else {
        format!("{KIMI_BASE}/models")
    };
    let response = apply_headers(
        client.get(url).bearer_auth(
            provider
                .preferred_api_key()
                .ok_or("Missing OAuth session")?,
        ),
        &provider,
    )
    .send()
    .await
    .map_err(|_| "OAuth model discovery network error".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "OAuth model discovery failed (HTTP {})",
            response.status()
        ));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "Invalid model list".to_string())?;
    parse_models(&value)
}

fn parse_models(value: &Value) -> Result<Vec<String>, String> {
    let rows = value["models"]
        .as_array()
        .or_else(|| value["data"].as_array())
        .ok_or("Invalid model list")?;
    let mut models: Vec<String> = rows
        .iter()
        .filter(|m| m["visibility"].as_str() != Some("hide"))
        .filter_map(|m| m["slug"].as_str().or_else(|| m["id"].as_str()))
        .map(str::to_owned)
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

pub async fn test_connection(
    state: &AppState,
    provider: &ModelProvider,
    model: Option<&str>,
) -> Result<(), String> {
    use crate::chat::model::*;
    let Some(model) = model else {
        models(state, provider).await?;
        return Ok(());
    };
    let request = GenerateRequest {
        model: model.to_string(),
        system: String::new(),
        messages: vec![ModelMessage::text(
            ModelRole::User,
            "Reply with exactly: kivio-ok",
        )],
        tools: vec![],
        options: GenerateOptions {
            max_tokens: 64,
            ..Default::default()
        },
        metadata: RequestMetadata {
            label: "OAuth connection test".into(),
            ..Default::default()
        },
    };
    generate_with_chat_provider(state, provider, 1, request)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider() -> ModelProvider {
        serde_json::from_value(json!({"id":"p", "name":"Codex", "baseUrl":CODEX_BASE,
            "apiFormat":"openai_responses", "request":{"oauth":{"provider":"codex","credentialId":uuid::Uuid::new_v4().to_string()}}})).unwrap()
    }
    #[test]
    fn oauth_destination_and_protocol_are_bound_before_credentials_are_read() {
        let mut p = provider();
        assert!(validate_provider(&p).is_ok());
        assert!(p.authentication_ready());
        for base in [
            "http://chatgpt.com/backend-api/codex",
            "https://chatgpt.com.evil.test/backend-api/codex",
            "https://chatgpt.com/backend-api/codex?redirect=evil",
            KIMI_BASE,
        ] {
            p.base_url = base.into();
            assert!(validate_provider(&p).is_err());
        }
        p.base_url = CODEX_BASE.into();
        p.api_format = "openai_chat".into();
        assert!(validate_provider(&p).is_err());
        p.request.oauth.as_mut().unwrap().credential_id = None;
        p.api_keys = vec!["old-api-key".into()];
        assert!(!p.authentication_ready());
    }
    #[test]
    fn codex_account_headers_override_custom_headers_without_duplicates() {
        let mut p = provider();
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(
                &json!({"https://api.openai.com/auth":{"chatgpt_account_id":"account-123"}}),
            )
            .unwrap(),
        );
        p.api_keys = vec![format!("header.{payload}.signature")];
        p.request
            .custom_headers
            .push(crate::settings::ProviderCustomHeader {
                key: "Originator".into(),
                value: "other".into(),
            });
        let pairs = crate::provider_request::header_pairs(&p, None);
        assert_eq!(
            pairs
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("originator"))
                .count(),
            1
        );
        assert!(pairs.contains(&("originator".into(), "kivio".into())));
        assert!(pairs.contains(&("ChatGPT-Account-Id".into(), "account-123".into())));

        // 同时选了 Codex 身份：OAuth 的 originator / UA 覆盖身份预设，配套的 version 不留。
        p.request.cli_identity = "codex".into();
        let pairs = crate::provider_request::header_pairs(&p, None);
        assert!(pairs.contains(&("originator".into(), "kivio".into())));
        assert!(!pairs.iter().any(|(k, _)| k.eq_ignore_ascii_case("version")));
        assert!(pairs
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.starts_with("Kivio/")));
    }
    #[test]
    fn json_credentials_migrate_durably_and_preserve_legacy_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("credentials");
        let id = uuid::Uuid::new_v4().to_string();
        let make_legacy = || {
            Ok((
                Tokens {
                    provider: "codex".into(),
                    access_token: "synthetic-access".repeat(400),
                    refresh_token: "synthetic-refresh".into(),
                    expires_at: 42,
                    project_id: None,
                },
                CredentialManifest {
                    version: uuid::Uuid::new_v4().to_string(),
                    chunks: 2,
                },
            ))
        };
        let deleted = std::sync::atomic::AtomicBool::new(false);
        let blocked = directory.path().join("blocked");
        assert!(read_or_migrate_tokens_in(
            &blocked,
            &id,
            || {
                // Fail the replacement write only after the legacy authority was retrieved.
                std::fs::write(&blocked, "original").unwrap();
                make_legacy()
            },
            |_| {
                deleted.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        )
        .is_err());
        assert!(!deleted.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "original");
        let tokens = read_or_migrate_tokens_in(&root, &id, make_legacy, |_| {
            // The native entry must survive until the complete replacement is independently readable.
            assert_eq!(
                read_tokens_in(&root, &id).unwrap().refresh_token,
                "synthetic-refresh"
            );
            deleted.store(true, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap();
        assert!(deleted.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(tokens.access_token, "synthetic-access".repeat(400));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(credential_path(&root, &id).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let mut updated = tokens;
        updated.refresh_token = "rotated".into();
        save_tokens_in(&root, &id, &updated).unwrap();
        assert_eq!(
            read_or_migrate_tokens_in(
                &root,
                &id,
                || Err("legacy has already been deleted".into()),
                |_| panic!("unexpected cleanup")
            )
            .unwrap()
            .refresh_token,
            "rotated"
        );
        std::fs::write(credential_path(&root, &id).unwrap(), "invalid-json").unwrap();
        assert!(
            read_or_migrate_tokens_in(&root, &id, make_legacy, |_| panic!(
                "revived stale credentials"
            ))
            .is_err()
        );
    }
    #[test]
    fn oauth_backup_preserves_shared_credential_rotation_without_overwriting_existing_accounts() {
        let directory = tempfile::tempdir().unwrap();
        let first = provider();
        let mut second = first.clone();
        second.id = "second-provider".into();
        let old_id = first
            .request
            .oauth
            .as_ref()
            .unwrap()
            .credential_id
            .as_ref()
            .unwrap()
            .clone();
        let tokens = Tokens {
            provider: "codex".into(),
            access_token: "synthetic-access".into(),
            refresh_token: "synthetic-refresh".into(),
            expires_at: 42,
            project_id: None,
        };
        save_tokens_in(directory.path(), &old_id, &tokens).unwrap();
        let backup = json!({ old_id.clone(): serde_json::to_value(&tokens).unwrap() });
        let mut settings = crate::settings::Settings::default();
        settings.providers = vec![first, second];
        let imported = import_credentials_in(directory.path(), &mut settings, &backup).unwrap();
        let first_id = settings.providers[0]
            .request
            .oauth
            .as_ref()
            .unwrap()
            .credential_id
            .as_ref()
            .unwrap();
        let second_id = settings.providers[1]
            .request
            .oauth
            .as_ref()
            .unwrap()
            .credential_id
            .as_ref()
            .unwrap();
        assert_eq!(
            first_id, second_id,
            "One account must have one refresh authority after restore"
        );
        assert_ne!(first_id, &old_id);
        assert_eq!(imported.len(), 1);
        let mut restored = read_tokens_in(directory.path(), first_id).unwrap();
        restored.refresh_token = "rotated-restored-refresh".into();
        save_tokens_in(directory.path(), first_id, &restored).unwrap();
        assert_eq!(
            read_tokens_in(directory.path(), second_id)
                .unwrap()
                .refresh_token,
            "rotated-restored-refresh"
        );
        assert_eq!(
            read_tokens_in(directory.path(), &old_id)
                .unwrap()
                .refresh_token,
            "synthetic-refresh"
        );
    }
    #[test]
    fn concurrent_json_migration_reads_one_legacy_authority() {
        let directory = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let reads = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let tokens = read_or_migrate_tokens_in(
                        directory.path(),
                        &id,
                        || {
                            reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            Ok((
                                Tokens {
                                    provider: "kimi".into(),
                                    access_token: "synthetic".into(),
                                    refresh_token: "synthetic-refresh".into(),
                                    expires_at: 42,
                                    project_id: None,
                                },
                                CredentialManifest {
                                    version: uuid::Uuid::new_v4().to_string(),
                                    chunks: 1,
                                },
                            ))
                        },
                        |_| {},
                    )
                    .unwrap();
                    assert_eq!(tokens.refresh_token, "synthetic-refresh");
                });
            }
        });
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    #[test]
    fn oauth_tokens_require_refresh_and_preserve_rotation() {
        assert!(tokens_from(&json!({"access_token":"x"}), "kimi", "").is_err());
        let t = tokens_from(
            &json!({"access_token":"x", "expires_in":600}),
            "kimi",
            "old",
        )
        .unwrap();
        assert_eq!(t.refresh_token, "old");
        let t = tokens_from(
            &json!({"access_token":"x", "refresh_token":"new"}),
            "kimi",
            "old",
        )
        .unwrap();
        assert_eq!(t.refresh_token, "new");
    }
    #[test]
    fn codex_requires_stream_without_unsupported_fields() {
        let mut body = json!({"max_output_tokens":64,"temperature":0.5,"input":[],"tools":[{"type":"function"}]});
        codex_body(&mut body);
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["instructions"], "");
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("temperature").is_none());
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn discovery_accepts_both_provider_shapes() {
        assert_eq!(
            parse_models(&json!({"models":[{"slug":"a"},{"slug":"hidden","visibility":"hide"}]}))
                .unwrap(),
            vec!["a"]
        );
        assert_eq!(
            parse_models(&json!({"data":[{"id":"kimi"}]})).unwrap(),
            vec!["kimi"]
        );
        assert!(parse_models(&json!({"error":"unauthorized"})).is_err());
    }
}
