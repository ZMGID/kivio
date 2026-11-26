//! Public-client Sign in with ChatGPT. Protocol: developers.openai.com/siwc/token-sharing-open-source.
use super::*;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use tokio::{net::TcpListener, task::JoinHandle};

const ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
const DYNAMIC: &str = "dynamic_agent_client";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const DIRECT: &str = "chatgpt.tokens.use.direct";
pub(super) const CREDENTIAL_PREFIX: &str = "chatgpt-";
pub(super) fn new_credential_id() -> String {
    format!("{CREDENTIAL_PREFIX}{}", uuid::Uuid::new_v4())
}

pub(super) fn saved_session(root: &Path, id: &str) -> Result<Tokens, String> {
    if !id.starts_with(CREDENTIAL_PREFIX) {
        return Err("Select a saved ChatGPT account or sign in again".into());
    }
    let tokens = read_tokens_in(root, id)?;
    if tokens.provider != "chatgpt" {
        return Err("OAuth credential belongs to another provider".into());
    }
    Ok(tokens)
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub(super) struct Registration {
    pub(super) subject: Option<String>,
    #[serde(default)]
    revision: Option<uuid::Uuid>,
    pub(super) email: Option<String>,
    pub(super) name: Option<String>,
    client_id: String,
    id_token: Option<String>,
    scopes: Vec<String>,
    earliest_refresh_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_refresh: Option<PendingRefresh>,
}

impl Registration {
    fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == DIRECT)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedAccount {
    credential_id: String,
    client_id: String,
    subject: Option<String>,
    email: Option<String>,
    connected: bool,
    plan_enabled: bool,
}

#[tauri::command]
pub fn provider_oauth_chatgpt_accounts() -> Result<Vec<SavedAccount>, String> {
    saved_accounts(&credential_root()?)
}

fn saved_accounts(root: &Path) -> Result<Vec<SavedAccount>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("Cannot read saved ChatGPT accounts".into()),
    };
    let mut accounts = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|_| "Cannot list saved ChatGPT accounts")?
            .path();
        let Some(id) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        if path.extension().and_then(|v| v.to_str()) != Some("json")
            || !id.starts_with(CREDENTIAL_PREFIX)
        {
            continue;
        }
        let Ok(tokens) = saved_session(root, id) else {
            continue;
        };
        let Some(reg) = tokens.chatgpt else { continue };
        accounts.push(SavedAccount {
            credential_id: id.into(),
            plan_enabled: reg.plan_enabled(),
            connected: reg.id_token.is_some(),
            client_id: reg.client_id,
            subject: reg.subject,
            email: reg.email,
        });
    }
    accounts.sort_by(|a, b| {
        (&a.email, &a.client_id, &a.credential_id).cmp(&(&b.email, &b.client_id, &b.credential_id))
    });
    Ok(accounts)
}

// A desktop and headless process can share this directory. Lock across read, rotation and
// replacement so neither process can reuse the other's already-rotated refresh token.
pub(super) async fn store_lock() -> Result<fs::File, String> {
    let root = credential_root()?;
    tokio::task::spawn_blocking(move || -> std::io::Result<_> {
        fs::create_dir_all(&root)?;
        let file = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(".chatgpt.lock"))?;
        file.lock()?;
        Ok(file)
    })
    .await
    .map_err(|_| "ChatGPT credential lock interrupted")?
    .map_err(|_| "Cannot lock the ChatGPT credential store".into())
}

fn host_id(root: &Path) -> Result<String, String> {
    let path = root.join("chatgpt-host-id");
    match fs::read_to_string(&path) {
        Ok(id)
            if id
                .strip_prefix("urn:uuid:")
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .is_some() =>
        {
            Ok(id)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
            crate::chat::storage::atomic_write(&path, &id, "ChatGPT host ID")?;
            Ok(id)
        }
        _ => Err("Cannot read this host's saved ChatGPT identifier".into()),
    }
}

struct Session {
    task: JoinHandle<Result<OAuthConfig, String>>,
    expires: i64,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}

struct Attempt {
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: url::Url,
    client_id: String,
}

