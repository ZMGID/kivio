//! Bundled extension lifecycle owner. Preferences alone never authorize a command:
//! the live state is published only after activation and persistence both succeed.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::OnceLock};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock, RwLockReadGuard};

static CHANGE: Mutex<()> = Mutex::const_new(());
static ACTIVE: RwLock<BTreeSet<String>> = RwLock::const_new(BTreeSet::new());
static READY: OnceLock<()> = OnceLock::new();
#[derive(Default, Serialize, Deserialize)]
struct Preferences {
    #[serde(default)]
    enabled: BTreeSet<String>,
}
fn catalog() -> Vec<String> {
    serde_json::from_str(include_str!(concat!(
        env!("OUT_DIR"),
        "/extension_ids.json"
    )))
    .expect("build-validated extension catalog")
}
fn path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("extensions.json"))
}
fn read(app: &AppHandle) -> Result<Preferences, String> {
    match std::fs::read(path(app)?) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("扩展设置读取失败：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.to_string()),
    }
}
fn save(app: &AppHandle, enabled: BTreeSet<String>) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(&Preferences { enabled }).map_err(|e| e.to_string())?;
    crate::chat::storage::atomic_write(&path(app)?, &content, "extension preferences")
}
/// A command that starts background work holds this lease until its task is registered.
/// Disabling takes the write lock before stopping resources, excluding late starts.
pub struct CommandLease {
    _guard: RwLockReadGuard<'static, BTreeSet<String>>,
}
pub async fn command_lease(id: &str) -> Result<CommandLease, String> {
    let guard = ACTIVE.read().await;
    if !guard.contains(id) {
        return Err(format!("请先在扩展管理中启用 {id}。"));
    }
    Ok(CommandLease { _guard: guard })
}
pub fn is_active(id: &str) -> bool {
    ACTIVE.try_read().is_ok_and(|s| s.contains(id))
}

async fn initialize(app: &AppHandle) -> Result<(), String> {
    if READY.get().is_some() {
        return Ok(());
    }
    let prefs = read(app)?;
    if prefs.enabled.contains("study") {
        crate::reading::activate(app).await?;
    }
    *ACTIVE.write().await = prefs
        .enabled
        .into_iter()
        .filter(|id| catalog().contains(id))
        .collect();
    let _ = READY.set(());
    Ok(())
}
#[tauri::command]
pub async fn extensions_list(app: AppHandle) -> Result<Vec<String>, String> {
    let _change = CHANGE.lock().await;
    initialize(&app).await?;
    Ok(ACTIVE.read().await.iter().cloned().collect())
}
#[tauri::command]
pub async fn extensions_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    if !catalog().contains(&id) {
        return Err(format!("未知扩展：{id}"));
    }
    let _change = CHANGE.lock().await;
    initialize(&app).await?;
    let mut active = ACTIVE.write().await;
    if active.contains(&id) == enabled {
        return Ok(active.iter().cloned().collect());
    }
    let mut next = read(&app)?.enabled;
    if enabled {
        if id == "study" {
            crate::reading::activate(&app).await?;
        }
        next.insert(id.clone());
        if let Err(e) = save(&app, next) {
            if id == "study" {
                crate::reading::deactivate().await;
            }
            return Err(e);
        }
        active.insert(id);
    } else {
        // Persist first: if storage fails, keep the running extension intact.
        next.remove(&id);
        save(&app, next)?;
        if id == "study" {
            crate::reading::deactivate().await;
        }
        active.remove(&id);
    }
    let result: Vec<_> = active.iter().cloned().collect();
    let _ = app.emit("extensions-changed", &result);
    Ok(result)
}
pub fn resume(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = extensions_list(app).await {
            eprintln!("Extensions: {e}");
        }
    });
}
pub async fn shutdown() {
    let _change = CHANGE.lock().await;
    let mut active = ACTIVE.write().await;
    if active.remove("study") {
        crate::reading::deactivate().await;
    }
}

// Native packages are compiled into the trusted desktop binary; register them here.
pub fn owns_command(command: &str) -> bool {
    command.starts_with("reading_")
}
pub fn invoke(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    crate::reading::invoke(invoke)
}
