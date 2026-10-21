//! Study component installation owns its disk state and the background job.
//! Downloads are isolated from the user's Python and existing PDF2zh installations.
use crate::extension_sdk::NoConsoleWindow;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Manager};

pub(super) mod pandoc;

const PDF2ZH: &str = "2.9.0";
const UV: &str = "0.13.1";
const INSTALL_REVISION: &str = "pdf2zh-next-2.9.0-babeldoc-0.6.2-bitstring-4.3.1";
static JOB: Mutex<Option<Progress>> = Mutex::new(None);
static TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

/// Abort and drop the process tree before the desktop runtime exits.
pub async fn shutdown() {
    let task = TASK.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
    *JOB.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    acknowledged: bool,
    #[serde(default = "selected")]
    pdf2zh: bool,
    #[serde(default)]
    pandoc: bool,
}
fn selected() -> bool {
    true
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    component: &'static str,
    steps: u8,
    phase: String,
    step: u8,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}
impl Progress {
    fn new(component: &'static str, phase: &str, step: u8) -> Self {
        Self {
            component,
            steps: if component == "pandoc" { 3 } else { 5 },
            phase: phase.into(),
            step,
            downloaded: 0,
            total: None,
            error: None,
        }
    }
    fn running(&self) -> bool {
        self.phase != "ready" && self.phase != "error"
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    acknowledged: bool,
    pdf2zh: bool,
    installed: bool,
    version: &'static str,
    pandoc: bool,
    pandoc_installed: bool,
    pandoc_version: &'static str,
    pandoc_download_bytes: Option<u64>,
    progress: Option<Progress>,
}
pub(super) fn root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.join("study"))
        .map_err(|e| e.to_string())
}
fn preferences(dir: &Path) -> Result<Preferences, String> {
    match std::fs::read(dir.join("setup.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Study 配置文件无法读取：{e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences {
            pdf2zh: true,
            ..Default::default()
        }),
        Err(e) => Err(e.to_string()),
    }
}
fn save(dir: &Path, prefs: &Preferences) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    crate::extension_sdk::atomic_write(
        &dir.join("setup.json"),
        &serde_json::to_string(prefs).map_err(|e| e.to_string())?,
        "Study setup",
    )
}
pub(super) fn python(dir: &Path) -> PathBuf {
    dir.join("environment").join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    })
}
pub(super) fn installed(dir: &Path) -> bool {
    python(dir).is_file()
        && std::fs::read_to_string(dir.join("ready")).is_ok_and(|s| s == INSTALL_REVISION)
}
fn snapshot(dir: &Path) -> Result<Status, String> {
    let prefs = preferences(dir)?;
    Ok(Status {
        acknowledged: prefs.acknowledged,
        pdf2zh: prefs.pdf2zh,
        installed: installed(dir),
        version: PDF2ZH,
        pandoc: prefs.pandoc,
        pandoc_installed: pandoc::installed(dir),
        pandoc_version: pandoc::VERSION,
        pandoc_download_bytes: pandoc::download_size(),
        progress: JOB.lock().map_err(|e| e.to_string())?.clone(),
    })
}
fn progress(phase: &str, step: u8) {
    *JOB.lock().unwrap_or_else(|e| e.into_inner()) = Some(Progress::new("pdf2zh", phase, step));
}

#[tauri::command]
pub async fn reading_setup_status(app: AppHandle) -> Result<Status, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    snapshot(&root(&app)?)
}

/// Saving an unselected component never uninstalls existing files.
#[tauri::command]
pub async fn reading_setup_save(
    app: AppHandle,
    pdf2zh: bool,
    acknowledge: bool,
    pandoc: Option<bool>,
) -> Result<Status, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let dir = root(&app)?;
    let mut job = JOB.lock().map_err(|e| e.to_string())?;
    if job.as_ref().is_some_and(Progress::running) {
        return Err("正在配置，请等待完成后修改选项。".into());
    }
    let mut prefs = preferences(&dir)?;
    prefs.pdf2zh = pdf2zh;
    if let Some(selected) = pandoc {
        prefs.pandoc = selected;
    }
    if acknowledge {
        if pdf2zh && !installed(&dir) {
            return Err("PDF2zh 尚未配置完成，请重试或取消勾选。".into());
        }
        if prefs.pandoc && !pandoc::installed(&dir) {
            return Err("Pandoc 尚未配置完成，请重试或取消勾选。".into());
        }
        prefs.acknowledged = true;
    }
    save(&dir, &prefs)?;
    *job = None;
    drop(job);
    snapshot(&dir)
}