impl Attempt {
    async fn open(
        app: &tauri::AppHandle,
        host: &str,
        registration: Option<&Registration>,
        client_id: String,
    ) -> Result<(Self, TcpListener), String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "Cannot listen for ChatGPT sign-in on 127.0.0.1")?;
        let addr = listener
            .local_addr()
            .map_err(|_| "Cannot determine ChatGPT callback port")?;
        let attempt = Attempt {
            state: random_secret(),
            nonce: random_secret(),
            verifier: random_secret(),
            redirect_uri: url::Url::parse(&format!("http://{addr}/auth/callback"))
                .expect("loopback URL"),
            client_id,
        };
        crate::commands::open_external(
            app.clone(),
            attempt.authorization_url(host, registration).into(),
        )
        .map_err(|_| "Could not open the browser for ChatGPT sign-in")?;
        Ok((attempt, listener))
    }

    fn authorization_url(&self, host: &str, registration: Option<&Registration>) -> url::Url {
        let mut url = url::Url::parse(AUTHORIZE).expect("static authorize URL");
        let mut query = url.query_pairs_mut();
        query.extend_pairs([
            ("ext_agent_host_id", host),
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", SCOPES),
            ("resource", CHATGPT_BASE),
            ("state", self.state.as_str()),
            ("nonce", self.nonce.as_str()),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes())),
            ),
        ]);
        if let Some(reg) = registration {
            if let Some(email) = &reg.email {
                query.append_pair("login_hint", email);
            }
            if reg.id_token.is_some() && !reg.plan_enabled() {
                query.append_pair("prompt", "consent");
            }
        }
        if self.client_id == DYNAMIC {
            query.append_pair("agent_name_hint", "Kivio");
        }
        drop(query);
        url
    }

    // Invalid requests keep the listener open. A state-bound OAuth failure terminates this attempt.
    fn callback(&self, request: &str) -> Result<Result<(String, String), String>, &'static str> {
        let mut lines = request.lines();
        let mut line = lines.next().unwrap_or_default().split_whitespace();
        let expected_host = &self.redirect_uri[url::Position::BeforeHost..url::Position::AfterPort];
        let mut hosts = lines
            .take_while(|line| !line.is_empty())
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("host").then_some(value.trim())
            });
        if hosts.next() != Some(expected_host) || hosts.next().is_some() {
            return Err("Invalid callback host");
        }
        if line.next() != Some("GET") {
            return Err("Invalid callback method");
        }
        let query = line
            .next()
            .and_then(|s| s.strip_prefix("/auth/callback?"))
            .ok_or("Invalid callback path")?;
        let mut params = HashMap::new();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if params.insert(key, value).is_some() {
                return Err("Duplicate callback parameter");
            }
        }
        if params.get("state").map(|s| s.as_ref()) != Some(&self.state) {
            return Err("Invalid OAuth state");
        }
        if params.contains_key("error") {
            return Ok(Err(
                "ChatGPT sign-in was declined or interrupted. Return to settings to try again."
                    .into(),
            ));
        }
        let issued = params.get("client_id").map(|s| s.as_ref());
        let selected = (self.client_id != DYNAMIC).then_some(self.client_id.as_str());
        if selected.is_some() && issued.is_some() && issued != selected {
            return Ok(Err(
                "ChatGPT returned a different registration. The saved account has been preserved."
                    .into(),
            ));
        }
        let Some(id) = issued
            .or(selected)
            .filter(|id| !id.trim().is_empty() && *id != DYNAMIC)
        else {
            return Ok(Err(
                "ChatGPT did not issue a client ID. Start sign-in again.".into(),
            ));
        };
        let Some(code) = params.get("code").filter(|code| !code.is_empty()) else {
            return Ok(Err("ChatGPT did not return an authorization code".into()));
        };
        Ok(Ok((code.to_string(), id.into())))
    }

    async fn exchange(
        mut self,
        mut listener: TcpListener,
        app: tauri::AppHandle,
        host: String,
        mut profile: LoginProfile,
    ) -> Result<OAuthConfig, String> {
        let client = http()?;
        let mut restarted = false;
        let (client_id, value) = loop {
            let (code, client_id) =
                wait_for_callback(listener, |request| self.callback(request)).await??;
            {
                let _lock = store_lock().await?;
                profile.record_client(&credential_root()?, &client_id)?;
            }
            let (status, value) = response_json(client.post(TOKEN).form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id.as_str()),
                ("code", &code),
                ("code_verifier", &self.verifier),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("resource", CHATGPT_BASE),
            ]))
            .await?;
            if status == 200 {
                break (client_id, value);
            }
            let code_expired =
                value["error"] == "invalid_grant" || value["error"]["code"] == "invalid_grant";
            if status == 400 && code_expired && !restarted {
                // The registration was issued but its code expired. Retry once with fresh proof,
                // the issued client and the same expected identity; never replay the spent code.
                restarted = true;
                (self, listener) =
                    Self::open(&app, &host, profile.registration.as_ref(), client_id).await?;
                continue;
            }
            return Err(format!(
                "ChatGPT code exchange failed (HTTP {status}). Start sign-in again."
            ));
        };
        let id_token = field(&value, "id_token")?;
        let keys = signing_keys(&client).await?;
        let identity = verified_identity(&id_token, &client_id, Some(&self.nonce), now(), &keys)?;
        if profile
            .registration
            .as_ref()
            .and_then(|reg| reg.subject.as_ref())
            .is_some_and(|sub| sub != &identity.sub)
        {
            return Err("The verified identity differs from the selected ChatGPT account. Its credentials have been preserved.".into());
        }
        let reg = Registration {
            client_id,
            subject: Some(identity.sub),
            revision: Some(uuid::Uuid::new_v4()),
            name: identity.name,
            email: identity.email,
            id_token: Some(id_token),
            scopes: Vec::new(),
            earliest_refresh_at: None,
            pending_refresh: None,
        };
        let tokens = session_tokens(&value, reg, true)?;
        let _lock = store_lock().await?;
        profile.complete(&credential_root()?, &tokens)
    }
}

