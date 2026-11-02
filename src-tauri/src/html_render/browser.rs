use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    ffi::OsString,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures::{FutureExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    sync::Semaphore,
    time::{timeout_at, Instant},
};

#[cfg(unix)]
use crate::proc::NoConsoleWindow;
#[cfg(windows)]
#[path = "browser_windows.rs"]
mod pipe_process;
use super::{MAX_HEIGHT, MIN_HEIGHT};
#[cfg(windows)]
use pipe_process::{spawn_browser, stop_browser};

const MANIFEST: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
const PAGE_URL: &str = "https://kivio-preview.invalid/render";
use crate::html_preview::PREVIEW_CSP;
const INSTALLING: &str = "The preview browser is installing; call again in a minute.";
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 1536 * 1024 * 1024;
const MAX_CONSOLE_MESSAGES: usize = 50;
const MAX_CONSOLE_CHARS: usize = 1000;
const MAX_CDP_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConsoleMessage {
    pub(crate) level: String,
    pub(crate) text: String,
}

pub(crate) struct Capture {
    pub(crate) png_base64: String,
    pub(crate) content_height: u32,
    pub(crate) captured_height: u32,
    pub(crate) console: Vec<ConsoleMessage>,
}

fn platform() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64" | "aarch64") => Ok("win64"),
        ("windows", "x86") => Ok("win32"),
        ("macos", "x86_64") => Ok("mac-x64"),
        ("macos", "aarch64") => Ok("mac-arm64"),
        ("linux", "x86_64") => Ok("linux64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        (os, arch) => Err(format!(
            "No system browser found, and Chrome for Testing does not support {os}/{arch}."
        )),
    }
}

// Roots and PATH are arguments so discovery order can be exercised without changing the host.
fn system_candidates(os: &str, roots: &[PathBuf], path: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    match os {
        "windows" => {
            for relative in [
                "Microsoft/Edge/Application/msedge.exe",
                "Google/Chrome/Application/chrome.exe",
                "Chromium/Application/chrome.exe",
                "BraveSoftware/Brave-Browser/Application/brave.exe",
            ] {
                candidates.extend(roots.iter().map(|root| root.join(relative)));
            }
        }
        "macos" => {
            for relative in [
                "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                "Google Chrome.app/Contents/MacOS/Google Chrome",
                "Chromium.app/Contents/MacOS/Chromium",
                "Brave Browser.app/Contents/MacOS/Brave Browser",
            ] {
                candidates.extend(roots.iter().map(|root| root.join(relative)));
            }
        }
        "linux" => {
            for name in [
                "microsoft-edge",
                "microsoft-edge-stable",
                "msedge",
                "google-chrome",
                "google-chrome-stable",
                "chromium",
                "chromium-browser",
                "brave-browser",
                "brave",
            ] {
                candidates.extend(path.iter().map(|dir| dir.join(name)));
            }
        }
        _ => {}
    }
    candidates
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

fn system_browser() -> Option<PathBuf> {
    let roots = match std::env::consts::OS {
        "windows" => ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect(),
        "macos" => {
            let mut roots = vec![PathBuf::from("/Applications")];
            if let Some(home) = std::env::var_os("HOME") {
                roots.push(PathBuf::from(home).join("Applications"));
            }
            roots
        }
        _ => Vec::new(),
    };
    let path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    system_candidates(std::env::consts::OS, &roots, &path)
        .into_iter()
        .find(|path| is_executable(path))
}

fn version_parts(version: &str) -> Option<[u32; 4]> {
    let mut parts = version.split('.');
    let mut result = [0; 4];
    for part in &mut result {
        let value = parts.next()?;
        if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *part = value.parse().ok()?;
    }
    if parts.next().is_some() {
        None
    } else {
        Some(result)
    }
}

fn installed_path(root: &Path, version: &str, platform: &str) -> PathBuf {
    let exe = if platform.starts_with("win") {
        "chrome-headless-shell.exe"
    } else {
        "chrome-headless-shell"
    };
    root.join(version)
        .join(format!("chrome-headless-shell-{platform}"))
        .join(exe)
}

fn installed_browser(root: &Path, platform: &str) -> Option<PathBuf> {
    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let version = entry.file_name().into_string().ok()?;
            let parts = version_parts(&version)?;
            let path = installed_path(root, &version, platform);
            is_executable(&path).then_some((parts, path))
        })
        .max_by_key(|(parts, _)| *parts)
        .map(|(_, path)| path)
}

enum InstallState {
    Installing,
    Failed(String),
}

static INSTALLS: LazyLock<Mutex<HashMap<PathBuf, InstallState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn claim_install(root: &Path, platform: &str) -> Result<Option<PathBuf>, String> {
    let mut states = INSTALLS
        .lock()
        .map_err(|_| "Preview browser installer unavailable".to_string())?;
    // The previous installer may have finished between the first scan and this lock.
    if let Some(browser) = installed_browser(root, platform) {
        return Ok(Some(browser));
    }
    match states.remove(root) {
        Some(InstallState::Installing) => {
            states.insert(root.to_owned(), InstallState::Installing);
            Err(INSTALLING.into())
        }
        Some(InstallState::Failed(error)) => Err(format!(
            "Preview browser installation failed: {error}. Call again to retry."
        )),
        None => {
            states.insert(root.to_owned(), InstallState::Installing);
            Ok(None)
        }
    }
}