#[tauri::command]
pub async fn reading_setup_start(app: AppHandle, repair: Option<bool>) -> Result<Status, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let dir = root(&app)?;
    let (need_pdf2zh, need_pandoc);
    {
        let mut job = JOB.lock().map_err(|e| e.to_string())?;
        if job.as_ref().is_some_and(Progress::running) {
            drop(job);
            return snapshot(&dir);
        }
        let prefs = preferences(&dir)?;
        if !prefs.pdf2zh && !prefs.pandoc {
            return Err("请先选择要配置的组件。".into());
        }
        need_pdf2zh = prefs.pdf2zh && (repair.unwrap_or(false) || !installed(&dir));
        need_pandoc = prefs.pandoc && (repair.unwrap_or(false) || !pandoc::installed(&dir));
        *job = Some(Progress::new(
            if need_pdf2zh { "pdf2zh" } else { "pandoc" },
            if need_pdf2zh || need_pandoc {
                "download"
            } else {
                "ready"
            },
            1,
        ));
        if !need_pdf2zh && !need_pandoc {
            drop(job);
            return snapshot(&dir);
        }
    }
    let task_dir = dir.clone();
    let task = tauri::async_runtime::spawn(async move {
        let result = async {
            if need_pdf2zh {
                install_pdf2zh(&task_dir).await?;
            }
            if need_pandoc {
                pandoc::install(&task_dir).await?;
            }
            Ok::<(), String>(())
        }
        .await;
        let mut job = JOB.lock().unwrap_or_else(|e| e.into_inner());
        let mut finished = job
            .clone()
            .unwrap_or_else(|| Progress::new("pdf2zh", "error", 0));
        match result {
            Ok(()) => {
                finished.phase = "ready".into();
                finished.step = finished.steps;
            }
            Err(error) => {
                finished.phase = "error".into();
                finished.error = Some(error);
            }
        }
        *job = Some(finished);
    });
    *TASK.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
    snapshot(&dir)
}

fn cached_archive(path: &Path, checksum: &str) -> Option<Vec<u8>> {
    std::fs::read(path)
        .ok()
        .filter(|bytes| format!("{:x}", Sha256::digest(bytes)) == checksum)
}

