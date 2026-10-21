//! Local-only Word task pane. No remote library access, no document write endpoints.
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

const PORT: u16 = if crate::app_data::is_dev() {
    23128
} else {
    23127
};
const ORIGIN: &str = if crate::app_data::is_dev() {
    "https://localhost:23128"
} else {
    "https://localhost:23127"
};
const HOST: &str = if crate::app_data::is_dev() {
    "localhost:23128"
} else {
    "localhost:23127"
};
const MANIFEST_NAME: &str = if crate::app_data::is_dev() {
    "kivio-study-dev.xml"
} else {
    "kivio-study.xml"
};
const DISPLAY_NAME: &str = if crate::app_data::is_dev() {
    "Kivio Study Dev"
} else {
    "Kivio Study"
};
const ADDIN_ID: &str = if crate::app_data::is_dev() {
    "c15d6e93-c16c-439d-9229-91851b37f206"
} else {
    "61c5c219-0386-4fea-8292-8e86cf23d27e"
};
struct Running {
    handle: axum_server::Handle,
    task: tokio::task::JoinHandle<()>,
    token: String,
}
static INSTALLING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RUNNING: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
fn running() -> &'static Mutex<Option<Running>> {
    RUNNING.get_or_init(|| Mutex::new(None))
}
fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("study-word"))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    running: bool,
    installed: bool,
    pairing_code: String,
}
#[derive(Clone)]
struct Server {
    library: PathBuf,
    token: Arc<String>,
    assets: PathBuf,
}
fn assets(app: &AppHandle) -> Result<PathBuf, String> {
    let bundled = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("word");
    if bundled.join("index.html").is_file() {
        return Ok(bundled);
    }
    #[cfg(debug_assertions)]
    {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/word");
        if dev.join("index.html").is_file() {
            return Ok(dev);
        }
    }
    Err("Word 插件资源缺失，请重新安装 Kivio。".into())
}
fn valid_origin(headers: &HeaderMap) -> bool {
    headers.get("host").and_then(|s| s.to_str().ok()) == Some(HOST)
        && headers.get("origin").map(|s| s == ORIGIN).unwrap_or(true)
}
async fn library(State(server): State<Server>, headers: HeaderMap) -> Response {
    if !valid_origin(&headers)
        || headers.get("authorization").and_then(|s| s.to_str().ok())
            != Some(&format!("Bearer {}", server.token))
    {
        return (
            StatusCode::UNAUTHORIZED,
            "请在 Study 中复制连接码，再连接。",
        )
            .into_response();
    }
    match tauri::async_runtime::spawn_blocking(move || super::references(&server.library)).await {
        Ok(Ok(items)) => ([("Cache-Control", "no-store")], Json(items)).into_response(),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "文献库读取失败，请回到 Study 检查保存状态。",
        )
            .into_response(),
    }
}
async fn static_file(State(server): State<Server>, headers: HeaderMap, uri: Uri) -> Response {
    if !valid_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let name = uri.path().trim_start_matches('/');
    let name = if name.is_empty() { "index.html" } else { name };
    if name.contains("..") || name.contains('\\') || name.contains('%') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mime = if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".png") {
        "image/png"
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(server.assets.join(name)).await {
        Ok(data) => (
            [
                ("Content-Type", mime),
                ("X-Content-Type-Options", "nosniff"),
                ("Referrer-Policy", "no-referrer"),
            ],
            data,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
fn prepare(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    if !dir.join("localhost.key").exists() || !dir.join("localhost.crt").exists() {
        let mut params =
            rcgen::CertificateParams::new(vec!["localhost".into()]).map_err(|e| e.to_string())?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Kivio Study Local");
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let key_pair = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
        let cert = params.self_signed(&key_pair).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("localhost.key"), key_pair.serialize_pem())
            .map_err(|e| e.to_string())?;
        std::fs::write(dir.join("localhost.crt"), cert.pem()).map_err(|e| e.to_string())?;
    }
    if !dir.join("token").exists() {
        std::fs::write(
            dir.join("token"),
            format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
async fn start(app: &AppHandle) -> Result<(), String> {
    let mut guard = running().lock().await;
    if guard.as_ref().is_some_and(|r| !r.task.is_finished()) {
        return Ok(());
    }
    let dir = root(app)?;
    let asset_dir = assets(app)?;
    prepare(&dir)?;
    let token = std::fs::read_to_string(dir.join("token")).map_err(|e| e.to_string())?;
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        dir.join("localhost.crt"),
        dir.join("localhost.key"),
    )
    .await
    .map_err(|e| e.to_string())?;
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, PORT))
        .map_err(|e| format!("Word 本机连接启动失败，端口 {PORT} 可能已被占用：{e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let router = Router::new()
        .route("/library", get(library))
        .fallback(static_file)
        .with_state(Server {
            library: super::super::directory(app)?,
            token: Arc::new(token.clone()),
            assets: asset_dir,
        });
    let handle = axum_server::Handle::new();
    let serving = axum_server::from_tcp_rustls(listener, tls).handle(handle.clone());
    let task = tokio::spawn(async move {
        if let Err(e) = serving.serve(router.into_make_service()).await {
            eprintln!("Study Word server stopped: {e}");
        }
    });
    *guard = Some(Running {
        handle,
        task,
        token,
    });
    Ok(())
}
#[tauri::command]
pub async fn reading_word_status(app: AppHandle) -> Result<Status, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    status(app).await
}
async fn status(app: AppHandle) -> Result<Status, String> {
    let dir = root(&app)?;
    let guard = running().lock().await;
    let active = guard.as_ref().filter(|r| !r.task.is_finished());
    Ok(Status {
        running: active.is_some(),
        installed: dir.join("installed").exists(),
        pairing_code: active.map(|r| r.token.clone()).unwrap_or_default(),
    })
}
#[tauri::command]
pub async fn reading_word_start(app: AppHandle) -> Result<Status, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    start_and_remember(&app).await?;
    status(app).await
}
async fn start_and_remember(app: &AppHandle) -> Result<(), String> {
    start(app).await?;
    std::fs::write(root(app)?.join("enabled"), "1").map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn reading_word_stop(app: AppHandle) -> Result<Status, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let dir = root(&app)?;
    if dir.join("enabled").exists() {
        std::fs::remove_file(dir.join("enabled")).map_err(|e| e.to_string())?;
    }
    if let Some(server) = running().lock().await.take() {
        server
            .handle
            .graceful_shutdown(Some(Duration::from_secs(1)));
        server.task.abort();
    }
    if dir.join("token").exists() {
        std::fs::remove_file(dir.join("token")).map_err(|e| e.to_string())?;
    }
    status(app).await
}
/// Activation never installs certificates or Office integration without user intent.
pub async fn resume(app: &AppHandle) -> Result<(), String> {
    if root(app)?.join("enabled").exists() {
        start(app).await?;
    }
    Ok(())
}
/// Extension disable preserves the user's Word preference for the next activation.
pub async fn suspend() {
    if let Some(server) = running().lock().await.take() {
        server.handle.shutdown();
        let _ = server.task.await;
    }
}

fn manifest() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="TaskPaneApp">
<Id>{ADDIN_ID}</Id><Version>1.0.0.0</Version><ProviderName>Kivio</ProviderName><DefaultLocale>zh-CN</DefaultLocale>
<DisplayName DefaultValue="{DISPLAY_NAME}"/><Description DefaultValue="从本机 Study 文献库插入引用并更新参考文献。"/>
<IconUrl DefaultValue="{ORIGIN}/icon.png"/><SupportUrl DefaultValue="https://github.com/ZMGID/kivio"/>
<AppDomains><AppDomain>{ORIGIN}</AppDomain></AppDomains><Hosts><Host Name="Document"/></Hosts>
<Requirements><Sets DefaultMinVersion="1.3"><Set Name="WordApi"/></Sets></Requirements>
<DefaultSettings><SourceLocation DefaultValue="{ORIGIN}/"/></DefaultSettings><Permissions>ReadWriteDocument</Permissions>
</OfficeApp>"#
    )
}
#[tauri::command]
pub async fn reading_word_install(app: AppHandle) -> Result<Status, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    install(&app, true).await?;
    start_and_remember(&app).await?;
    status(app).await
}
async fn install(app: &AppHandle, repair: bool) -> Result<(), String> {
    let _guard = INSTALLING.lock().await;
    let dir = root(app)?;
    let first_install = !dir.join("installed").exists();
    let trust_required = repair
        || first_install
        || !dir.join("localhost.crt").is_file()
        || !dir.join("localhost.key").is_file();
    assets(app)?;
    prepare(&dir)?;
    let manifest_path = dir.join(MANIFEST_NAME);
    std::fs::write(&manifest_path, manifest()).map_err(|e| e.to_string())?;
    let certificate = dir.join("localhost.crt");
    #[cfg(target_os = "macos")]
    {
        let home = app.path().home_dir().map_err(|e| e.to_string())?;
        let keychain = home.join("Library/Keychains/login.keychain-db");
        if trust_required {
            let result = tokio::process::Command::new("/usr/bin/security")
                .args(["add-trusted-cert", "-r", "trustRoot", "-p", "ssl", "-k"])
                .arg(keychain)
                .arg(&certificate)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            if !result.status.success() {
                return Err("本机证书尚未获得信任，请允许系统确认后重新安装。".into());
            }
        }
        let folder = home.join("Library/Containers/com.microsoft.Word/Data/Documents/wef");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        std::fs::copy(&manifest_path, folder.join(MANIFEST_NAME)).map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        if trust_required {
            let trust = tokio::process::Command::new("certutil")
                .args(["-user", "-addstore", "Root"])
                .arg(&certificate)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            if !trust.status.success() {
                return Err("本机证书安装失败，请允许系统确认后重试。".into());
            }
        }
        let result = tokio::process::Command::new("reg")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Office\16.0\WEF\Developer",
                "/v",
                ADDIN_ID,
                "/t",
                "REG_SZ",
                "/d",
            ])
            .arg(&manifest_path)
            .arg("/f")
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err("Word 插件注册失败，请重试。".into());
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (certificate, trust_required);
        return Err("Word 插件安装支持 macOS 和 Windows。".into());
    }
    // Preserve an explicit disconnect across future launches and upgrades.
    if first_install {
        std::fs::write(dir.join("enabled"), "1").map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join("installed"), "1").map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn https_library_requires_pairing_and_rejects_foreign_origins_and_trash() {
        let temporary = tempfile::tempdir().unwrap();
        let dir = temporary.path();
        prepare(dir).unwrap();
        let library_dir = dir.join("library");
        std::fs::create_dir_all(&library_dir).unwrap();
        let mut document = super::super::super::blank("Visible paper".into());
        let record_dir = library_dir.join(&document.id);
        std::fs::create_dir_all(&record_dir).unwrap();
        std::fs::write(
            record_dir.join("document.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        document.id = uuid::Uuid::new_v4().to_string();
        document.trashed_at = Some("2026-10-20".into());
        let record_dir = library_dir.join(&document.id);
        std::fs::create_dir_all(&record_dir).unwrap();
        std::fs::write(
            record_dir.join("document.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            dir.join("localhost.crt"),
            dir.join("localhost.key"),
        )
        .await
        .unwrap();
        let router = Router::new()
            .route("/library", get(library))
            .with_state(Server {
                library: library_dir,
                token: Arc::new("valid-pairing".into()),
                assets: dir.to_owned(),
            });
        let task = tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls)
                .serve(router.into_make_service())
                .await
                .unwrap()
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .tls_certs_only([reqwest::Certificate::from_pem(
                &std::fs::read(dir.join("localhost.crt")).unwrap(),
            )
            .unwrap()])
            .resolve("localhost", address)
            .build()
            .unwrap();
        let url = format!("https://localhost:{}/library", address.port());
        let request = || client.get(&url).header("host", HOST);
        assert_eq!(request().send().await.unwrap().status(), 401);
        assert_eq!(
            request()
                .bearer_auth("valid-pairing")
                .header("origin", "https://evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        let response = request()
            .bearer_auth("valid-pairing")
            .header("origin", ORIGIN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let items: serde_json::Value = response.json().await.unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["name"], "Visible paper");
        assert!(items[0].get("notes").is_none());
        task.abort();
    }
}