/// The profile seen when the browser opened. All checks and writes are made under the store lock.
struct LoginProfile {
    id: String,
    registration: Option<Registration>,
}

impl LoginProfile {
    fn load(root: &Path, id: Option<String>) -> Result<Self, String> {
        if let Some(id) = id {
            let registration = saved_session(root, &id)?
                .chatgpt
                .ok_or("Missing ChatGPT registration")?;
            Ok(Self {
                id,
                registration: Some(registration),
            })
        } else {
            Ok(Self {
                id: new_credential_id(),
                registration: None,
            })
        }
    }

    fn check_current(&self, root: &Path) -> Result<(), String> {
        let expected = self
            .registration
            .as_ref()
            .ok_or("Missing ChatGPT registration")?;
        let current = saved_session(root, &self.id)?
            .chatgpt
            .ok_or("Missing ChatGPT registration")?;
        if current.revision != expected.revision || current.client_id != expected.client_id {
            return Err(
                "This ChatGPT account changed during sign-in. Select the account and try again."
                    .into(),
            );
        }
        Ok(())
    }

    fn record_client(&mut self, root: &Path, client_id: &str) -> Result<(), String> {
        if self.registration.is_some() {
            return self.check_current(root);
        }
        if saved_accounts(root)?
            .iter()
            .any(|account| account.client_id == client_id)
        {
            return Err(
                "This ChatGPT registration is already saved. Select it to continue.".into(),
            );
        }
        let pending = Registration {
            client_id: client_id.into(),
            revision: Some(uuid::Uuid::new_v4()),
            ..Registration::default()
        };
        let tokens = Tokens {
            provider: "chatgpt".into(),
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
            project_id: None,
            chatgpt: Some(pending.clone()),
        };
        save_tokens_in(root, &self.id, &tokens)?;
        self.registration = Some(pending);
        Ok(())
    }

    fn complete(&self, root: &Path, tokens: &Tokens) -> Result<OAuthConfig, String> {
        self.check_current(root)?;
        let registration = tokens
            .chatgpt
            .as_ref()
            .ok_or("Missing ChatGPT registration")?;
        save_tokens_in(root, &self.id, tokens)?;
        Ok(OAuthConfig {
            provider: "chatgpt".into(),
            credential_id: Some(self.id.clone()),
            project_id: None,
            requires_reauthorization: !registration.plan_enabled(),
        })
    }
}

