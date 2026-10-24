// Manual remote backups. Credentials and transfer files never enter a backup.
use reqwest::{Client, Method, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
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