/// Shared verified download policy; component code owns its artifact manifest and extraction.
async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    archive: &Path,
    checksum: &str,
) -> Result<Vec<u8>, String> {
    if let Some(bytes) = cached_archive(archive, checksum) {
        return Ok(bytes);
    }
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("组件文件超过预期大小".into());
        }
        if let Some(p) = JOB.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            p.downloaded = bytes.len() as u64;
            p.total = total;
        }
    }
    if format!("{:x}", Sha256::digest(&bytes)) != checksum {
        return Err("组件文件校验失败，请重试下载。".into());
    }
    std::fs::write(archive, &bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn uv_platform() -> Result<(&'static str, &'static str), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok((
            "macosx_11_0_arm64",
            "cfa719fbbd23f26713394dea2692b7f146aa1b5713ce5e776e878701214ea52c",
        )),
        ("macos", "x86_64") => Ok((
            "macosx_10_12_x86_64",
            "7e60223e3e9636a3734d6c11b3552aeed987c40f1ca2497a546af58f2f883d56",
        )),
        ("windows", "x86_64") => Ok((
            "win_amd64",
            "4f693cd4feb52f08f30474c4612f24be2050472609be65b9960b635f286da284",
        )),
        _ => Err("当前系统暂不支持自动安装 PDF2zh。可取消勾选后使用基础阅读。".into()),
    }
}
async fn download_uv(dir: &Path) -> Result<PathBuf, String> {
    let (platform, checksum) = uv_platform()?;
    let uv = dir.join(if cfg!(windows) { "uv.exe" } else { "uv" });
    // Verify cached executables against the previously verified wheel on every installation.
    let archive = dir.join("uv.whl");
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = if let Some(bytes) = cached_archive(&archive, checksum) {
        bytes
    } else {
        let metadata: serde_json::Value = client
            .get(format!("https://pypi.org/pypi/uv/{UV}/json"))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let name = format!("uv-{UV}-py3-none-{platform}.whl");
        let url = metadata["urls"]
            .as_array()
            .and_then(|items| items.iter().find(|v| v["filename"].as_str() == Some(&name)))
            .and_then(|v| v["url"].as_str())
            .ok_or("未找到适合当前系统的运行环境")?;
        let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
        if parsed.scheme() != "https" || parsed.host_str() != Some("files.pythonhosted.org") {
            return Err("运行环境下载地址校验失败".into());
        }
        download_verified(&client, parsed.as_str(), &archive, checksum).await?
    };
    let mut wheel = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let suffix = if cfg!(windows) {
        "/scripts/uv.exe"
    } else {
        "/scripts/uv"
    };
    let entry = (0..wheel.len())
        .find(|&i| {
            wheel
                .by_index(i)
                .is_ok_and(|f| f.name().is_ok_and(|name| name.ends_with(suffix)))
        })
        .ok_or("运行环境中缺少 uv 程序")?;
    let mut file = wheel.by_index(entry).map_err(|e| e.to_string())?;
    let mut executable = Vec::new();
    file.read_to_end(&mut executable)
        .map_err(|e| e.to_string())?;
    let temporary = dir.join("uv.tmp");
    std::fs::File::create(&temporary)
        .and_then(|mut f| f.write_all(&executable))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    if uv.exists() {
        std::fs::remove_file(&uv).map_err(|e| e.to_string())?;
    }
    std::fs::rename(temporary, &uv).map_err(|e| e.to_string())?;
    Ok(uv)
}
async fn run(dir: &Path, executable: &Path, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let log_path = dir.join("install.log");
    let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .current_dir(dir)
        .env("UV_PYTHON_INSTALL_DIR", dir.join("python"))
        .env("UV_CACHE_DIR", dir.join("cache"))
        .env("UV_PYTHON_PREFERENCE", "only-managed")
        .env("HOME", dir.join("home"))
        .env("USERPROFILE", dir.join("home"))
        .env("PYTHONNOUSERSITE", "1")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log))
        .kill_on_drop(true)
        .no_console_window();
    let mut wrapped = process_wrap::tokio::CommandWrap::from(command);
    #[cfg(unix)]
    wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        let mut flags = process_wrap::tokio::CreationFlags(Default::default());
        flags.0 .0 = crate::extension_sdk::CREATE_NO_WINDOW;
        wrapped.wrap(flags).wrap(process_wrap::tokio::JobObject);
    }
    wrapped.wrap(process_wrap::tokio::KillOnDrop);
    let mut child = wrapped
        .spawn()
        .map_err(|e| format!("无法启动配置程序：{e}"))?;
    let status = tokio::time::timeout(Duration::from_secs(1200), child.wait())
        .await
        .map_err(|_| "配置超时，请检查网络后重试。".to_string())?
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let text = std::fs::read_to_string(&log_path).unwrap_or_default();
        let tail: String = text
            .chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!("组件配置失败，请重试。\n{tail}"));
    }
    Ok(std::fs::read_to_string(log_path).unwrap_or_default())
}
async fn install_pdf2zh(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("home")).map_err(|e| e.to_string())?;
    // An old marker must never certify an interrupted repair.
    if dir.join("ready").exists() {
        std::fs::remove_file(dir.join("ready")).map_err(|e| e.to_string())?;
    }
    progress("download", 1);
    let uv = download_uv(dir).await?;
    progress("python", 2);
    run(
        dir,
        &uv,
        &["python".as_ref(), "install".as_ref(), "3.12".as_ref()],
    )
    .await?;
    let environment = dir.join("environment");
    if !python(dir).exists() {
        run(
            dir,
            &uv,
            &[
                "venv".as_ref(),
                "--python".as_ref(),
                "3.12".as_ref(),
                environment.as_os_str(),
            ],
        )
        .await?;
    }
    progress("packages", 3);
    let package = format!("pdf2zh-next=={PDF2ZH}");
    run(
        dir,
        &uv,
        &[
            "pip".as_ref(),
            "install".as_ref(),
            "--python".as_ref(),
            python(dir).as_os_str(),
            package.as_ref(),
            "babeldoc==0.6.2".as_ref(),
            // Bitstring 5 removed BitStream, which BabelDOC 0.6.2 imports.
            "bitstring==4.3.1".as_ref(),
        ],
    )
    .await?;
    progress("assets", 4);
    run(
        dir,
        &python(dir),
        &[
            "-c".as_ref(),
            "from babeldoc.assets.assets import warmup; warmup()".as_ref(),
        ],
    )
    .await?;
    progress("verify", 5);
    run(dir, &python(dir), &["-c".as_ref(), "from importlib.metadata import version; import pdf2zh_next.main; import babeldoc; assert version('pdf2zh-next') == '2.9.0'".as_ref()]).await?;
    crate::extension_sdk::atomic_write(
        &dir.join("ready"),
        INSTALL_REVISION,
        "Study component readiness",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_component_archives_are_rejected_after_tampering() {
        let archive =
            std::env::temp_dir().join(format!("kivio-component-{}", uuid::Uuid::new_v4()));
        let checksum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        std::fs::write(&archive, b"abc").unwrap();
        assert_eq!(cached_archive(&archive, checksum).unwrap(), b"abc");
        std::fs::write(&archive, b"abcd").unwrap();
        assert!(cached_archive(&archive, checksum).is_none());
        std::fs::remove_file(archive).unwrap();
    }

    #[test]
    fn reopening_preserves_opt_out_and_requires_both_a_version_marker_and_python() {
        let dir = std::env::temp_dir().join(format!("kivio-study-{}", uuid::Uuid::new_v4()));
        assert!(preferences(&dir).unwrap().pdf2zh);
        assert!(!preferences(&dir).unwrap().pandoc);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("setup.json"),
            r#"{"acknowledged":true,"pdf2zh":false}"#,
        )
        .unwrap();
        assert!(!preferences(&dir).unwrap().pandoc);
        assert!(!preferences(&dir).unwrap().pdf2zh);
        save(
            &dir,
            &Preferences {
                acknowledged: true,
                pdf2zh: false,
                pandoc: false,
            },
        )
        .unwrap();
        assert!(preferences(&dir).unwrap().acknowledged);
        assert!(!preferences(&dir).unwrap().pdf2zh);
        std::fs::write(dir.join("ready"), INSTALL_REVISION).unwrap();
        assert!(!installed(&dir));
        std::fs::create_dir_all(python(&dir).parent().unwrap()).unwrap();
        std::fs::write(python(&dir), b"python fixture").unwrap();
        assert!(installed(&dir));
        // A new selected component cannot borrow PDF2zh's readiness marker.
        save(
            &dir,
            &Preferences {
                acknowledged: true,
                pdf2zh: true,
                pandoc: true,
            },
        )
        .unwrap();
        let state = snapshot(&dir).unwrap();
        assert!(state.installed && state.pandoc && !state.pandoc_installed);
        assert_eq!(
            std::fs::read_to_string(dir.join("ready")).unwrap(),
            INSTALL_REVISION
        );
        std::fs::write(dir.join("ready"), "old-version").unwrap();
        assert!(!installed(&dir));
        std::fs::write(dir.join("setup.json"), b"broken").unwrap();
        assert!(preferences(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