pub(super) async fn start(
    app: tauri::AppHandle,
    credential_id: Option<String>,
) -> Result<Login, String> {
    let (host, profile) = {
        let _lock = store_lock().await?;
        let root = credential_root()?;
        (host_id(&root)?, LoginProfile::load(&root, credential_id)?)
    };
    let mut active = sessions().lock().await;
    active.retain(|_, s| s.expires > now());
    if active.len() >= 8 {
        return Err("Too many pending ChatGPT sign-ins".into());
    }
    let initial_client = profile
        .registration
        .as_ref()
        .map(|reg| reg.client_id.clone())
        .unwrap_or_else(|| DYNAMIC.into());
    let (attempt, listener) =
        Attempt::open(&app, &host, profile.registration.as_ref(), initial_client).await?;
    let login_id = uuid::Uuid::new_v4().to_string();
    let expires = now() + 300;
    let task = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(300),
            attempt.exchange(listener, app, host, profile),
        )
        .await
        .map_err(|_| "ChatGPT sign-in expired. Start again.".to_string())?
    });
    active.insert(login_id.clone(), Session { task, expires });
    Ok(Login {
        login_id,
        user_code: String::new(),
        verification_url: String::new(),
        interval: 2,
        expires_at: expires,
    })
}

pub(super) async fn poll(id: &str) -> Option<Result<PollResult, String>> {
    let mut session = {
        let mut active = sessions().lock().await;
        let session = active.get(id)?;
        if !session.task.is_finished() {
            if session.expires <= now() {
                active.remove(id);
                return Some(Err("ChatGPT sign-in expired. Start again.".into()));
            }
            return Some(Ok(PollResult {
                status: "pending",
                interval: 2,
                auth: None,
            }));
        }
        active.remove(id)?
    };
    Some(match (&mut session.task).await {
        Ok(result) => result.map(|auth| PollResult {
            status: "authorized",
            interval: 0,
            auth: Some(auth),
        }),
        Err(_) => Err("ChatGPT sign-in interrupted".into()),
    })
}

pub(super) async fn cancel(id: &str) {
    if let Some(mut session) = sessions().lock().await.remove(id) {
        session.task.abort();
        let _ = (&mut session.task).await;
    }
}

async fn discovery_endpoint(client: &reqwest::Client, key: &str) -> Result<url::Url, String> {
    let (status, value) = response_json(client.get(DISCOVERY)).await?;
    if status != 200 || value["issuer"] != ISSUER {
        return Err("Invalid OpenAI discovery document".into());
    }
    let url =
        url::Url::parse(&field(&value, key)?).map_err(|_| "Invalid OpenAI discovery endpoint")?;
    if url.origin().ascii_serialization() != ISSUER
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("Untrusted OpenAI discovery endpoint".into());
    }
    Ok(url)
}

#[derive(Debug)]
enum IdentityError {
    Invalid,
    KeysUnavailable,
}

impl From<IdentityError> for String {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Invalid => "Invalid ChatGPT ID token. Sign in again.",
            IdentityError::KeysUnavailable => {
                "ChatGPT signing keys are temporarily unavailable. Try again shortly."
            }
        }
        .into()
    }
}

async fn signing_keys(client: &reqwest::Client) -> Result<JwkSet, IdentityError> {
    let url = discovery_endpoint(client, "jwks_uri")
        .await
        .map_err(|_| IdentityError::KeysUnavailable)?;
    let (status, value) = response_json(client.get(url))
        .await
        .map_err(|_| IdentityError::KeysUnavailable)?;
    if status != 200 {
        return Err(IdentityError::KeysUnavailable);
    }
    serde_json::from_value(value).map_err(|_| IdentityError::KeysUnavailable)
}

#[derive(Deserialize)]
struct Identity {
    sub: String,
    iss: String,
    exp: i64,
    #[serde(rename = "iat")]
    _issued_at: i64,
    nbf: Option<i64>,
    nonce: Option<String>,
    email: Option<String>,
    name: Option<String>,
    azp: Option<String>,
    aud: Value,
}

