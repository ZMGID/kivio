//! App-wide outbound proxy: the single owner of how Kivio reaches the network.
//!
//! One setting (`Settings.network_proxy`) decides the route for every outbound HTTP client,
//! every child process (CLI agents, MCP stdio servers, shells, installers, the HTML preview
//! browser) and, on Windows, the WebView2 windows:
//!
//! - `system`: OS / inherited environment proxy (the behaviour before this setting existed).
//! - `none`: always connect directly.
//! - `custom`: route through one `http`/`https`/`socks5`/`socks5h` proxy URL.
//!
//! Loopback traffic always bypasses a custom proxy, so local IPC (Kivio-server, hooks, CLI
//! loopback servers) never leaks bearer credentials to a third party. Callers that talk only to
//! loopback keep building `no_proxy()` clients on purpose.
//!
//! Lifecycle: [`install`] runs once at startup and after every published settings change. It
//! bumps a generation counter; [`SharedHttp`] caches rebuild lazily on the next use, while
//! requests already in flight finish on the client they started with. WebView2 reads its
//! proxy only when the browser process starts, so [`webview_browser_args`] is frozen at the
//! first window and a change takes effect after restarting Kivio.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const MODE_SYSTEM: &str = "system";
pub const MODE_NONE: &str = "none";
pub const MODE_CUSTOM: &str = "custom";

/// Hosts that never go through a custom proxy, whatever the user's bypass list says.
const LOOPBACK_BYPASS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];
const PROXY_ENV: [&str; 3] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"];
const TEST_URL: &str = "https://www.google.com/generate_204";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkProxySettings {
    /// `system` | `none` | `custom`.
    pub mode: String,
    /// Custom proxy URL, e.g. `http://127.0.0.1:7890` or `socks5://user:pass@host:1080`.
    /// Kept while another mode is selected so switching back does not lose it.
    pub url: String,
    /// Extra hosts that skip the custom proxy (comma, semicolon or newline separated).
    pub bypass: String,
}

impl Default for NetworkProxySettings {
    fn default() -> Self {
        Self {
            mode: MODE_SYSTEM.to_string(),
            url: String::new(),
            bypass: String::new(),
        }
    }
}

/// Backend normalization for loaded or submitted settings. A hand-edited file with an unusable
/// custom URL falls back to the system route instead of silently connecting through nothing.
pub fn sanitize(settings: &mut NetworkProxySettings) {
    settings.url = settings.url.trim().to_string();
    settings.bypass = bypass_entries(&settings.bypass).join(", ");
    if ![MODE_SYSTEM, MODE_NONE, MODE_CUSTOM].contains(&settings.mode.as_str()) {
        settings.mode = MODE_SYSTEM.to_string();
    }
    if settings.mode == MODE_CUSTOM && parse_url(&settings.url).is_err() {
        settings.mode = MODE_SYSTEM.to_string();
    }
}

/// Save-time check so an invalid custom proxy is reported instead of normalized away.
pub fn validate(settings: &NetworkProxySettings) -> Result<(), String> {
    if settings.mode.trim() == MODE_CUSTOM {
        parse_url(settings.url.trim())?;
    }
    Ok(())
}

