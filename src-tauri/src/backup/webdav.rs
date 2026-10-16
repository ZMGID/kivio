//! Manual remote backups. Credentials and transfer files never enter a backup.
use crate::state::AppState;
use reqwest::{Client, Method, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use tokio::io::AsyncWriteExt;
use ts_rs::TS;

static TRANSFER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MAX_ARCHIVE: u64 = 129 * 1024 * 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    url: String,
    username: String,
    password: String,
    directory: String,
}
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WebDavConfig {
    pub url: String,
    pub username: String,
    pub directory: String,
    pub has_password: bool,
}
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WebDavConfigInput {
    pub url: String,
    pub username: String,
    pub directory: String,
    /// None keeps the saved password; an empty string explicitly clears it.
    pub password: Option<String>,
}
#[derive(Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum WebDavBackupKind {
    Configuration,
    Full,
}
impl WebDavBackupKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Configuration => "json",
            Self::Full => "kivio",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Full => "full",
        }
    }
}
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WebDavBackup {
    pub name: String,
    pub kind: WebDavBackupKind,
    pub size: u64,
    pub modified: String,
}
#[derive(Serialize, TS)]
pub struct WebDavDownload {
    pub id: String,
    pub path: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn network_error(e: reqwest::Error) -> String {
    format!("WebDAV: {}", e.without_url())
}
fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app.path().app_data_dir().map_err(err)?;
    // A sibling survives full restore and is outside the archived app-data tree.
    let name = root
        .file_name()
        .ok_or("Invalid app data directory")?
        .to_string_lossy();
    Ok(root.with_file_name(format!("{name}.webdav.json")))
}
fn load(path: &Path) -> Result<Config, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(err),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config {
            directory: "Kivio".into(),
            ..Config::default()
        }),
        Err(e) => Err(err(e)),
    }
}
fn view(config: Config) -> WebDavConfig {
    WebDavConfig {
        url: config.url,
        username: config.username,
        directory: config.directory,
        has_password: !config.password.is_empty(),
    }
}
fn resolve(input: WebDavConfigInput, previous: Config) -> Result<Config, String> {
    let config = Config {
        url: input.url.trim().trim_end_matches('/').into(),
        username: input.username.trim().into(),
        directory: input.directory.trim().trim_matches('/').into(),
        password: input.password.unwrap_or(previous.password),
    };
    Dav::new(&config)?;
    Ok(config)
}

