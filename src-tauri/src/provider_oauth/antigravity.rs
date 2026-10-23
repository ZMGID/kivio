//! Protocol adapted from Rahul Arya's pi-antigravity (MIT); see docs/licenses/.
use super::*;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket},
};

#[path = "antigravity_wire.rs"]
mod wire;
#[cfg(test)]
use wire::custom_schema;
pub(crate) use wire::model_includes_effort;
pub use wire::{BASE, is_provider, unwrap_response, user_agent, wrap_request};
pub(super) const CLIENT: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
// Public installed desktop client value, not an account credential.
pub(super) const CLIENT_SECRET: &str = "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";
pub(super) const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REDIRECT: &str = "http://localhost:51121/oauth-callback";

struct Session {
    task: tokio::task::JoinHandle<Result<Tokens, String>>,
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
fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) async fn start(proxy: bool) -> Result<Login, String> {
    let mut active = sessions().lock().await;
    active.retain(|_, s| s.expires > now());
    if !active.is_empty() {
        return Err("Antigravity login is already pending; cancel it first".into());
    }
    let listener = callback_listener((std::net::Ipv4Addr::LOCALHOST, 51121).into()).map_err(
        |_| "Cannot listen on localhost:51121; close another Antigravity login and retry",
    )?;
    let verifier = random_secret();
    let state = random_secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", CLIENT), ("response_type", "code"), ("redirect_uri", REDIRECT),
        ("scope", "https://www.googleapis.com/auth/aicode https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs"),
        ("code_challenge", &challenge), ("code_challenge_method", "S256"),
        ("state", &state), ("access_type", "offline"), ("prompt", "consent"),
    ]);
    let id = uuid::Uuid::new_v4().to_string();
    let expires = now() + 300;
    let task = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(300),
            authorize(listener, state, verifier, proxy),
        )
        .await
        .map_err(|_| "Antigravity login expired; start again".to_string())?
    });
    active.insert(id.clone(), Session { task, expires });
    Ok(Login {
        login_id: id,
        user_code: String::new(),
        verification_url: url.into(),
        interval: 3,
        expires_at: expires,
    })
}

fn callback_listener(address: std::net::SocketAddr) -> std::io::Result<TcpListener> {
    let socket = TcpSocket::new_v4()?;
    // A completed callback can leave the accepted connection in TIME_WAIT on Unix.
    // Reusing the address lets an immediate retry bind the fixed OAuth callback port.
    #[cfg(unix)]
    socket.set_reuseaddr(true)?;
    socket.bind(address)?;
    socket.listen(1024)
}

// Reject unexpected paths, methods, duplicate parameters and state before accepting a code.
fn callback(request: &str, state: &str) -> Result<Option<String>, &'static str> {
    let mut line = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    if line.next() != Some("GET") {
        return Err("Invalid callback method");
    }
    let target = line.next().ok_or("Missing callback path")?;
    if !target.starts_with("/oauth-callback?") {
        return Err("Invalid callback path");
    }
    let url = url::Url::parse(&format!("http://localhost:51121{target}"))
        .map_err(|_| "Invalid callback URL")?;
    if url.path() != "/oauth-callback" {
        return Err("Invalid callback path");
    }
    let params: Vec<_> = url.query_pairs().collect();
    let states: Vec<_> = params.iter().filter(|(k, _)| k == "state").collect();
    if states.len() != 1 || states[0].1 != state {
        return Err("OAuth state mismatch");
    }
    if params.iter().any(|(k, _)| k == "error") {
        return Ok(None);
    }
    let codes: Vec<_> = params.iter().filter(|(k, _)| k == "code").collect();
    if codes.len() != 1 || codes[0].1.is_empty() {
        return Err("Missing authorization code");
    }
    Ok(Some(codes[0].1.to_string()))
}