fn parse_url(raw: &str) -> Result<reqwest::Url, String> {
    if raw.is_empty() {
        return Err("Enter a proxy address, e.g. http://127.0.0.1:7890".into());
    }
    let url = reqwest::Url::parse(raw)
        .map_err(|_| "Invalid proxy address (expected scheme://host:port)".to_string())?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err(format!(
            "Unsupported proxy protocol \"{}\"; use http, https, socks5 or socks5h",
            url.scheme()
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("The proxy address is missing a host".into());
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        return Err("A proxy address cannot contain a path or query".into());
    }
    Ok(url)
}

fn bypass_entries(raw: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for entry in raw.split([',', ';', '\n', '\r', ' ', '\t']) {
        let entry = entry.trim();
        if !entry.is_empty() && !entries.iter().any(|e| e.eq_ignore_ascii_case(entry)) {
            entries.push(entry.to_string());
        }
    }
    entries
}

/// User bypass list plus the mandatory loopback hosts.
fn full_bypass(settings: &NetworkProxySettings) -> Vec<String> {
    let mut entries = bypass_entries(&settings.bypass);
    for host in LOOPBACK_BYPASS {
        if !entries.iter().any(|e| e.eq_ignore_ascii_case(host)) {
            entries.push(host.to_string());
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Active configuration
// ---------------------------------------------------------------------------

static ACTIVE: RwLock<Option<NetworkProxySettings>> = RwLock::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(1);

fn active() -> NetworkProxySettings {
    ACTIVE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Apply a canonical setting to the whole process. No-op when nothing changed.
pub fn install(settings: &NetworkProxySettings) {
    let mut settings = settings.clone();
    sanitize(&mut settings);
    {
        let mut slot = ACTIVE.write().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref() == Some(&settings) {
            return;
        }
        // The first install also freezes the WebView proxy for this process.
        let _ = webview_browser_args_for(&settings);
        apply_child_env(&settings);
        *slot = Some(settings);
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

// ---------------------------------------------------------------------------
// HTTP clients
// ---------------------------------------------------------------------------

/// A `reqwest::Client::builder()` that follows the app-wide proxy. Every outbound client in
/// Kivio starts here; only loopback-only clients use a plain `no_proxy()` builder.
pub fn client_builder() -> reqwest::ClientBuilder {
    apply(reqwest::Client::builder(), &active())
}

fn apply(
    builder: reqwest::ClientBuilder,
    settings: &NetworkProxySettings,
) -> reqwest::ClientBuilder {
    match settings.mode.as_str() {
        MODE_NONE => builder.no_proxy(),
        MODE_CUSTOM => match custom_proxy(settings) {
            Ok(proxy) => builder.no_proxy().proxy(proxy),
            Err(error) => {
                // Unreachable for sanitized settings; keep the system route rather than guess.
                eprintln!("[network-proxy] ignoring invalid custom proxy: {error}");
                builder
            }
        },
        _ => builder,
    }
}

fn custom_proxy(settings: &NetworkProxySettings) -> Result<reqwest::Proxy, String> {
    let url = parse_url(settings.url.trim())?;
    let proxy = reqwest::Proxy::all(url.as_str()).map_err(|e| e.to_string())?;
    Ok(proxy.no_proxy(reqwest::NoProxy::from_string(
        &full_bypass(settings).join(","),
    )))
}

/// A cached client that is rebuilt after the proxy setting changes. Clones handed out earlier
/// keep their connection pool, so streams in flight are never cut by a settings save.
pub struct SharedHttp {
    build: Option<fn() -> reqwest::Client>,
    slot: RwLock<Option<(u64, reqwest::Client)>>,
}

impl SharedHttp {
    /// Follows the app-wide proxy; `build` must start from [`client_builder`].
    pub const fn live(build: fn() -> reqwest::Client) -> Self {
        Self {
            build: Some(build),
            slot: RwLock::new(None),
        }
    }

    /// A fixed client (tests and fakes). Never rebuilt.
    pub fn fixed(client: reqwest::Client) -> Self {
        Self {
            build: None,
            slot: RwLock::new(Some((0, client))),
        }
    }

    pub fn get(&self) -> reqwest::Client {
        let Some(build) = self.build else {
            let slot = self.slot.read().unwrap_or_else(|e| e.into_inner());
            return slot.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
        };
        let current = generation();
        if let Some((built, client)) = &*self.slot.read().unwrap_or_else(|e| e.into_inner()) {
            if *built == current {
                return client.clone();
            }
        }
        let client = build();
        *self.slot.write().unwrap_or_else(|e| e.into_inner()) = Some((current, client.clone()));
        client
    }
}

impl From<reqwest::Client> for SharedHttp {
    fn from(client: reqwest::Client) -> Self {
        Self::fixed(client)
    }
}

// ---------------------------------------------------------------------------
// Child processes
// ---------------------------------------------------------------------------

/// The inherited proxy environment, captured before the first change so `system` restores it.
static ORIGINAL_ENV: OnceLock<Vec<(String, Option<std::ffi::OsString>)>> = OnceLock::new();

fn env_names() -> Vec<String> {
    let mut names: Vec<String> = PROXY_ENV.iter().map(|n| n.to_string()).collect();
    names.push("NO_PROXY".into());
    // Windows environment names are case-insensitive; Unix tools read both spellings.
    if !cfg!(windows) {
        let lower: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
        names.extend(lower);
    }
    names
}

/// Children inherit the process environment (the same pattern as PATH and LANG in `path_env`),
/// so one write covers CLI agents, MCP stdio servers, shell tools, the terminal and installers.
fn apply_child_env(settings: &NetworkProxySettings) {
    let original = ORIGINAL_ENV.get_or_init(|| {
        env_names()
            .into_iter()
            .map(|name| {
                let value = std::env::var_os(&name);
                (name, value)
            })
            .collect()
    });
    let set = |name: &str, value: Option<&str>| {
        let names = if cfg!(windows) {
            vec![name.to_string()]
        } else {
            vec![name.to_string(), name.to_ascii_lowercase()]
        };
        for name in names {
            match value {
                Some(value) => std::env::set_var(&name, value),
                None => std::env::remove_var(&name),
            }
        }
    };
    match settings.mode.as_str() {
        MODE_CUSTOM => {
            for name in PROXY_ENV {
                set(name, Some(&settings.url));
            }
            set("NO_PROXY", Some(&full_bypass(settings).join(",")));
        }
        MODE_NONE => {
            for name in PROXY_ENV {
                set(name, None);
            }
            restore(original, |name| name.eq_ignore_ascii_case("NO_PROXY"));
        }
        _ => restore(original, |_| true),
    }
}

fn restore(original: &[(String, Option<std::ffi::OsString>)], keep: impl Fn(&str) -> bool) {
    for (name, value) in original.iter().filter(|(name, _)| keep(name)) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

// ---------------------------------------------------------------------------
// Chromium (WebView2 and the HTML preview browser)
// ---------------------------------------------------------------------------

/// Chromium command-line flags for a setting. `system` adds nothing (Chromium's default).
/// Chromium flags cannot carry proxy credentials; an authenticating proxy prompts or fails.
fn chromium_args(settings: &NetworkProxySettings) -> Vec<String> {
    match settings.mode.as_str() {
        MODE_NONE => vec!["--no-proxy-server".into()],
        MODE_CUSTOM => {
            let Ok(url) = parse_url(settings.url.trim()) else {
                return Vec::new();
            };
            // Chromium's SOCKS5 already resolves names on the proxy side.
            let scheme = if url.scheme() == "socks5h" {
                "socks5"
            } else {
                url.scheme()
            };
            let host = url.host_str().unwrap_or_default();
            let server = match url.port() {
                Some(port) => format!("{scheme}://{host}:{port}"),
                None => format!("{scheme}://{host}"),
            };
            let mut bypass = full_bypass(settings);
            bypass.push("<-loopback>".into());
            vec![
                format!("--proxy-server={server}"),
                format!("--proxy-bypass-list={}", bypass.join(";")),
            ]
        }
        _ => Vec::new(),
    }
}

/// Proxy flags for a freshly started headless browser (follows the live setting).
pub fn browser_proxy_args() -> Vec<String> {
    chromium_args(&active())
}

/// WebView2 browser arguments for this process, or `None` to keep wry's defaults (system).
/// Frozen at first use: every WebView2 in one process must share identical arguments.
pub fn webview_browser_args() -> Option<String> {
    webview_browser_args_for(&active())
}

fn webview_browser_args_for(settings: &NetworkProxySettings) -> Option<String> {
    static FROZEN: OnceLock<Option<String>> = OnceLock::new();
    FROZEN
        .get_or_init(|| {
            let args = chromium_args(settings);
            // Setting browser args replaces wry's defaults, so restate them.
            (!args.is_empty()).then(|| {
                format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection {}",
                    args.join(" ")
                )
            })
        })
        .clone()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkProxyTestResult {
    pub latency_ms: u64,
    pub status: u16,
}

/// Probe a (possibly unsaved) setting with one request to an external endpoint.
#[tauri::command]
pub async fn network_proxy_test(
    settings: NetworkProxySettings,
) -> Result<NetworkProxyTestResult, String> {
    validate(&settings)?;
    let mut settings = settings;
    sanitize(&mut settings);
    let client = apply(reqwest::Client::builder(), &settings)
        .timeout(TEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Cannot create the test client: {e}"))?;
    let started = Instant::now();
    let response = client
        .get(TEST_URL)
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", describe(&e)))?;
    Ok(NetworkProxyTestResult {
        latency_ms: started.elapsed().as_millis() as u64,
        status: response.status().as_u16(),
    })
}

fn describe(error: &reqwest::Error) -> String {
    use std::error::Error as _;
    if error.is_timeout() {
        return "timed out".into();
    }
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        message = format!("{message}: {inner}");
        source = inner.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(url: &str, bypass: &str) -> NetworkProxySettings {
        NetworkProxySettings {
            mode: MODE_CUSTOM.into(),
            url: url.into(),
            bypass: bypass.into(),
        }
    }

    #[test]
    fn sanitize_keeps_valid_custom_and_resets_unusable_settings() {
        let mut valid = custom(" socks5h://u:p@127.0.0.1:1080 ", "a.com;b.com\n a.com");
        sanitize(&mut valid);
        assert_eq!(valid.mode, MODE_CUSTOM);
        assert_eq!(valid.url, "socks5h://u:p@127.0.0.1:1080");
        assert_eq!(valid.bypass, "a.com, b.com");

        for url in ["", "127.0.0.1:7890", "ftp://h:1", "http://h:1/path"] {
            let mut invalid = custom(url, "");
            assert!(validate(&invalid).is_err(), "{url}");
            sanitize(&mut invalid);
            assert_eq!(invalid.mode, MODE_SYSTEM, "{url}");
        }

        let mut unknown = NetworkProxySettings {
            mode: "bogus".into(),
            ..Default::default()
        };
        sanitize(&mut unknown);
        assert_eq!(unknown.mode, MODE_SYSTEM);
    }

    #[test]
    fn chromium_args_map_modes_and_always_bypass_loopback() {
        assert!(chromium_args(&NetworkProxySettings::default()).is_empty());
        let none = NetworkProxySettings {
            mode: MODE_NONE.into(),
            ..Default::default()
        };
        assert_eq!(chromium_args(&none), vec!["--no-proxy-server"]);
        let args = chromium_args(&custom("socks5h://user:secret@10.0.0.2:1080", "corp.local"));
        assert_eq!(args[0], "--proxy-server=socks5://10.0.0.2:1080");
        assert!(!args.join(" ").contains("secret"));
        assert_eq!(
            args[1],
            "--proxy-bypass-list=corp.local;localhost;127.0.0.1;::1;<-loopback>"
        );
    }

    /// A custom proxy must carry traffic for external hosts but never for loopback.
    #[tokio::test]
    async fn custom_proxy_routes_external_requests_and_bypasses_loopback() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn serve_once(listener: tokio::net::TcpListener, body: &'static str) -> String {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        }

        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let settings = custom(&proxy_url, "");
        let client = apply(reqwest::Client::builder(), &settings)
            .build()
            .unwrap();

        let proxied = tokio::spawn(serve_once(proxy, "via-proxy"));
        let body = client
            .get("http://example.invalid/ping")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "via-proxy");
        assert!(proxied
            .await
            .unwrap()
            .starts_with("GET http://example.invalid/ping"));

        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_url = format!("http://{}/local", local.local_addr().unwrap());
        let direct = tokio::spawn(serve_once(local, "direct"));
        let body = client
            .get(&local_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "direct");
        assert!(direct.await.unwrap().starts_with("GET /local"));
    }

    #[test]
    fn fixed_shared_client_is_never_rebuilt() {
        let shared = SharedHttp::fixed(reqwest::Client::new());
        GENERATION.fetch_add(1, Ordering::AcqRel);
        let _ = shared.get();
        assert_eq!(shared.slot.read().unwrap().as_ref().unwrap().0, 0);
    }
}
