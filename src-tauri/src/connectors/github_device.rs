//! GitHub's public desktop client: device authorization needs no application secret.
//! Only the official MCP resource can use this built-in identity.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::{sleep, timeout};

use super::oauth::{self, TokenResponse};
use crate::settings::ChatMcpServer;

// Public application identity, registered by Kivio's maintainer. Never embed its secret.
pub const CLIENT_ID: &str = "Ov23liilcX7ps76sGn1r";
pub const RESOURCE: &str = "https://api.githubcopilot.com/mcp";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub fn is_resource(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.githubcopilot.com")
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "/mcp" | "/mcp/")
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

fn trusted_endpoint(value: &str, path: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == path
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

async fn post_form(
    http: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    timeout(REQUEST_TIMEOUT, async {
        let response = http
            .post(endpoint)
            .header("Accept", "application/json")
            .form(form)
            .send()
            .await
            .map_err(|_| "GitHub authorization network request failed".to_string())?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| "GitHub authorization returned an invalid response".to_string())?;
        if !status.is_success() && value.get("error").is_none() {
            return Err(format!("GitHub authorization HTTP {}", status.as_u16()));
        }
        Ok(value)
    })
    .await
    .map_err(|_| "GitHub authorization request timed out".to_string())?
}

fn authorization_error(code: &str) -> String {
    match code {
        "device_flow_disabled" => "GitHub device authorization is not enabled for Kivio. Please contact the application maintainer.",
        "access_denied" => "GitHub authorization was declined. You can connect again.",
        "expired_token" | "token_expired" => "GitHub authorization code expired. Please connect again.",
        "incorrect_client_credentials" => "GitHub did not recognize Kivio's application identity.",
        _ => "GitHub authorization failed. Please try again.",
    }.to_string()
}

async fn poll_token(
    http: &reqwest::Client,
    endpoint: &str,
    device_code: &str,
    mut interval: Duration,
) -> Result<TokenResponse, String> {
    loop {
        sleep(interval).await;
        let value = post_form(
            http,
            endpoint,
            &[
                ("client_id", CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
        )
        .await?;
        match value.get("error").and_then(|v| v.as_str()) {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                interval =
                    interval
                        .saturating_add(Duration::from_secs(5))
                        .max(Duration::from_secs(
                            value
                                .get("interval")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                .min(900),
                        ));
            }
            Some(error) => return Err(authorization_error(error)),
            None => return oauth::parse_token_response(&value),
        }
    }
}

async fn authorize(
    http: &reqwest::Client,
    device_endpoint: &str,
    token_endpoint: &str,
    on_prompt: impl FnOnce(DevicePrompt) -> Result<(), String>,
) -> Result<TokenResponse, String> {
    let value = post_form(
        http,
        device_endpoint,
        &[
            ("client_id", CLIENT_ID),
            ("scope", "public_repo offline_access"),
        ],
    )
    .await?;
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
        return Err(authorization_error(error));
    }
    let code: DeviceCode = serde_json::from_value(value)
        .map_err(|_| "GitHub returned an invalid device authorization code".to_string())?;
    if code.device_code.is_empty()
        || code.user_code.is_empty()
        || code.expires_in == 0
        || !trusted_endpoint(&code.verification_uri, "/login/device")
    {
        return Err("GitHub returned an invalid device authorization code".into());
    }
    let expires_in = code.expires_in.min(900);
    timeout(Duration::from_secs(expires_in), async {
        on_prompt(DevicePrompt {
            user_code: code.user_code,
            verification_uri: code.verification_uri,
            expires_in,
        })?;
        poll_token(
            http,
            token_endpoint,
            &code.device_code,
            Duration::from_secs(code.interval.clamp(1, 900)),
        )
        .await
    })
    .await
    .map_err(|_| authorization_error("expired_token"))?
}