struct Dav {
    client: Client,
    base: Url,
    directory: Url,
    username: String,
    password: String,
    parts: Vec<String>,
}
impl Dav {
    fn new(config: &Config) -> Result<Self, String> {
        let mut base =
            Url::parse(&config.url).map_err(|_| "请输入有效的 WebDAV 地址 / Invalid WebDAV URL")?;
        if !matches!(base.scheme(), "https" | "http")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(
                "WebDAV URL must use HTTP(S), without embedded credentials, query or fragment"
                    .into(),
            );
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let parts: Vec<String> = config.directory.split('/').map(str::to_owned).collect();
        if parts.iter().any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.contains(['\\', '\0'])
                || part.chars().any(char::is_control)
        }) {
            return Err("请输入有效的远程目录 / Invalid remote directory".into());
        }
        let mut directory = base.clone();
        {
            let mut segments = directory
                .path_segments_mut()
                .map_err(|_| "Invalid WebDAV path")?;
            segments.pop_if_empty();
            for part in &parts {
                segments.push(part);
            }
            segments.push("");
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .timeout(Duration::from_secs(3600))
            .build()
            .map_err(network_error)?;
        Ok(Self {
            client,
            base,
            directory,
            parts,
            username: config.username.clone(),
            password: config.password.clone(),
        })
    }
    fn request(&self, method: &str, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(
                Method::from_bytes(method.as_bytes()).expect("static method"),
                url,
            )
            .basic_auth(&self.username, Some(&self.password))
    }
    fn file(&self, name: &str) -> Result<Url, String> {
        if name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err("Invalid backup name".into());
        }
        let mut url = self.directory.clone();
        url.path_segments_mut()
            .map_err(|_| "Invalid path")?
            .pop_if_empty()
            .push(name);
        Ok(url)
    }
    async fn ensure_directory(&self) -> Result<(), String> {
        let mut url = self.base.clone();
        for part in &self.parts {
            url.path_segments_mut()
                .map_err(|_| "Invalid path")?
                .pop_if_empty()
                .push(part)
                .push("");
            let response = self
                .request("MKCOL", url.clone())
                .send()
                .await
                .map_err(network_error)?;
            if matches!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED | StatusCode::CONFLICT
            ) && self.collection_exists(&url).await?
            {
                continue;
            }
            checked(response)?;
        }
        Ok(())
    }
    async fn collection_exists(&self, url: &Url) -> Result<bool, String> {
        let response = self
            .request("PROPFIND", url.clone())
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(r#"<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#)
            .send()
            .await
            .map_err(network_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        let bytes = bounded_bytes(checked(response)?, 8 * 1024 * 1024).await?;
        Ok(parse_multistatus(&bytes)?
            .response
            .into_iter()
            .any(|entry| {
                url.join(&entry.href).is_ok_and(|href| href == *url)
                    && entry.propstat.iter().any(|props| {
                        props.status.split_whitespace().nth(1) == Some("200")
                            && props.prop.resourcetype.collection.is_some()
                    })
            }))
    }
    async fn delete(&self, name: &str) -> Result<(), String> {
        checked(
            self.request("DELETE", self.file(name)?)
                .send()
                .await
                .map_err(network_error)?,
        )?;
        Ok(())
    }
    async fn publish(&self, temporary: &str, name: &str) -> Result<(), String> {
        checked(
            self.request("MOVE", self.file(temporary)?)
                .header("Destination", self.file(name)?.as_str())
                .header("Overwrite", "F")
                .send()
                .await
                .map_err(network_error)?,
        )?;
        Ok(())
    }
    async fn list(&self) -> Result<Vec<WebDavBackup>, String> {
        let response = self.request("PROPFIND", self.directory.clone()).header("Depth", "1").header("Content-Type", "application/xml; charset=utf-8")
            .body(r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#).send().await.map_err(network_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        let bytes = bounded_bytes(checked(response)?, 8 * 1024 * 1024).await?;
        parse_listing(&bytes, &self.directory)
    }
    async fn upload(&self, path: &Path, name: &str) -> Result<(), String> {
        self.ensure_directory().await?;
        let temporary = format!("{name}.partial");
        let result = async {
            let file = tokio::fs::File::open(path).await.map_err(err)?;
            let size = file.metadata().await.map_err(err)?.len();
            if size > MAX_ARCHIVE {
                return Err("Backup exceeds size limit".into());
            }
            let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
            checked(
                self.request("PUT", self.file(&temporary)?)
                    .header("Content-Length", size)
                    .header("Content-Type", "application/octet-stream")
                    .header("If-None-Match", "*")
                    .body(body)
                    .send()
                    .await
                    .map_err(network_error)?,
            )?;
            self.publish(&temporary, name).await
        }
        .await;
        if result.is_err() {
            let _ = self.delete(&temporary).await;
        }
        result
    }
    async fn download(&self, name: &str, path: &Path) -> Result<(), String> {
        let kind = backup_kind(name).ok_or("Invalid Kivio backup name")?;
        let limit = match kind {
            WebDavBackupKind::Full => MAX_ARCHIVE,
            WebDavBackupKind::Configuration => 64 * 1024 * 1024,
        };
        let mut response = checked(
            self.request("GET", self.file(name)?)
                .send()
                .await
                .map_err(network_error)?,
        )?;
        if response.content_length().is_some_and(|size| size > limit) {
            return Err("Backup exceeds size limit".into());
        }
        let mut file = tokio::fs::File::create(path).await.map_err(err)?;
        let mut total = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            total += chunk.len() as u64;
            if total > limit {
                return Err("Backup exceeds size limit".into());
            }
            file.write_all(&chunk).await.map_err(err)?;
        }
        file.sync_all().await.map_err(err)
    }
}
fn checked(response: Response) -> Result<Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let hint = match status.as_u16() {
        401 | 403 => "认证或权限不足 / Check credentials and permissions",
        404 => "远程文件或目录不存在 / File or directory not found",
        409 => "目录冲突；请先在服务端创建目标目录，并确认上级目录与写入权限 / Create the directory on the server and check its parent and write permissions",
        413 => "备份超过服务器单文件大小限制 / Backup exceeds the server's file size limit",
        429 => "请求过于频繁，请稍后重试 / Too many requests; try again later",
        405 | 501 => "服务器不支持此 WebDAV 操作 / Unsupported WebDAV operation",
        507 => "远程存储空间不足 / Remote storage is full",
        300..=399 => "请填写重定向后的最终地址 / Use the final URL (redirect refused)",
        _ => "请求失败 / Request failed",
    };
    Err(format!("WebDAV HTTP {}: {hint}", status.as_u16()))
}
async fn bounded_bytes(mut response: Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > limit {
            return Err("WebDAV response exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn backup_kind(name: &str) -> Option<WebDavBackupKind> {
    if !name
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"-._".contains(&c))
    {
        return None;
    }
    if name.starts_with("kivio-full-") && name.ends_with(".kivio") {
        Some(WebDavBackupKind::Full)
    } else if name.starts_with("kivio-configuration-") && name.ends_with(".json") {
        Some(WebDavBackupKind::Configuration)
    } else {
        None
    }
}
#[derive(Deserialize)]
#[serde(rename = "multistatus")]
struct Listing {
    #[serde(default)]
    response: Vec<DavResponse>,
}
#[derive(Deserialize)]
struct DavResponse {
    href: String,
    #[serde(default)]
    propstat: Vec<PropStat>,
}
#[derive(Deserialize)]
struct PropStat {
    status: String,
    prop: Properties,
}
#[derive(Deserialize)]
struct Properties {
    #[serde(default)]
    getcontentlength: Option<String>,
    #[serde(default)]
    getlastmodified: String,
    #[serde(default)]
    resourcetype: ResourceType,
}
#[derive(Default, Deserialize)]
struct ResourceType {
    collection: Option<Collection>,
}
#[derive(Deserialize)]
struct Collection {}
fn parse_multistatus(bytes: &[u8]) -> Result<Listing, String> {
    let xml = std::str::from_utf8(bytes).map_err(err)?;
    // Serde ignores root names, so a successful HTTP login page must not look like an empty directory.
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event().map_err(err)? {
            quick_xml::events::Event::Start(element) | quick_xml::events::Event::Empty(element) => {
                if element.local_name().as_ref() != b"multistatus" {
                    return Err("Server did not return a WebDAV multistatus document".into());
                }
                break;
            }
            quick_xml::events::Event::Decl(_) | quick_xml::events::Event::Comment(_) => {}
            quick_xml::events::Event::Text(text) if text.iter().all(u8::is_ascii_whitespace) => {}
            _ => return Err("Invalid WebDAV listing".into()),
        }
    }

    quick_xml::de::from_str(xml).map_err(|e| format!("Invalid WebDAV listing: {e}"))
}
fn parse_listing(bytes: &[u8], directory: &Url) -> Result<Vec<WebDavBackup>, String> {
    let listing = parse_multistatus(bytes)?;
    let mut results = BTreeMap::new();
    for entry in listing.response {
        let Ok(url) = directory.join(&entry.href) else {
            continue;
        };
        let Some(name) = url
            .path_segments()
            .and_then(|mut parts| parts.next_back())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(kind) = backup_kind(&name) else {
            continue;
        };
        if url != directory.join(&name).map_err(err)? {
            continue;
        }
        for props in entry.propstat {
            if props.status.split_whitespace().nth(1) != Some("200")
                || props.prop.resourcetype.collection.is_some()
            {
                continue;
            }
            if let Some(size) = props
                .prop
                .getcontentlength
                .and_then(|value| value.trim().parse::<u64>().ok())
            {
                results.insert(
                    name.clone(),
                    WebDavBackup {
                        name: name.clone(),
                        kind,
                        size,
                        modified: props.prop.getlastmodified,
                    },
                );
                break;
            }
        }
    }
    let mut results: Vec<_> = results.into_values().collect();
    results.sort_by(|a, b| {
        let timestamp = |file: &WebDavBackup| {
            chrono::DateTime::parse_from_rfc2822(&file.modified)
                .map(|date| date.timestamp())
                .unwrap_or(0)
        };
        timestamp(b)
            .cmp(&timestamp(a))
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(results)
}
fn downloads() -> &'static Mutex<HashMap<String, tempfile::TempDir>> {
    static DOWNLOADS: OnceLock<Mutex<HashMap<String, tempfile::TempDir>>> = OnceLock::new();
    DOWNLOADS.get_or_init(Mutex::default)
}
#[tauri::command]
pub(crate) fn webdav_load(app: AppHandle) -> Result<WebDavConfig, String> {
    Ok(view(load(&config_path(&app)?)?))
}
#[tauri::command]
pub(crate) async fn webdav_save(
    app: AppHandle,
    input: WebDavConfigInput,
) -> Result<WebDavConfig, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let path = config_path(&app)?;
    let config = resolve(input, load(&path)?)?;
    crate::chat::storage::atomic_write(
        &path,
        &serde_json::to_string(&config).map_err(err)?,
        "WebDAV configuration",
    )?;
    Ok(view(config))
}
#[tauri::command]
pub(crate) async fn webdav_test(app: AppHandle, input: WebDavConfigInput) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let dav = Dav::new(&resolve(input, load(&config_path(&app)?)?)?)?;
    let temp = tempfile::tempdir().map_err(err)?;
    let path = temp.path().join("probe");
    std::fs::write(&path, b"Kivio WebDAV connection test").map_err(err)?;
    let name = format!(".kivio-probe-{}", uuid::Uuid::new_v4());
    dav.upload(&path, &name).await?;
    let result = async {
        let response = checked(
            dav.request("GET", dav.file(&name)?)
                .send()
                .await
                .map_err(network_error)?,
        )?;
        if bounded_bytes(response, 1024).await? != b"Kivio WebDAV connection test" {
            return Err("WebDAV read-back verification failed".to_string());
        }
        dav.list().await
    }
    .await;
    let cleanup = dav.delete(&name).await;
    result?;
    cleanup
}
#[tauri::command]
pub(crate) async fn webdav_list(app: AppHandle) -> Result<Vec<WebDavBackup>, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    Dav::new(&load(&config_path(&app)?)?)?.list().await
}
#[tauri::command]
pub(crate) async fn webdav_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: WebDavBackupKind,
    interface: BTreeMap<String, String>,
) -> Result<String, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    if super::backup_restore_pending(app.clone())? {
        return Err("请先重启完成恢复 / Restart to finish pending restore".into());
    }
    let dav = Dav::new(&load(&config_path(&app)?)?)?;
    let temporary = tempfile::tempdir().map_err(err)?;
    let name = format!(
        "kivio-{}-{}-{}.{}",
        kind.label(),
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        uuid::Uuid::new_v4(),
        kind.extension()
    );
    let path = temporary.path().join(&name);
    match kind {
        WebDavBackupKind::Full => {
            super::export_full_backup(app, state, path.to_string_lossy().into(), interface).await?
        }
        WebDavBackupKind::Configuration => {
            crate::commands::export_settings(app, state, path.to_string_lossy().into(), None)?
        }
    }
    dav.upload(&path, &name).await?;
    Ok(name)
}
#[tauri::command]
pub(crate) async fn webdav_download(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<WebDavDownload, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let temporary = tempfile::tempdir().map_err(err)?;
    backup_kind(&name).ok_or("Invalid Kivio backup name")?;
    let path = temporary.path().join(&name);
    Dav::new(&load(&config_path(&app)?)?)?
        .download(&name, &path)
        .await?;
    if matches!(backup_kind(&name), Some(WebDavBackupKind::Configuration)) {
        let recovery = app
            .path()
            .app_data_dir()
            .map_err(err)?
            .join(".backup-webdav-recovery");
        std::fs::create_dir_all(&recovery).map_err(err)?;
        let previous = recovery.join(format!("configuration-{}.json", uuid::Uuid::new_v4()));
        crate::commands::export_settings(app, state, previous.to_string_lossy().into(), None)?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    downloads()
        .lock()
        .map_err(err)?
        .insert(id.clone(), temporary);
    Ok(WebDavDownload {
        id,
        path: path.to_string_lossy().into(),
    })
}
#[tauri::command]
pub(crate) fn webdav_release(id: String) -> Result<(), String> {
    downloads().lock().map_err(err)?.remove(&id);
    Ok(())
}
#[tauri::command]
pub(crate) async fn webdav_delete(app: AppHandle, name: String) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    backup_kind(&name).ok_or("Invalid Kivio backup name")?;
    Dav::new(&load(&config_path(&app)?)?)?.delete(&name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // Real HTTP boundary; responses model server behavior, not the backup implementation.
    fn server(
        script: Vec<(&'static str, &'static str, u16, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut bodies = vec![];
            for (method, path, status, body) in script {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(socket) => break socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "missing {method} {path}"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = vec![];
                let header_end = loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(index) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                assert_eq!(
                    headers.lines().next().unwrap(),
                    format!("{method} {path} HTTP/1.1")
                );
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|s| s.parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                bodies.push(bytes[header_end..].to_vec());
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            bodies
        });
        (url, thread)
    }
    fn config(url: String) -> Config {
        Config {
            url,
            username: "user".into(),
            password: "secret".into(),
            directory: "Kivio".into(),
        }
    }
    #[tokio::test]
    async fn remote_backup_round_trip_publishes_then_lists_downloads_and_deletes() {
        let name = "kivio-configuration-20261016-test.json";
        let xml = format!(
            r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/Kivio/{name}</D:href><D:propstat><D:prop><D:getcontentlength>6</D:getcontentlength><D:getlastmodified>Fri, 16 Oct 2026 12:00:00 GMT</D:getlastmodified><D:resourcetype/></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#
        );
        let (url, thread) = server(vec![
            ("MKCOL", "/Kivio/", 405, "".into()),
            ("PROPFIND", "/Kivio/", 207, r#"<multistatus xmlns="DAV:"><response><href>/Kivio/</href><propstat><prop><resourcetype><collection/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#.into()),
            (
                "PUT",
                "/Kivio/kivio-configuration-20261016-test.json.partial",
                201,
                "".into(),
            ),
            (
                "MOVE",
                "/Kivio/kivio-configuration-20261016-test.json.partial",
                201,
                "".into(),
            ),
            ("PROPFIND", "/Kivio/", 207, xml),
            (
                "GET",
                "/Kivio/kivio-configuration-20261016-test.json",
                200,
                "backup".into(),
            ),
            (
                "DELETE",
                "/Kivio/kivio-configuration-20261016-test.json",
                204,
                "".into(),
            ),
        ]);
        let dav = Dav::new(&config(url)).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::write(&source, b"backup").unwrap();
        dav.upload(&source, name).await.unwrap();
        let files = dav.list().await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, name);
        assert_eq!(files[0].size, 6);
        let target = temp.path().join("download");
        dav.download(&files[0].name, &target).await.unwrap();
        assert_eq!(std::fs::read(target).unwrap(), b"backup");
        dav.delete(name).await.unwrap();
        assert_eq!(thread.join().unwrap()[2], b"backup");
    }
    #[tokio::test]
    async fn failed_publish_cleans_partial_file_and_reports_failure() {
        let (url, thread) = server(vec![
            ("MKCOL", "/Kivio/", 201, "".into()),
            (
                "PUT",
                "/Kivio/kivio-full-test.kivio.partial",
                201,
                "".into(),
            ),
            (
                "MOVE",
                "/Kivio/kivio-full-test.kivio.partial",
                507,
                "".into(),
            ),
            (
                "DELETE",
                "/Kivio/kivio-full-test.kivio.partial",
                204,
                "".into(),
            ),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::write(&source, b"backup").unwrap();
        let error = Dav::new(&config(url))
            .unwrap()
            .upload(&source, "kivio-full-test.kivio")
            .await
            .unwrap_err();
        assert!(error.contains("507"));
        thread.join().unwrap();
    }
    #[test]
    fn listing_rejects_foreign_paths_collections_partial_files_and_failed_properties() {
        let response = |href: &str, status: &str, resource: &str| {
            format!(
                r#"<response><href>{href}</href><propstat><prop><getcontentlength>9</getcontentlength><resourcetype>{resource}</resourcetype></prop><status>HTTP/1.1 {status}</status></propstat></response>"#
            )
        };
        let xml = format!(
            "<multistatus xmlns=\"DAV:\">{}{}</multistatus>",
            [
                "https://evil.test/Kivio/kivio-full-a.kivio",
                "/other/kivio-full-b.kivio",
                "/Kivio/kivio-full-c.kivio.partial"
            ]
            .map(|href| response(href, "200 OK", ""))
            .join(""),
            [
                response("/Kivio/kivio-full-dir.kivio", "200 OK", "<collection/>"),
                response("/Kivio/kivio-full-missing.kivio", "404 Not Found", ""),
                response("/Kivio/kivio-full-good.kivio", "200 OK", "")
            ]
            .join("")
        );
        let files = parse_listing(
            xml.as_bytes(),
            &Url::parse("https://dav.test/Kivio/").unwrap(),
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "kivio-full-good.kivio");
        assert!(backup_kind("../../kivio-full-good.kivio").is_none());
        assert!(parse_listing(
            b"<html>login</html>",
            &Url::parse("https://dav.test/Kivio/").unwrap()
        )
        .is_err());
    }
    #[test]
    fn listing_accepts_empty_unsupported_properties_on_collections() {
        let xml = br#"<d:multistatus xmlns:d="DAV:">
          <d:response><d:href>/Kivio/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
          <d:response><d:href>/Kivio/kivio-full-ok.kivio</d:href><d:propstat><d:prop><d:getcontentlength>42</d:getcontentlength><d:getlastmodified>Fri, 16 Oct 2026 12:00:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let files = parse_listing(xml, &Url::parse("https://dav.test/Kivio/").unwrap()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 42);
    }

    #[tokio::test]
    async fn conflicting_mkcol_can_reuse_a_verified_existing_collection() {
        let xml = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/Kivio/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let (url, thread) = server(vec![
            ("MKCOL", "/Kivio/", 409, "".into()),
            ("PROPFIND", "/Kivio/", 207, xml.into()),
        ]);
        Dav::new(&config(url))
            .unwrap()
            .ensure_directory()
            .await
            .unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn saved_password_is_preserved_without_exposing_it_and_explicit_clear_works() {
        let input = |password| WebDavConfigInput {
            url: "https://dav.test".into(),
            username: "user".into(),
            directory: "Kivio".into(),
            password,
        };
        let saved = resolve(input(None), config("https://dav.test".into())).unwrap();
        assert_eq!(saved.password, "secret");
        let view = serde_json::to_value(view(saved)).unwrap();
        assert_eq!(view["hasPassword"], true);
        assert!(view.get("password").is_none());
        let cleared = resolve(input(Some("".into())), config("https://dav.test".into())).unwrap();
        assert!(cleared.password.is_empty());
        let mut invalid = input(None);
        invalid.directory = "../outside".into();
        assert!(resolve(invalid, Config::default()).is_err());
    }
}