pub(crate) async fn executable(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(browser) = system_browser() {
        return Ok(browser);
    }
    let platform = platform()?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot locate preview browser directory: {e}"))?
        .join("html-preview-browser");
    if let Some(browser) = installed_browser(&root, platform) {
        return Ok(browser);
    }
    if let Some(browser) = claim_install(&root, platform)? {
        return Ok(browser);
    }
    tokio::spawn(async move {
        let result = std::panic::AssertUnwindSafe(install_browser(&root, platform))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err("Browser installer stopped unexpectedly".into()));
        let mut states = INSTALLS.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(()) => {
                states.remove(&root);
            }
            Err(error) => {
                states.insert(root, InstallState::Failed(error));
            }
        }
    });
    Err(INSTALLING.into())
}

async fn install_browser(root: &Path, platform: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(MANIFEST)
        .send()
        .await
        .map_err(|e| format!("Download manifest: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download manifest: {e}"))?;
    let mut manifest = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Read manifest: {e}"))?;
        if manifest.len() + chunk.len() > 1024 * 1024 {
            return Err("Browser manifest exceeds 1 MiB".into());
        }
        manifest.extend_from_slice(&chunk);
    }
    let manifest: Value =
        serde_json::from_slice(&manifest).map_err(|e| format!("Invalid browser manifest: {e}"))?;
    let stable = &manifest["channels"]["Stable"];
    let version = stable["version"]
        .as_str()
        .filter(|v| version_parts(v).is_some())
        .ok_or("Invalid stable browser version")?;
    let url = stable["downloads"]["chrome-headless-shell"]
        .as_array()
        .and_then(|downloads| downloads.iter().find(|entry| entry["platform"] == platform))
        .and_then(|entry| entry["url"].as_str())
        .ok_or_else(|| format!("Chrome for Testing is unavailable for {platform}"))?;
    let expected = format!("https://storage.googleapis.com/chrome-for-testing-public/{version}/{platform}/chrome-headless-shell-{platform}.zip");
    if url != expected {
        return Err("Browser manifest contains an unexpected download URL".into());
    }
    if is_executable(&installed_path(root, version, platform)) {
        return Ok(());
    }
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|e| format!("Create browser directory: {e}"))?;
    let stage = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root)
        .map_err(|e| format!("Create browser staging directory: {e}"))?;
    let archive = stage.path().join("browser.zip");
    let mut output = tokio::fs::File::create(&archive)
        .await
        .map_err(|e| format!("Create browser archive: {e}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download browser: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download browser: {e}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err("Browser archive exceeds 512 MiB".into());
    }
    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Read browser archive: {e}"))?;
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or("Browser archive is too large")?;
        if downloaded > MAX_ARCHIVE_BYTES {
            return Err("Browser archive exceeds 512 MiB".into());
        }
        output
            .write_all(&chunk)
            .await
            .map_err(|e| format!("Save browser archive: {e}"))?;
    }
    output
        .flush()
        .await
        .map_err(|e| format!("Save browser archive: {e}"))?;
    drop(output);
    let unpacked = stage.path().join("unpacked");
    let unpack_target = unpacked.clone();
    let platform_owned = platform.to_owned();
    tokio::task::spawn_blocking(move || extract_archive(&archive, &unpack_target, &platform_owned))
        .await
        .map_err(|e| format!("Browser extraction task failed: {e}"))??;
    let staged_exe = installed_path(stage.path(), "unpacked", platform);
    if !is_executable(&staged_exe) {
        return Err("Browser archive did not contain the expected executable".into());
    }
    publish_install(root, version, platform, &unpacked)
}

fn publish_install(
    root: &Path,
    version: &str,
    platform: &str,
    unpacked: &Path,
) -> Result<(), String> {
    let destination = root.join(version);
    let installed_exe = installed_path(root, version, platform);
    match std::fs::rename(unpacked, &destination) {
        Ok(()) => Ok(()),
        Err(_) if is_executable(&installed_exe) => Ok(()),
        Err(error) => {
            // The single-flight installer owns this version; quarantine an incomplete cache.
            let obsolete = unpacked
                .parent()
                .ok_or("Invalid browser staging directory")?
                .join("obsolete");
            if let Err(quarantine_error) = std::fs::rename(&destination, &obsolete) {
                if is_executable(&installed_exe) {
                    return Ok(());
                }
                return Err(format!(
                    "Install browser: {error}; replace incomplete cache: {quarantine_error}"
                ));
            }
            match std::fs::rename(unpacked, &destination) {
                Ok(()) => Ok(()),
                Err(_) if is_executable(&installed_exe) => Ok(()),
                Err(error) => Err(format!("Install browser: {error}")),
            }
        }
    }
}

fn reserved_device(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or("").trim_end_matches(' ');
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
    {
        return true;
    }
    let prefix = stem.get(..3).unwrap_or("");
    let suffix = stem.get(3..).unwrap_or("");
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
}

fn archive_path(name: &str, platform: &str) -> Result<PathBuf, String> {
    if name
        .chars()
        .any(|c| matches!(c, '\\' | ':' | '\0' | '<' | '>' | '"' | '|' | '?' | '*'))
        || name
            .split('/')
            .any(|part| part.ends_with('.') || part.ends_with(' ') || reserved_device(part))
    {
        return Err("Unsafe browser archive path".into());
    }
    let path = Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("Unsafe browser archive path".into());
    }
    let expected = format!("chrome-headless-shell-{platform}");
    if path.components().next() != Some(Component::Normal(std::ffi::OsStr::new(&expected))) {
        return Err("Unexpected browser archive root".into());
    }
    Ok(path.to_owned())
}