fn verified_identity(
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
    received_at: i64,
    keys: &JwkSet,
) -> Result<Identity, IdentityError> {
    use jsonwebtoken::jwk::{AlgorithmParameters, KeyAlgorithm, KeyOperations, PublicKeyUse};
    let header = decode_header(token).map_err(|_| IdentityError::Invalid)?;
    if header.alg != Algorithm::RS256 {
        return Err(IdentityError::Invalid);
    }
    let mut matching = keys.keys.iter().filter(|jwk| {
        header
            .kid
            .as_ref()
            .is_none_or(|kid| jwk.common.key_id.as_ref() == Some(kid))
            && matches!(jwk.algorithm, AlgorithmParameters::RSA(_))
            && jwk
                .common
                .key_algorithm
                .is_none_or(|alg| alg == KeyAlgorithm::RS256)
            && jwk
                .common
                .public_key_use
                .as_ref()
                .is_none_or(|usage| *usage == PublicKeyUse::Signature)
            && jwk
                .common
                .key_operations
                .as_ref()
                .is_none_or(|ops| ops.contains(&KeyOperations::Verify))
    });
    let jwk = matching.next().ok_or(IdentityError::KeysUnavailable)?;
    if matching.next().is_some() {
        return Err(IdentityError::KeysUnavailable);
    }
    let key = DecodingKey::from_jwk(jwk).map_err(|_| IdentityError::KeysUnavailable)?;
    // The JWK decoder accepts empty or undersized RSA components. These indicate an
    // unusable published key, before a signature can establish anything about the account.
    let jsonwebtoken::DecodingKeyKind::RsaModulusExponent { n, e } = key.kind() else {
        return Err(IdentityError::KeysUnavailable);
    };
    let bits = n.len() * 8 - n.first().map_or(0, |first| first.leading_zeros() as usize);
    if !(2048..=8192).contains(&bits)
        || n.last().is_none_or(|byte| byte & 1 == 0)
        || e.first().is_none_or(|first| *first == 0)
        || e.last().is_none_or(|byte| byte & 1 == 0)
        || e.as_slice() == [1]
    {
        return Err(IdentityError::KeysUnavailable);
    }

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[client_id]);
    // jsonwebtoken's temporal checks always use the wall clock. Check the signed claims
    // against receipt instead so a JWKS outage cannot expire an otherwise valid rotation.
    validation.validate_exp = false;
    validation.validate_nbf = false;
    let identity = decode::<Identity>(token, &key, &validation)
        .map_err(|_| IdentityError::Invalid)?
        .claims;
    if identity.iss != ISSUER
        || identity.exp <= received_at.saturating_sub(5)
        || identity
            .nbf
            .is_some_and(|t| t > received_at.saturating_add(5))
        || nonce.is_some_and(|value| identity.nonce.as_deref() != Some(value))
        || identity.sub.trim().is_empty()
        || identity.azp.as_deref().is_some_and(|p| p != client_id)
        || (identity.aud.as_array().is_some_and(|aud| aud.len() > 1) && identity.azp.is_none())
    {
        return Err(IdentityError::Invalid);
    }
    Ok(identity)
}

#[derive(Serialize, Deserialize, Clone)]
struct PendingRefresh {
    id_token: String,
    received_at: i64,
    grant: TokenGrant,
}

#[derive(Serialize, Deserialize, Clone)]
struct TokenGrant {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    scopes: Vec<String>,
    earliest_refresh_at: Option<i64>,
}

impl TokenGrant {
    fn parse(
        value: &Value,
        previous_scopes: Option<&[String]>,
        received_at: i64,
    ) -> Result<Self, String> {
        let scopes: Vec<String> = match (value.get("scope"), previous_scopes) {
            (Some(Value::String(scope)), _) => {
                scope.split_ascii_whitespace().map(str::to_owned).collect()
            }
            (None | Some(Value::Null), Some(previous)) => previous.to_vec(),
            _ => {
                return Err(
                    "ChatGPT did not confirm the granted scopes. Start sign-in again.".into(),
                )
            }
        };
        let requires_refresh =
            previous_scopes.is_some() || scopes.iter().any(|s| s == "offline_access");
        let (access_token, expires_at) = match field(value, "access_token") {
            Err(_) if !requires_refresh && !scopes.iter().any(|s| s == DIRECT) => {
                (String::new(), 0)
            }
            Ok(access) => {
                if !field(value, "token_type")?.eq_ignore_ascii_case("Bearer") {
                    return Err("Unsupported ChatGPT token type".into());
                }
                let lifetime = value["expires_in"]
                    .as_i64()
                    .filter(|v| *v > 0)
                    .ok_or("Invalid ChatGPT access token lifetime")?;
                (
                    access,
                    received_at
                        .checked_add(lifetime)
                        .ok_or("Invalid ChatGPT access token lifetime")?,
                )
            }
            Err(error) => return Err(error),
        };
        let earliest_refresh_at = value["earliest_refresh_at"].as_i64().or_else(|| {
            let timestamp = value["earliest_refresh_at"].as_str()?;
            timestamp.parse().ok().or_else(|| {
                chrono::DateTime::parse_from_rfc3339(timestamp)
                    .ok()
                    .map(|t| t.timestamp())
            })
        });
        let refresh_token = if requires_refresh {
            field(value, "refresh_token")?
        } else {
            value["refresh_token"].as_str().unwrap_or_default().into()
        };
        Ok(Self {
            access_token,
            expires_at,
            refresh_token,
            scopes,
            earliest_refresh_at,
        })
    }