pub async fn connect(
    http: &reqwest::Client,
    connector_id: &str,
    name: &str,
    resource_url: &str,
    on_prompt: impl FnOnce(DevicePrompt) -> Result<(), String>,
) -> Result<ChatMcpServer, String> {
    if !is_resource(resource_url) {
        return Err("Unsupported GitHub MCP address".into());
    }
    let metadata = oauth::discover_auth_server(http, resource_url).await?;
    let device_endpoint = metadata
        .device_authorization_endpoint
        .as_deref()
        .filter(|endpoint| trusted_endpoint(endpoint, "/login/device/code"))
        .ok_or_else(|| "GitHub did not advertise device authorization".to_string())?;
    if !trusted_endpoint(&metadata.token_endpoint, "/login/oauth/access_token") {
        return Err("Unexpected GitHub token endpoint".into());
    }
    let mut token = authorize(http, device_endpoint, &metadata.token_endpoint, on_prompt).await?;
    // Revalidate the identity associated with this token; do not reuse a previous account label.
    let account = timeout(REQUEST_TIMEOUT, async {
        http.get("https://api.github.com/user")
            .header("User-Agent", "Kivio")
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(&token.access_token)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await
    })
    .await
    .map_err(|_| "GitHub account verification timed out".to_string())?
    .map_err(|_| "GitHub account verification failed. Please authorize again.".to_string())?;
    token.account = Some(
        account
            .get("login")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "GitHub account verification returned no account".to_string())?
            .to_string(),
    );
    Ok(oauth::materialize_server(
        connector_id,
        name,
        resource_url,
        &token,
        &metadata.token_endpoint,
        CLIENT_ID,
        &["public_repo".into()],
        oauth::now_unix(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn endpoint(
        responses: Vec<serde_json::Value>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = vec![];
            for value in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = vec![];
                loop {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0);
                    data.extend_from_slice(&buffer[..count]);
                    if let Some(end) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                        let size = String::from_utf8_lossy(&data[..end])
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if data.len() >= end + 4 + size {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(data).unwrap());
                let body = value.to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        (url, task)
    }

    fn challenge(expires: u64) -> serde_json::Value {
        serde_json::json!({"device_code":"private-device-code", "user_code":"ABCD-EFGH", "verification_uri":"https://github.com/login/device", "expires_in":expires, "interval":1})
    }

    #[test]
    fn builtin_identity_is_only_used_with_the_official_resource() {
        assert!(is_resource(RESOURCE));
        assert!(is_resource(&format!("{RESOURCE}/")));
        for url in [
            "https://api.githubcopilot.com.evil.test/mcp",
            "http://api.githubcopilot.com/mcp",
            "https://user@api.githubcopilot.com/mcp",
            "https://api.githubcopilot.com:444/mcp",
            "https://api.githubcopilot.com/mcp?redirect=evil",
            "https://api.githubcopilot.com/mcp#evil",
        ] {
            assert!(!is_resource(url), "{url}");
        }
    }

    #[tokio::test]
    async fn authorizes_without_an_app_secret_and_keeps_device_code_out_of_ui() {
        let (url, requests) = endpoint(vec![challenge(30), serde_json::json!({"error":"authorization_pending"}), serde_json::json!({"access_token":"test-token", "refresh_token":"test-refresh", "scope":"public_repo", "expires_in":28800})]).await;
        let token = authorize(&reqwest::Client::new(), &url, &url, |prompt| {
            let ui = serde_json::to_string(&prompt).unwrap();
            assert!(ui.contains("ABCD-EFGH"));
            assert!(!ui.contains("private-device-code"));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(token.access_token, "test-token");
        assert_eq!(token.refresh_token.as_deref(), Some("test-refresh"));
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].contains("scope=public_repo+offline_access"));
        assert!(requests[1].contains("device_code=private-device-code"));
        assert!(requests.iter().all(|r| !r.contains("client_secret")));
    }

    #[tokio::test]
    async fn denial_and_disabled_device_flow_are_terminal() {
        for error in ["access_denied", "device_flow_disabled"] {
            let (url, requests) = endpoint(vec![serde_json::json!({"error":error})]).await;
            let result = authorize(&reqwest::Client::new(), &url, &url, |_| {
                panic!("no browser prompt on failure")
            })
            .await;
            assert_eq!(result.unwrap_err(), authorization_error(error));
            requests.await.unwrap();
        }
    }

    #[tokio::test]
    async fn expiration_stops_polling() {
        let (url, requests) = endpoint(vec![challenge(1)]).await;
        let error = authorize(&reqwest::Client::new(), &url, &url, |_| Ok(()))
            .await
            .unwrap_err();
        assert_eq!(error, authorization_error("expired_token"));
        assert_eq!(requests.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn slow_down_increases_the_delay_and_polling_denial_stops() {
        let (url, requests) = endpoint(vec![
            serde_json::json!({"error":"slow_down", "interval":5}),
            serde_json::json!({"error":"access_denied"}),
        ])
        .await;
        let started = tokio::time::Instant::now();
        let result = poll_token(
            &reqwest::Client::new(),
            &url,
            "private-device-code",
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(result.unwrap_err(), authorization_error("access_denied"));
        assert!(started.elapsed() >= Duration::from_secs(5));
        assert_eq!(requests.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rejects_a_verification_page_outside_github() {
        let mut code = challenge(30);
        code["verification_uri"] = serde_json::json!("https://github.com.evil.test/login/device");
        let (url, requests) = endpoint(vec![code]).await;
        assert!(authorize(&reqwest::Client::new(), &url, &url, |_| panic!(
            "untrusted browser navigation"
        ))
        .await
        .is_err());
        requests.await.unwrap();
    }

    #[tokio::test]
    async fn device_token_refresh_needs_no_secret() {
        let (url, requests) = endpoint(vec![serde_json::json!({"access_token":"rotated-token", "refresh_token":"rotated-refresh", "expires_in":28800})]).await;
        let token = oauth::refresh_access_token(
            &reqwest::Client::new(),
            &url,
            "old-refresh",
            Some(CLIENT_ID),
            Some(RESOURCE),
            None,
        )
        .await
        .unwrap();
        assert_eq!(token.refresh_token.as_deref(), Some("rotated-refresh"));
        let request = &requests.await.unwrap()[0];
        assert!(request.contains("grant_type=refresh_token"));
        assert!(!request.contains("client_secret"));
    }
}