fn extract_archive(archive: &Path, output: &Path, platform: &str) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("Open browser archive: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid browser archive: {e}"))?;
    if zip.len() > 10_000 {
        return Err("Browser archive has too many entries".into());
    }
    let mut remaining = MAX_UNPACKED_BYTES;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| format!("Read browser archive: {e}"))?;
        let name = entry
            .name()
            .map_err(|e| format!("Invalid browser archive path: {e}"))?;
        let path = archive_path(&name, platform)?;
        if entry.unix_mode().is_some_and(|mode| {
            let kind = mode & 0o170000;
            kind != 0 && kind != 0o100000 && kind != 0o040000
        }) {
            return Err("Browser archive contains a link or special file".into());
        }
        let dest = output.join(path);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("Extract browser directory: {e}"))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Extract browser directory: {e}"))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map_err(|e| format!("Extract browser file: {e}"))?;
        let copied = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut file)
            .map_err(|e| format!("Extract browser file: {e}"))?;
        remaining = remaining
            .checked_sub(copied)
            .ok_or("Extracted browser exceeds 1.5 GiB")?;
        file.flush()
            .map_err(|e| format!("Extract browser file: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = entry.unix_mode().unwrap_or(0o644) & 0o777;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode))
                .map_err(|e| format!("Set browser permissions: {e}"))?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exe = output
            .join(format!("chrome-headless-shell-{platform}"))
            .join("chrome-headless-shell");
        let metadata =
            std::fs::metadata(&exe).map_err(|e| format!("Browser executable missing: {e}"))?;
        std::fs::set_permissions(
            &exe,
            std::fs::Permissions::from_mode(metadata.permissions().mode() | 0o111),
        )
        .map_err(|e| format!("Set browser executable permissions: {e}"))?;
    }
    Ok(())
}

fn theme_css(dark: bool) -> String {
    let (
        scheme,
        background,
        card,
        muted,
        border,
        input,
        text,
        muted_text,
        accent,
        accent_soft,
        danger,
        warning,
        warning_text,
        success_text,
        info_text,
        charts,
    ) = if dark {
        (
            "dark",
            "#212121",
            "#262629",
            "#2a2a2d",
            "#3a3a40",
            "#4e4e56",
            "#f0f0f3",
            "#a1a1aa",
            "#5c8df7",
            "#2a3243",
            "#ef6a5a",
            "#e6a04a",
            "#fbbf24",
            "#34d399",
            "#60a5fa",
            ["#2dd4bf", "#fbbf24", "#c084fc", "#fb7185", "#a3e635"],
        )
    } else {
        (
            "light",
            "#fdfcfa",
            "#f9f8f6",
            "#f4f3f1",
            "#e2e1df",
            "#d2d1cf",
            "#1d1d1f",
            "#737373",
            "#2f6ff0",
            "#e6efff",
            "#c4341c",
            "#c47b16",
            "#92400e",
            "#047857",
            "#1d4ed8",
            ["#0d9488", "#d97706", "#9333ea", "#e11d48", "#65a30d"],
        )
    };
    format!(":root{{color-scheme:{scheme};--background:{background};--foreground:{text};--muted:{muted};--muted-foreground:{muted_text};--card:{card};--card-foreground:{text};--popover:{background};--popover-foreground:{text};--secondary:{muted};--secondary-foreground:{text};--border:{border};--input:{input};--ring:{accent};--primary:{accent};--primary-foreground:#fff;--accent:{accent};--accent-foreground:#fff;--destructive:{danger};--destructive-foreground:#fff;--warning:{warning};--warning-foreground:{warning_text};--success:#10b981;--success-foreground:{success_text};--info:#3b82f6;--info-foreground:{info_text};--code-background:{muted};--code-foreground:{text};--chart-1:{accent};--chart-2:{};--chart-3:{};--chart-4:{};--chart-5:{};--chart-6:{};--radius:0.625rem;--font-sans:system-ui,-apple-system,\"Segoe UI\",sans-serif;--font-mono:ui-monospace,Menlo,Consolas,monospace}}html,body{{margin:0;background:transparent}}body{{color:var(--foreground);font:14px/1.6 var(--font-sans);overflow-wrap:anywhere}}a{{color:var(--accent)}}::selection{{background:{accent_soft}}}", charts[0], charts[1], charts[2], charts[3], charts[4])
}

fn console_event(method: &str, params: &Value) -> Option<ConsoleMessage> {
    let (level, text) = match method {
        "Runtime.consoleAPICalled" => {
            let level = match params["type"].as_str()? {
                "warning" => "warning",
                "error" | "assert" => "error",
                "info" => "info",
                _ => "log",
            };
            let args = params["args"].as_array()?;
            let mut text = String::new();
            let mut remaining = MAX_CONSOLE_CHARS;
            for (index, arg) in args.iter().enumerate() {
                if remaining == 0 {
                    break;
                }
                if index != 0 {
                    text.push(' ');
                    remaining -= 1;
                }
                let fragment: Cow<'_, str> = match arg.get("value") {
                    Some(Value::String(value)) => Cow::Borrowed(value),
                    Some(value @ (Value::Number(_) | Value::Bool(_))) => {
                        Cow::Owned(value.to_string())
                    }
                    Some(Value::Null) => Cow::Borrowed("null"),
                    _ => Cow::Borrowed(
                        arg["description"]
                            .as_str()
                            .or_else(|| arg["type"].as_str())
                            .unwrap_or("undefined"),
                    ),
                };
                for ch in fragment.chars().take(remaining) {
                    text.push(ch);
                    remaining -= 1;
                }
            }
            (level, text)
        }
        "Runtime.exceptionThrown" => {
            let details = &params["exceptionDetails"];
            let text = details["exception"]["description"]
                .as_str()
                .or_else(|| details["text"].as_str())?;
            ("error", text.chars().take(MAX_CONSOLE_CHARS).collect())
        }
        _ => return None,
    };
    Some(ConsoleMessage {
        level: level.into(),
        text,
    })
}