    fn into_tokens(self, mut reg: Registration) -> Tokens {
        reg.scopes = self.scopes;
        reg.earliest_refresh_at = self.earliest_refresh_at;
        reg.pending_refresh = None;
        Tokens {
            provider: "chatgpt".into(),
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at: self.expires_at,
            project_id: None,
            chatgpt: Some(reg),
        }
    }
}

fn session_tokens(value: &Value, reg: Registration, initial: bool) -> Result<Tokens, String> {
    let grant = TokenGrant::parse(value, (!initial).then_some(reg.scopes.as_slice()), now())?;
    Ok(grant.into_tokens(reg))
}

fn clear_session(tokens: &mut Tokens) {
    tokens.access_token.clear();
    tokens.refresh_token.clear();
    tokens.expires_at = 0;
    if let Some(reg) = &mut tokens.chatgpt {
        reg.id_token = None;
        reg.scopes.clear();
        reg.earliest_refresh_at = None;
        reg.pending_refresh = None;
        reg.revision = Some(uuid::Uuid::new_v4());
    }
}

fn invalidate(root: &Path, id: &str, tokens: &mut Tokens, error: String) -> Result<(), String> {
    clear_session(tokens);
    save_tokens_in(root, id, tokens)?;
    Err(error)
}

/// The issuer may have consumed the old grant. Write the successor before any further await.
fn record_refresh(
    root: &Path,
    id: &str,
    tokens: &mut Tokens,
    value: &Value,
    received_at: i64,
) -> Result<(), String> {
    let reg = tokens
        .chatgpt
        .as_mut()
        .ok_or("Missing ChatGPT registration")?;
    let grant = match TokenGrant::parse(value, Some(&reg.scopes), received_at) {
        Ok(grant) => grant,
        Err(error) => return invalidate(root, id, tokens, error),
    };
    reg.revision = Some(uuid::Uuid::new_v4());
    match value.get("id_token") {
        None => *tokens = grant.into_tokens(reg.clone()),
        Some(Value::String(id_token)) if !id_token.is_empty() => {
            reg.pending_refresh = Some(PendingRefresh {
                id_token: id_token.clone(),
                received_at,
                grant,
            });
        }
        Some(_) => return invalidate(root, id, tokens, IdentityError::Invalid.into()),
    }
    save_tokens_in(root, id, tokens)
}

fn finish_refresh(root: &Path, id: &str, tokens: &mut Tokens, keys: &JwkSet) -> Result<(), String> {
    let reg = tokens
        .chatgpt
        .as_mut()
        .ok_or("Missing ChatGPT registration")?;
    let pending = reg
        .pending_refresh
        .as_ref()
        .ok_or("Missing ChatGPT refresh checkpoint")?;
    if pending.received_at <= 0 || pending.received_at > now().saturating_add(5) {
        return invalidate(
            root,
            id,
            tokens,
            "Invalid ChatGPT refresh receipt. Sign in again.".into(),
        );
    }
    let identity = match verified_identity(
        &pending.id_token,
        &reg.client_id,
        None,
        pending.received_at,
        keys,
    ) {
        Ok(identity) if reg.subject.as_deref() == Some(identity.sub.as_str()) => identity,
        Ok(_) => {
            return invalidate(
                root,
                id,
                tokens,
                "The refreshed ChatGPT identity differs from the saved account. Sign in again."
                    .into(),
            )
        }
        Err(IdentityError::KeysUnavailable) => return Err(IdentityError::KeysUnavailable.into()),
        Err(error) => return invalidate(root, id, tokens, error.into()),
    };
    let pending = reg.pending_refresh.take().expect("checked above");
    reg.name = identity.name;
    reg.email = identity.email;
    reg.id_token = Some(pending.id_token);
    reg.revision = Some(uuid::Uuid::new_v4());
    *tokens = pending.grant.into_tokens(reg.clone());
    save_tokens_in(root, id, tokens)
}