async fn authorize(
    listener: TcpListener,
    state: String,
    verifier: String,
    proxy: bool,
) -> Result<Tokens, String> {
    let code = loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|_| "OAuth callback listener failed")?;
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let mut bytes = Vec::new();
            loop {
                let mut block = [0; 1024];
                let n = socket
                    .read(&mut block)
                    .await
                    .map_err(|_| "Callback read failed")?;
                if n == 0 {
                    return Err("Incomplete callback");
                }
                bytes.extend_from_slice(&block[..n]);
                if bytes.len() > 8192 {
                    return Err("Callback too large");
                }
                if bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    break;
                }
            }
            callback(
                std::str::from_utf8(&bytes).map_err(|_| "Invalid callback encoding")?,
                &state,
            )
        })
        .await;
        let accepted = matches!(result, Ok(Ok(_)));
        let status = if accepted {
            "200 OK"
        } else {
            "400 Bad Request"
        };
        let body = if accepted {
            "Authorization received. Return to Kivio to finish signing in."
        } else {
            "Invalid OAuth callback. Return to your Google sign-in page."
        };
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ =
            tokio::time::timeout(Duration::from_secs(2), socket.write_all(reply.as_bytes())).await;
        if let Ok(Ok(code)) = result {
            break code.ok_or("Google authorization was declined")?;
        }
    };
    drop(listener);
    let client = http(proxy)?;
    let (status, value) = response_json(client.post(TOKEN_URL).form(&[
        ("client_id", CLIENT),
        ("client_secret", CLIENT_SECRET),
        ("code", &code),
        ("grant_type", "authorization_code"),
        ("redirect_uri", REDIRECT),
        ("code_verifier", &verifier),
    ]))
    .await?;
    if status != 200 {
        return Err(format!("Antigravity token exchange failed (HTTP {status})"));
    }
    let mut tokens = tokens_from(&value, "antigravity", "")?;
    let value = control(
        &client,
        &tokens.access_token,
        "loadCodeAssist",
        json!({"metadata":{"ideType":"ANTIGRAVITY"}}),
    )
    .await?;
    tokens.project_id = project_id(&value);
    if tokens.project_id.is_none() {
        return Err("No Antigravity project is available for this account. Complete account setup in Antigravity, then sign in again.".into());
    }
    Ok(tokens)
}