struct Cdp<R, W> {
    reader: BufReader<R>,
    writer: W,
    next_id: u64,
    session: Option<String>,
    body: Option<String>,
    fulfill_id: Option<u64>,
    loaded: HashSet<String>,
    console: Vec<ConsoleMessage>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Cdp<R, W> {
    async fn send(&mut self, method: &str, params: Value, session: bool) -> Result<u64, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({"id": id, "method": method, "params": params});
        if session {
            message["sessionId"] = json!(self
                .session
                .as_deref()
                .ok_or("Browser session unavailable")?);
        }
        let mut bytes =
            serde_json::to_vec(&message).map_err(|e| format!("Encode browser command: {e}"))?;
        bytes.push(0);
        self.writer
            .write_all(&bytes)
            .await
            .map_err(|e| format!("Send browser command: {e}"))?;
        self.writer
            .flush()
            .await
            .map_err(|e| format!("Send browser command: {e}"))?;
        Ok(id)
    }

    async fn receive(&mut self) -> Result<Value, String> {
        let mut frame = Vec::new();
        loop {
            let chunk = self
                .reader
                .fill_buf()
                .await
                .map_err(|e| format!("Read browser response: {e}"))?;
            if chunk.is_empty() {
                return Err("Browser disconnected".into());
            }
            let end = chunk.iter().position(|byte| *byte == 0);
            let length = end.unwrap_or(chunk.len());
            if frame.len() + length > MAX_CDP_MESSAGE {
                return Err("Browser response exceeds 64 MiB".into());
            }
            frame.extend_from_slice(&chunk[..length]);
            self.reader.consume(length + usize::from(end.is_some()));
            if end.is_some() {
                break;
            }
        }
        let value: Value =
            serde_json::from_slice(&frame).map_err(|e| format!("Invalid browser response: {e}"))?;
        if value["id"].as_u64() == self.fulfill_id && self.fulfill_id.is_some() {
            self.fulfill_id = None;
            if !value["error"].is_null() {
                return Err(format!("Could not supply preview page: {}", value["error"]));
            }
        }
        if let Some(method) = value["method"].as_str() {
            let params = &value["params"];
            if method == "Fetch.requestPaused" {
                let id = params["requestId"]
                    .as_str()
                    .ok_or("Invalid paused browser request")?
                    .to_owned();
                let is_page = params["request"]["url"].as_str() == Some(PAGE_URL)
                    && params["resourceType"].as_str() == Some("Document");
                if is_page && self.body.is_some() {
                    let body = self.body.take().expect("checked above");
                    self.fulfill_id = Some(
                        self.send(
                            "Fetch.fulfillRequest",
                            json!({
                                "requestId": id, "responseCode": 200,
                                "responseHeaders": [
                                    {"name":"Content-Type", "value":"text/html; charset=utf-8"},
                                    {"name":"Content-Security-Policy", "value":PREVIEW_CSP},
                                    {"name":"Cache-Control", "value":"no-store"}
                                ], "body":body
                            }),
                            true,
                        )
                        .await?,
                    );
                } else {
                    self.send(
                        "Fetch.failRequest",
                        json!({"requestId":id,"errorReason":"AccessDenied"}),
                        true,
                    )
                    .await?;
                }
            } else if method == "Page.lifecycleEvent" && params["name"] == "load" {
                if let Some(loader) = params["loaderId"].as_str() {
                    self.loaded.insert(loader.to_owned());
                }
            } else if self.console.len() < MAX_CONSOLE_MESSAGES {
                if let Some(message) = console_event(method, params) {
                    self.console.push(message);
                }
            }
        }
        Ok(value)
    }

    async fn call(&mut self, method: &str, params: Value, session: bool) -> Result<Value, String> {
        let id = self.send(method, params, session).await?;
        loop {
            let mut response = self.receive().await?;
            if response["id"].as_u64() == Some(id) {
                if !response["error"].is_null() {
                    return Err(format!("Browser {method} failed: {}", response["error"]));
                }
                return Ok(response["result"].take());
            }
        }
    }
}