pub(super) async fn prepare_session(id: &str, tokens: &mut Tokens) -> Result<(), String> {
    let reg = tokens
        .chatgpt
        .as_ref()
        .ok_or("Missing ChatGPT registration. Sign in again.")?;
    if reg.id_token.is_none() {
        return Err("Sign in to your saved ChatGPT account again".into());
    }
    if !reg.plan_enabled() {
        return Err(
            "ChatGPT plan usage is disabled. Enable it in provider settings or select an API key."
                .into(),
        );
    }
    if reg.pending_refresh.is_none() {
        if tokens.access_token.is_empty() {
            return Err(
                "ChatGPT access token is missing. Sign in to your saved account again.".into(),
            );
        }
        let current = now();
        if tokens.expires_at > current + 60 {
            return Ok(());
        }
        if reg.earliest_refresh_at.is_some_and(|t| t > current) {
            return if tokens.expires_at > current {
                Ok(())
            } else {
                Err("ChatGPT cannot refresh this account yet. Try again shortly.".into())
            };
        }
        if tokens.refresh_token.is_empty() {
            if tokens.expires_at > current {
                return Ok(());
            }
            return invalidate(
                &credential_root()?,
                id,
                tokens,
                "ChatGPT access expired. Sign in to your saved account again.".into(),
            );
        }
    }
    refresh_session(&credential_root()?, id, tokens, &http()?).await
}

async fn refresh_session(
    root: &Path,
    id: &str,
    tokens: &mut Tokens,
    client: &reqwest::Client,
) -> Result<(), String> {
    let reg = tokens
        .chatgpt
        .as_ref()
        .ok_or("Missing ChatGPT registration")?;
    if reg.pending_refresh.is_none() {
        let (status, value) = response_json(client.post(TOKEN).form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &reg.client_id),
            ("refresh_token", &tokens.refresh_token),
            ("resource", CHATGPT_BASE),
        ]))
        .await?;
        if status != 200 {
            let code = value["error"]
                .as_str()
                .or_else(|| value["error"]["code"].as_str());
            if matches!(
                code,
                Some(
                    "invalid_grant"
                        | "invalid_refresh_token"
                        | "token_expired"
                        | "refresh_token_expired"
                        | "refresh_token_invalidated"
                        | "refresh_token_reused"
                )
            ) {
                return invalidate(
                    root,
                    id,
                    tokens,
                    "ChatGPT session expired or was revoked. Sign in to your saved account again."
                        .into(),
                );
            }
            return Err(format!("ChatGPT session refresh failed (HTTP {status})"));
        }
        record_refresh(root, id, tokens, &value, now())?;
    }
    if tokens
        .chatgpt
        .as_ref()
        .is_some_and(|reg| reg.pending_refresh.is_some())
    {
        let keys = signing_keys(client).await?;
        finish_refresh(root, id, tokens, &keys)?;
    }
    if !tokens
        .chatgpt
        .as_ref()
        .is_some_and(Registration::plan_enabled)
    {
        return Err("ChatGPT plan permission was removed. Enable it in provider settings.".into());
    }
    if tokens.expires_at <= now() {
        return Err(
            "ChatGPT credentials were recovered. Retry the request to renew the access token."
                .into(),
        );
    }
    Ok(())
}

pub(super) async fn sign_out(id: &str, tokens: &mut Tokens) -> Result<Option<String>, String> {
    let reg = tokens
        .chatgpt
        .as_ref()
        .ok_or("Missing ChatGPT registration")?;
    let refresh_token = reg
        .pending_refresh
        .as_ref()
        .map(|p| p.grant.refresh_token.as_str())
        .unwrap_or(&tokens.refresh_token);
    let mut confirmed = refresh_token.is_empty();
    if !confirmed {
        if let Ok(client) = http() {
            for attempt in 0..2 {
                if attempt > 0 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                let Ok(endpoint) = discovery_endpoint(&client, "revocation_endpoint").await else {
                    continue;
                };
                let response = client
                    .post(endpoint)
                    .form(&[
                        ("token", refresh_token),
                        ("token_type_hint", "refresh_token"),
                        ("client_id", &reg.client_id),
                    ])
                    .send()
                    .await;
                if let Ok(response) = response {
                    confirmed = response.status() == reqwest::StatusCode::OK;
                    if !response.status().is_server_error() {
                        break;
                    }
                }
            }
        }
    }
    clear_session(tokens);
    save_tokens(id, tokens)?;
    Ok((!confirmed).then(|| "Signed out locally. Remote revocation was not confirmed. You can disconnect Kivio in ChatGPT Settings.".into()))
}

#[cfg(test)]
#[path = "chatgpt_tests.rs"]
mod tests;