fn project_id(value: &Value) -> Option<String> {
    [
        "cloudaicompanionProject",
        "projectId",
        "antigravityProjectId",
        "project",
    ]
    .iter()
    .find_map(|k| {
        value[*k]
            .as_str()
            .or_else(|| value[*k]["id"].as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
    })
}

pub(super) async fn poll(id: &str) -> Option<Result<PollResult, String>> {
    let mut active = sessions().lock().await;
    let session = active.get_mut(id)?;
    if !session.task.is_finished() && session.expires > now() {
        return Some(Ok(PollResult {
            status: "pending",
            interval: 3,
            auth: None,
        }));
    }
    let result = if session.expires <= now() {
        Err("Antigravity login expired; start again".into())
    } else {
        match (&mut session.task).await {
            Ok(Ok(tokens)) => save_tokens(id, &tokens).map(|_| PollResult {
                status: "authorized",
                interval: 0,
                auth: Some(OAuthConfig {
                    provider: "antigravity".into(),
                    credential_id: Some(id.into()),
                    project_id: None,
                }),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Antigravity login was interrupted".into()),
        }
    };
    active.remove(id);
    Some(result)
}

pub(super) async fn cancel(id: &str) {
    if let Some(mut session) = sessions().lock().await.remove(id) {
        session.task.abort();
        let _ = (&mut session.task).await;
    }
}

pub(super) async fn control(
    client: &reqwest::Client,
    token: &str,
    method: &str,
    body: Value,
) -> Result<Value, String> {
    let (status, value) = response_json(
        client
            .post(format!("{BASE}/v1internal:{method}"))
            .bearer_auth(token)
            .header("User-Agent", user_agent())
            .json(&body),
    )
    .await?;
    if status != 200 {
        return Err(format!(
            "Antigravity {method} failed (HTTP {status}); check account access and quota"
        ));
    }
    Ok(value)
}

pub(super) async fn models(provider: &ModelProvider) -> Result<Vec<String>, String> {
    let value = control(
        &http(provider.request.use_system_proxy)?,
        provider
            .preferred_api_key()
            .ok_or("Missing OAuth session")?,
        "fetchAvailableModels",
        json!({"project": provider.request.oauth.as_ref().and_then(|a| a.project_id.as_deref())}),
    )
    .await?;
    parse_models(&value)
}
fn parse_models(value: &Value) -> Result<Vec<String>, String> {
    let map = value["models"]
        .as_object()
        .ok_or("Invalid Antigravity model catalog")?;
    let mut ids: Vec<String> = map
        .iter()
        .filter(|(id, v)| {
            ["gemini-", "claude-", "gpt-oss-"]
                .iter()
                .any(|p| id.starts_with(p))
                && !id.chars().any(char::is_whitespace)
                && v["visibility"] != "hide"
        })
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancel_releases_listener_without_saving_credentials() {
        let listener = callback_listener("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.local_addr().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let task = tokio::spawn(authorize(
            listener,
            "test-state".into(),
            "test-verifier".into(),
            false,
        ));
        sessions().lock().await.insert(
            id.clone(),
            Session {
                task,
                expires: now() + 300,
            },
        );
        assert_eq!(poll(&id).await.unwrap().unwrap().status, "pending");
        cancel(&id).await;
        let _listener = callback_listener(address).unwrap();
        assert!(poll(&id).await.is_none());
    }
    #[tokio::test]
    async fn callback_rejects_bad_state_then_accepts_denial() {
        let listener = callback_listener("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(authorize(
            listener,
            "expected".into(),
            "verifier".into(),
            false,
        ));
        for (state, status) in [("wrong", "400 Bad Request"), ("expected", "200 OK")] {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket.write_all(format!("GET /oauth-callback?state={state}&error=access_denied HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).await.unwrap();
            let mut response = String::new();
            tokio::time::timeout(Duration::from_secs(5), socket.read_to_string(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert!(response.contains(status));
            assert!(!response.contains(state));
        }
        assert!(task.await.unwrap().err().unwrap().contains("declined"));
        // A completed TCP connection can keep the port in TIME_WAIT on macOS.
        // Refusing a new connection verifies that the listener is gone without
        // depending on when the OS allows the same port to be rebound.
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }
    #[test]
    fn validates_callback() {
        assert_eq!(
            callback(
                "GET /oauth-callback?state=ok&code=a%2Bb HTTP/1.1\r\n\r\n",
                "ok"
            ),
            Ok(Some("a+b".into()))
        );
        for path in [
            "/other?state=ok&code=a",
            "/oauth-callback?state=bad&code=a",
            "/oauth-callback?state=ok&state=ok&code=a",
            "/oauth-callback?state=ok&code=a&code=b",
        ] {
            assert!(callback(&format!("GET {path} HTTP/1.1"), "ok").is_err());
        }
        assert_eq!(
            callback(
                "GET /oauth-callback?state=ok&error=access_denied HTTP/1.1",
                "ok"
            ),
            Ok(None)
        );
    }
    #[test]
    fn catalog_uses_runtime_keys() {
        assert_eq!(parse_models(&json!({"models":{"claude-example":{"model":"MODEL_PLACEHOLDER_1"},"MODEL_PLACEHOLDER_2":{},"gemini-hidden":{"visibility":"hide"}}})).unwrap(), vec!["claude-example"]);
        assert!(project_id(&json!({})).is_none());
        assert_eq!(
            project_id(&json!({"cloudaicompanionProject":{"id":"owned"}})),
            Some("owned".into())
        );
    }
    #[test]
    fn schema_keeps_property_names() {
        let schema = custom_schema(
            &json!({"type":"object","additionalProperties":false,"properties":{"default":{"type":["string","null"],"default":"x"}},"required":["default"]}),
        );
        assert_eq!(schema["properties"]["default"]["type"], "string");
        assert!(schema["properties"]["default"].get("default").is_none());
        assert_eq!(schema["required"], json!(["default"]));
    }
}