async fn page_capture<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    reader: R,
    writer: W,
    html: &str,
    width: u32,
    dark: bool,
) -> Result<Capture, String> {
    let mut cdp = Cdp {
        reader: BufReader::new(reader),
        writer,
        next_id: 0,
        session: None,
        body: Some(STANDARD.encode(html)),
        fulfill_id: None,
        loaded: HashSet::new(),
        console: Vec::new(),
    };
    let target = cdp
        .call("Target.createTarget", json!({"url":"about:blank"}), false)
        .await?;
    let target_id = target["targetId"]
        .as_str()
        .ok_or("Browser did not create a page")?;
    let attached = cdp
        .call(
            "Target.attachToTarget",
            json!({"targetId":target_id,"flatten":true}),
            false,
        )
        .await?;
    cdp.session = Some(
        attached["sessionId"]
            .as_str()
            .ok_or("Browser did not attach to the page")?
            .to_owned(),
    );
    cdp.call("Page.enable", json!({}), true).await?;
    cdp.call(
        "Page.setLifecycleEventsEnabled",
        json!({"enabled":true}),
        true,
    )
    .await?;
    cdp.call("Runtime.enable", json!({}), true).await?;
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width":width, "height":MIN_HEIGHT, "deviceScaleFactor":1, "mobile":false
        }),
        true,
    )
    .await?;
    cdp.call(
        "Emulation.setEmulatedMedia",
        json!({"features":[{
            "name":"prefers-color-scheme", "value":if dark { "dark" } else { "light" }
        }]}),
        true,
    )
    .await?;
    cdp.call(
        "Emulation.setDefaultBackgroundColorOverride",
        json!({"color":{"r":0,"g":0,"b":0,"a":0}}),
        true,
    )
    .await?;
    let css = serde_json::to_string(&theme_css(dark)).map_err(|e| e.to_string())?;
    let source = format!(
        r#"(() => {{ const css = {css}; const install = () => {{ if (!document.documentElement) return false; const style = document.createElement('style'); style.textContent = css; document.documentElement.prepend(style); return true; }}; if (!install()) {{ const observer = new MutationObserver(() => {{ if (install()) observer.disconnect(); }}); observer.observe(document, {{childList:true,subtree:true}}); }} }})()"#
    );
    cdp.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source":source}),
        true,
    )
    .await?;
    cdp.call(
        "Fetch.enable",
        json!({"patterns":[{"urlPattern":"*", "resourceType":"Document"}]}),
        true,
    )
    .await?;
    let navigation = cdp
        .call("Page.navigate", json!({"url":PAGE_URL}), true)
        .await?;
    if let Some(error) = navigation["errorText"].as_str() {
        return Err(format!("Could not load preview: {error}"));
    }
    let loader = navigation["loaderId"]
        .as_str()
        .ok_or("Browser did not load the preview")?
        .to_owned();
    while !cdp.loaded.contains(&loader) {
        cdp.receive().await?;
    }
    let settled = cdp.call("Runtime.evaluate", json!({
        "expression":"document.fonts.ready.then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true)))))",
        "awaitPromise":true, "returnByValue":true
    }), true).await?;
    if !settled["exceptionDetails"].is_null() {
        return Err(format!(
            "Preview did not settle: {}",
            settled["exceptionDetails"]
        ));
    }
    let measured = cdp.call("Runtime.evaluate", json!({
        "expression":"Math.max(document.body ? document.body.scrollHeight : 0, document.documentElement ? document.documentElement.scrollHeight : 0)",
        "returnByValue":true
    }), true).await?;
    let value = measured["result"]["value"]
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or("Could not measure preview height")?;
    let content_height = value.ceil().min(u32::MAX as f64) as u32;
    let captured_height = content_height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    let mut screenshot = cdp
        .call(
            "Page.captureScreenshot",
            json!({
                "format":"png", "captureBeyondViewport":true,
                "clip":{"x":0,"y":0,"width":width,"height":captured_height,"scale":1}
            }),
            true,
        )
        .await?;
    let png_base64 = match screenshot["data"].take() {
        Value::String(data) if !data.is_empty() => data,
        _ => return Err("Browser returned an empty screenshot".into()),
    };
    Ok(Capture {
        png_base64,
        content_height,
        captured_height,
        console: cdp.console,
    })
}

#[cfg(unix)]
struct BrowserChild {
    child: tokio::process::Child,
    group: Option<u32>,
}

#[cfg(unix)]
impl Drop for BrowserChild {
    fn drop(&mut self) {
        if let Some(group) = self.group.take() {
            unsafe {
                libc::kill(-(group as i32), libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
fn spawn_browser(
    exe: &Path,
    args: &[OsString],
) -> std::io::Result<(
    BrowserChild,
    tokio::net::UnixStream,
    tokio::net::UnixStream,
    tokio::process::ChildStderr,
)> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;

    let (parent_write, child_read) = UnixStream::pair()?;
    let (parent_read, child_write) = UnixStream::pair()?;
    parent_read.set_nonblocking(true)?;
    parent_write.set_nonblocking(true)?;
    let reader = tokio::net::UnixStream::from_std(parent_read)?;
    let writer = tokio::net::UnixStream::from_std(parent_write)?;
    let duplicate = |fd| {
        let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 5) };
        if copy < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { OwnedFd::from_raw_fd(copy) })
        }
    };
    let read_fd = duplicate(child_read.as_raw_fd())?;
    let write_fd = duplicate(child_write.as_raw_fd())?;
    let (read_raw, write_raw) = (read_fd.as_raw_fd(), write_fd.as_raw_fd());
    let mut command = tokio::process::Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .process_group(0);
    // Copy from descriptors above 4 so remapping cannot overwrite the other source.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(read_raw, 3) < 0 || libc::dup2(write_raw, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    let group = child.id();
    let mut browser = BrowserChild { child, group };
    let stderr = browser
        .child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("Browser stderr unavailable"))?;
    Ok((browser, reader, writer, stderr))
}

#[cfg(unix)]
async fn stop_browser(mut browser: BrowserChild) -> std::io::Result<()> {
    if let Some(group) = browser.group.take() {
        unsafe {
            libc::kill(-(group as i32), libc::SIGKILL);
        }
    }
    let _ = browser.child.start_kill();
    browser.child.wait().await.map(|_| ())
}

fn browser_slots() -> &'static Semaphore {
    static SLOTS: Semaphore = Semaphore::const_new(2);
    &SLOTS
}

pub(crate) async fn capture(
    exe: &Path,
    html: &str,
    width: u32,
    dark: bool,
) -> Result<Capture, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let permit = timeout_at(deadline, browser_slots().acquire())
        .await
        .map_err(|_| "Preview timed out waiting for a browser".to_string())?
        .map_err(|_| "Preview browser unavailable".to_string())?;
    let exe = exe.to_owned();
    let html = html.to_owned();
    // The detached worker owns cleanup even if the caller cancels or times out.
    let task = tokio::spawn(async move {
        let _permit = permit;
        run_capture(&exe, &html, width, dark, deadline).await
    });
    timeout_at(deadline, task)
        .await
        .map_err(|_| "HTML preview timed out after 15 seconds".to_string())?
        .map_err(|e| format!("Preview browser task failed: {e}"))?
}

async fn run_capture(
    exe: &Path,
    html: &str,
    width: u32,
    dark: bool,
    deadline: Instant,
) -> Result<Capture, String> {
    let profile = tempfile::Builder::new()
        .prefix("kivio-html-preview-")
        .tempdir()
        .map_err(|e| format!("Create browser profile: {e}"))?;
    let mut profile_arg = std::ffi::OsString::from("--user-data-dir=");
    profile_arg.push(profile.path());
    let mut args: Vec<OsString> = [
        "--headless=new",
        "--remote-debugging-pipe",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-extensions",
        "--disable-gpu",
        "--hide-scrollbars",
        "--mute-audio",
        "--block-new-web-contents",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.push(profile_arg);
    args.push("about:blank".into());
    let (child, reader, writer, mut stderr) =
        spawn_browser(exe, &args).map_err(|e| format!("Start preview browser: {e}"))?;
    let diagnostics = tokio::spawn(async move {
        let mut last = Vec::new();
        let mut chunk = [0u8; 1024];
        while let Ok(count) = stderr.read(&mut chunk).await {
            if count == 0 {
                break;
            }
            last.extend_from_slice(&chunk[..count]);
            if last.len() > 2048 {
                last.drain(..last.len() - 2048);
            }
        }
        let text = String::from_utf8_lossy(&last);
        let text = text.trim();
        let start = text
            .char_indices()
            .rev()
            .nth(499)
            .map_or(0, |(index, _)| index);
        text[start..].to_owned()
    });
    let result = timeout_at(deadline, page_capture(reader, writer, html, width, dark))
        .await
        .unwrap_or_else(|_| Err("HTML preview timed out after 15 seconds".into()));
    let waited = stop_browser(child)
        .await
        .map_err(|e| format!("Stop preview browser: {e}"));
    let diagnostics = diagnostics.await.unwrap_or_default();
    let removed = profile
        .close()
        .map_err(|e| format!("Remove preview browser profile: {e}"));
    waited?;
    removed?;
    result.map_err(|error| {
        if diagnostics.is_empty() {
            error
        } else {
            format!("{error}: {diagnostics}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"browser").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn discovery_prefers_browser_order_then_path_order_and_cached_versions() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        let chrome = a.join("google-chrome");
        let edge = b.join("microsoft-edge");
        executable_file(&chrome);
        executable_file(&edge);
        let candidates = system_candidates("linux", &[], &[a.clone(), b.clone()]);
        assert_eq!(
            candidates.into_iter().find(|p| is_executable(p)),
            Some(edge.clone())
        );
        executable_file(&a.join("microsoft-edge"));
        let candidates = system_candidates("linux", &[], &[a.clone(), b]);
        assert_eq!(
            candidates.into_iter().find(|p| is_executable(p)),
            Some(a.join("microsoft-edge"))
        );
        let old = installed_path(root.path(), "9.0.0.1", "linux64");
        let new = installed_path(root.path(), "10.0.0.1", "linux64");
        executable_file(&old);
        executable_file(&new);
        assert_eq!(installed_browser(root.path(), "linux64"), Some(new));
        assert!(version_parts("1.2.3.4/../bad").is_none());
    }

    #[test]
    fn platform_discovery_includes_expected_install_locations() {
        let root = PathBuf::from("injected-root");
        let windows = system_candidates("windows", &[root.clone()], &[]);
        assert_eq!(
            windows[0],
            root.join("Microsoft/Edge/Application/msedge.exe")
        );
        assert_eq!(
            windows[1],
            root.join("Google/Chrome/Application/chrome.exe")
        );
        let mac = system_candidates("macos", &[root.clone()], &[]);
        assert_eq!(
            mac[0],
            root.join("Microsoft Edge.app/Contents/MacOS/Microsoft Edge")
        );
        assert_eq!(
            mac[1],
            root.join("Google Chrome.app/Contents/MacOS/Google Chrome")
        );
    }

    #[test]
    fn installation_claims_are_single_flight_and_report_failures_before_retry() {
        let root = tempfile::tempdir().unwrap();
        let claims = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| scope.spawn(|| claim_install(root.path(), "linux64")))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            claims
                .iter()
                .filter(|claim| matches!(claim, Ok(None)))
                .count(),
            1
        );
        assert_eq!(
            claims
                .iter()
                .filter(|claim| matches!(claim, Err(error) if error == INSTALLING))
                .count(),
            15
        );
        INSTALLS.lock().unwrap().insert(
            root.path().to_owned(),
            InstallState::Failed("network unavailable".into()),
        );
        assert!(claim_install(root.path(), "linux64")
            .unwrap_err()
            .contains("network unavailable"));
        assert!(claim_install(root.path(), "linux64").unwrap().is_none());
        let installed = installed_path(root.path(), "1.2.3.4", "linux64");
        executable_file(&installed);
        assert_eq!(
            claim_install(root.path(), "linux64").unwrap(),
            Some(installed)
        );
        INSTALLS.lock().unwrap().remove(root.path());
    }

    #[test]
    fn installation_recovers_an_incomplete_nonempty_version() {
        let root = tempfile::tempdir().unwrap();
        let version = "1.2.3.4";
        let platform = "linux64";
        assert!(claim_install(root.path(), platform).unwrap().is_none());
        let destination = root.path().join(version);
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("leftover"), b"incomplete").unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        let unpacked = stage.path().join("unpacked");
        let staged_exe = installed_path(stage.path(), "unpacked", platform);
        executable_file(&staged_exe);

        let obsolete = stage.path().join("obsolete");
        std::fs::create_dir(&obsolete).unwrap();
        std::fs::write(obsolete.join("collision"), b"occupied").unwrap();
        assert!(publish_install(root.path(), version, platform, &unpacked).is_err());
        assert!(destination.join("leftover").exists());
        assert!(staged_exe.exists());
        std::fs::remove_dir_all(&obsolete).unwrap();

        publish_install(root.path(), version, platform, &unpacked).unwrap();
        let installed = installed_path(root.path(), version, platform);
        assert_eq!(std::fs::read(&installed).unwrap(), b"browser");
        assert!(!destination.join("leftover").exists());
        assert_eq!(
            installed_browser(root.path(), platform),
            Some(installed.clone())
        );
        assert_eq!(
            claim_install(root.path(), platform).unwrap(),
            Some(installed.clone())
        );

        let other = tempfile::tempdir_in(root.path()).unwrap();
        let other_exe = installed_path(other.path(), "unpacked", platform);
        executable_file(&other_exe);
        std::fs::write(&other_exe, b"replacement").unwrap();
        publish_install(
            root.path(),
            version,
            platform,
            &other.path().join("unpacked"),
        )
        .unwrap();
        assert_eq!(std::fs::read(installed).unwrap(), b"browser");
        assert!(other_exe.exists());
        INSTALLS.lock().unwrap().remove(root.path());
    }

    #[tokio::test]
    async fn cdp_pipe_frames_split_coalesced_messages_and_reports_truncation() {
        let (client, browser) = tokio::io::duplex(64);
        let (reader, writer) = tokio::io::split(client);
        let (browser_reader, mut browser_writer) = tokio::io::split(browser);
        let fake = tokio::spawn(async move {
            let mut reader = BufReader::new(browser_reader);
            let mut request = Vec::new();
            reader.read_until(0, &mut request).await.unwrap();
            assert_eq!(request.pop(), Some(0));
            let request: Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(request["method"], "Browser.getVersion");
            assert_eq!(request["params"]["text"], "before\0after");
            browser_writer
                .write_all(b"{\"method\":\"Runtime.consoleAPICalled\",")
                .await
                .unwrap();
            browser_writer.write_all(b"\"params\":{\"type\":\"log\",\"args\":[{\"value\":\"hello\"}]}}\0{\"id\":1,\"result\":{\"ok\":true}}\0{\"method\":\"Page.lifecycleEvent\",\"params\":{\"name\":\"load\",\"loaderId\":\"loader\"}}\0").await.unwrap();
            browser_writer.write_all(b"{\"id\":2").await.unwrap();
        });
        let mut cdp = Cdp {
            reader: BufReader::new(reader),
            writer,
            next_id: 0,
            session: None,
            body: None,
            fulfill_id: None,
            loaded: HashSet::new(),
            console: Vec::new(),
        };
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            cdp.call("Browser.getVersion", json!({"text":"before\0after"}), false),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(cdp.console[0].text, "hello");
        tokio::time::timeout(Duration::from_secs(5), cdp.receive())
            .await
            .unwrap()
            .unwrap();
        assert!(cdp.loaded.contains("loader"));
        assert!(tokio::time::timeout(Duration::from_secs(5), cdp.receive())
            .await
            .unwrap()
            .unwrap_err()
            .contains("disconnected"));
        fake.await.unwrap();
    }

    #[test]
    fn pipe_child_process() {
        use std::io::BufRead;
        let args: Vec<String> = std::env::args().collect();
        if !args.iter().any(|arg| arg == "--skip=kivio-test-pipe-") {
            return;
        }
        assert!(args
            .iter()
            .any(|arg| arg == "--skip=kivio-test-spaced argument\\\"quoted"));
        #[cfg(unix)]
        let (input, mut output) = {
            use std::os::fd::FromRawFd;
            unsafe { (std::fs::File::from_raw_fd(3), std::fs::File::from_raw_fd(4)) }
        };
        #[cfg(windows)]
        let (input, mut output) = {
            use std::os::windows::io::BorrowedHandle;
            extern "C" {
                fn _get_osfhandle(fd: i32) -> isize;
            }
            let read = unsafe { _get_osfhandle(3) };
            let write = unsafe { _get_osfhandle(4) };
            assert!(read >= 0 && write >= 0);
            let read = unsafe { BorrowedHandle::borrow_raw(read as *mut std::ffi::c_void) }
                .try_clone_to_owned()
                .unwrap();
            let write = unsafe { BorrowedHandle::borrow_raw(write as *mut std::ffi::c_void) }
                .try_clone_to_owned()
                .unwrap();
            (std::fs::File::from(read), std::fs::File::from(write))
        };
        let mut request = Vec::new();
        let mut input = std::io::BufReader::new(input);
        input.read_until(0, &mut request).unwrap();
        assert_eq!(request.pop(), Some(0));
        let request: Value = serde_json::from_slice(&request).unwrap();
        assert_eq!(request["method"], "Browser.getVersion");
        #[cfg(windows)]
        {
            use windows::Win32::{Foundation::HANDLE, System::Threading::SetEvent};
            let value = args
                .iter()
                .find_map(|arg| arg.strip_prefix("--skip=kivio-test-sentinel-"))
                .unwrap();
            let handle = HANDLE(value.parse::<usize>().unwrap() as *mut std::ffi::c_void);
            let _ = unsafe { SetEvent(handle) };
        }
        let response = json!({"id":request["id"],"result":{"inherited":true}});
        output.write_all(response.to_string().as_bytes()).unwrap();
        output.write_all(&[0]).unwrap();
        output.flush().unwrap();
        std::thread::sleep(Duration::from_secs(30));
    }

    #[tokio::test]
    async fn browser_pipe_process_inherits_only_its_endpoints_and_is_reaped() {
        let exe = std::env::current_exe().unwrap();
        let mut args: Vec<OsString> = [
            "--exact",
            "html_render::browser::tests::pipe_child_process",
            "--skip=kivio-test-spaced argument\\\"quoted",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        args.push("--skip=kivio-test-pipe-".into());
        #[cfg(windows)]
        let sentinel = {
            use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
            use windows::{
                core::PCWSTR,
                Win32::{
                    Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT},
                    System::Threading::CreateEventW,
                },
            };
            let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.unwrap();
            let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
            unsafe { SetHandleInformation(HANDLE(event.as_raw_handle()), 1, HANDLE_FLAG_INHERIT) }
                .unwrap();
            args.push(
                format!(
                    "--skip=kivio-test-sentinel-{}",
                    event.as_raw_handle() as usize
                )
                .into(),
            );
            event
        };
        let (child, reader, writer, _stderr) = spawn_browser(&exe, &args).unwrap();
        let mut cdp = Cdp {
            reader: BufReader::new(reader),
            writer,
            next_id: 0,
            session: None,
            body: None,
            fulfill_id: None,
            loaded: HashSet::new(),
            console: Vec::new(),
        };
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            cdp.call("Browser.getVersion", json!({}), false),
        )
        .await;
        let pending = match &response {
            Ok(Ok(_)) => match tokio::time::timeout(
                Duration::from_secs(5),
                cdp.send("Browser.getVersion", json!({}), false),
            )
            .await
            {
                Ok(Ok(_)) => tokio::time::timeout(Duration::from_millis(100), cdp.receive())
                    .await
                    .is_err(),
                _ => false,
            },
            _ => false,
        };
        let stopped = tokio::time::timeout(Duration::from_secs(5), stop_browser(child)).await;
        let response = response.expect("child did not answer").unwrap();
        assert_eq!(response["inherited"], true);
        stopped.expect("child was not reaped").unwrap();
        assert!(
            pending,
            "a stalled pipe read should time out before the child is stopped"
        );
        assert!(tokio::time::timeout(Duration::from_secs(5), cdp.receive())
            .await
            .unwrap()
            .unwrap_err()
            .contains("disconnected"));
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::{
                Foundation::{HANDLE, WAIT_TIMEOUT},
                System::Threading::WaitForSingleObject,
            };
            assert_eq!(
                unsafe { WaitForSingleObject(HANDLE(sentinel.as_raw_handle()), 0) },
                WAIT_TIMEOUT
            );
        }
    }

    #[test]
    fn archive_rejects_traversal_and_extracts_executable() {
        let root = tempfile::tempdir().unwrap();
        for bad in [
            "../escape",
            "/absolute",
            "chrome-headless-shell-linux64/../../escape",
            "chrome-headless-shell-linux64/.. /escape",
            "chrome-headless-shell-linux64/C:/escape",
            "chrome-headless-shell-linux64/..\\escape",
            "chrome-headless-shell-linux64/COM1.txt",
            "chrome-headless-shell-linux64/nul",
            "other/file",
        ] {
            assert!(archive_path(bad, "linux64").is_err(), "{bad}");
        }
        let zip_path = root.path().join("browser.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zip.start_file(
            "chrome-headless-shell-linux64/chrome-headless-shell",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"browser").unwrap();
        zip.finish().unwrap();
        let output = root.path().join("output");
        extract_archive(&zip_path, &output, "linux64").unwrap();
        assert!(is_executable(
            &output.join("chrome-headless-shell-linux64/chrome-headless-shell")
        ));

        let malicious = root.path().join("malicious.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&malicious).unwrap());
        zip.start_file(
            "chrome-headless-shell-linux64/../../escaped",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        assert!(extract_archive(&malicious, &root.path().join("bad-output"), "linux64").is_err());
        assert!(!root.path().join("escaped").exists());

        let linked = root.path().join("linked.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&linked).unwrap());
        zip.add_symlink(
            "chrome-headless-shell-linux64/link",
            "../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        assert!(extract_archive(&linked, &root.path().join("link-output"), "linux64").is_err());
    }

    #[test]
    fn console_and_theme_include_expected_values() {
        let log = console_event("Runtime.consoleAPICalled", &json!({"type":"warning","args":[{"type":"string","value":"hello"},{"type":"number","value":7}]})).unwrap();
        assert_eq!(
            (log.level.as_str(), log.text.as_str()),
            ("warning", "hello 7")
        );
        let long = console_event(
            "Runtime.consoleAPICalled",
            &json!({"type":"log","args":[{"value":"字".repeat(2000)},{"value":"later"}]}),
        )
        .unwrap();
        assert_eq!(long.text.chars().count(), MAX_CONSOLE_CHARS);
        let error = console_event(
            "Runtime.exceptionThrown",
            &json!({"exceptionDetails":{"exception":{"description":"Error: broken"}}}),
        )
        .unwrap();
        assert_eq!(
            (error.level.as_str(), error.text.as_str()),
            ("error", "Error: broken")
        );
        assert!(theme_css(true).contains("--foreground:#f0f0f3"));
        assert!(theme_css(false).contains("--foreground:#1d1d1f"));
    }

    #[tokio::test]
    #[ignore = "requires a locally installed Chromium-family browser"]
    async fn captures_real_browser_with_console_theme_transparency_and_height_limit() {
        let browser = system_browser().expect("install a Chromium-family browser to run this test");
        let html = "<!doctype html><html><body><div style='width:100px;height:3000px;background:var(--accent)'></div><script>console.log('preview ready',matchMedia('(prefers-color-scheme: dark)').matches)</script></body></html>";
        for (dark, accent) in [(true, [92, 141, 247, 255]), (false, [47, 111, 240, 255])] {
            let result = capture(&browser, html, 728, dark).await.unwrap();
            let png = STANDARD.decode(&result.png_base64).unwrap();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            let image = image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), (728, MAX_HEIGHT));
            assert_eq!(image.get_pixel(10, 10).0, accent);
            assert_eq!(image.get_pixel(200, 10).0[3], 0);
            assert!(result.content_height >= 3000);
            assert_eq!(result.captured_height, MAX_HEIGHT);
            assert!(result
                .console
                .iter()
                .any(|entry| entry.text == format!("preview ready {dark}")));
        }
    }
}
