//! Portable application backups. Restoration is staged and committed before any
//! stores, workers or windows start; live stores never observe a partial restore.
use crate::{settings::Settings, state::AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, State};

static OPERATION: Mutex<()> = Mutex::new(());
const VERSION: u32 = 1;
const MAX_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 500_000;

#[derive(Serialize, Deserialize)]
struct Manifest {
    app: String,
    version: u32,
    kind: String,
    roots: Vec<BackupRoot>,
    files: BTreeMap<String, Entry>,
}
#[derive(Serialize, Deserialize)]
struct BackupRoot {
    source: String,
    archive: String,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Entry {
    bytes: u64,
    sha256: String,
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn excluded(name: &str) -> bool {
    matches!(
        name,
        "WebKit" | "EBWebView" | "WebView2" | "Cache" | "cache" | "logs" | "Crashpad"
    ) || name.ends_with("-shm")
        || name.starts_with(".backup-")
}
fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(error)?;
    }
    Ok(())
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    crate::chat::storage::atomic_write(
        path,
        &serde_json::to_string(value).map_err(error)?,
        "backup",
    )
}
fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)
}
fn digest(path: &Path) -> Result<Entry, String> {
    let mut file = File::open(path).map_err(error)?;
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(error)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        bytes += n as u64;
    }
    Ok(Entry {
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}
fn collect(root: &Path, path: &Path, out: &mut Vec<PathBuf>, app_data: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if metadata.file_type().is_symlink() {
        let target = path.canonicalize().map_err(error)?;
        let boundary = root.canonicalize().map_err(error)?;
        if !target.starts_with(&boundary)
            || path
                .ancestors()
                .skip(1)
                .take_while(|ancestor| ancestor.starts_with(root))
                .any(|ancestor| ancestor.canonicalize().ok().as_ref() == Some(&target))
        {
            return Err(format!(
                "符号链接指向备份目录之外或形成循环：{}",
                path.display()
            ));
        }
    }
    // Materialize internal links so restoring also works on Windows without
    // symlink privileges. Links escaping the selected roots are never followed.
    let metadata = fs::metadata(path).map_err(error)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(error)? {
            let entry = entry.map_err(error)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if app_data && ((path == root && excluded(&name)) || name.ends_with("-shm")) {
                continue;
            }
            collect(root, &entry.path(), out, app_data)?;
        }
    } else if metadata.is_file() {
        out.push(path.strip_prefix(root).map_err(error)?.to_path_buf());
        if out.len() > MAX_FILES {
            return Err("备份文件数量超过上限".into());
        }
    } else {
        return Err(format!("无法备份特殊文件：{}", path.display()));
    }
    Ok(())
}
fn files(root: &Path, app_data: bool) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    if root.exists() {
        collect(root, root, &mut paths, app_data)?;
    }
    paths.sort();
    Ok(paths)
}
fn archive_name(prefix: &str, path: &Path) -> Result<String, String> {
    let mut names = vec![prefix.to_string()];
    for part in path.components() {
        let Component::Normal(part) = part else {
            return Err("无效备份路径".into());
        };
        let part = part.to_str().ok_or("文件名不是 UTF-8")?;
        if part.contains(['\\', ':']) {
            return Err(format!("文件名不能跨平台恢复：{part}"));
        }
        names.push(part.into());
    }
    Ok(names.join("/"))
}
fn portable_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.contains(['\\', ':', '\0'])
        || name.starts_with('/')
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err("备份包含不安全的文件路径".into());
    }
    Ok(name.split('/').collect())
}

fn export_to(
    root: &Path,
    workspace: &Path,
    destination: &Path,
    ui: &BTreeMap<String, String>,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or("请选择备份保存目录")?
        .canonicalize()
        .map_err(error)?;
    let destination = parent.join(destination.file_name().ok_or("备份文件名为空")?);
    let mut roots = vec![BackupRoot {
        source: root.to_string_lossy().into(),
        archive: "app".into(),
    }];
    if !workspace.is_absolute() {
        return Err("工作目录必须是绝对路径".into());
    }
    if !workspace.starts_with(root) {
        roots.push(BackupRoot {
            source: workspace.to_string_lossy().into(),
            archive: "work/0".into(),
        });
    }
    // Project folders are user-owned; capture their files without restoring over
    // any existing project on the target machine.
    let projects = root.join("conversations/projects.json");
    if projects.exists() {
        if let Some(projects) = read_json(&projects)?
            .get("projects")
            .and_then(Value::as_array)
        {
            for project in projects {
                if let Some(path) = project
                    .get("root_path")
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                {
                    let path = PathBuf::from(path);
                    if !path.is_dir() {
                        return Err(format!("项目目录不存在，无法完全备份：{}", path.display()));
                    }
                    if !roots.iter().any(|r| path.starts_with(&r.source)) {
                        roots.push(BackupRoot {
                            source: path.to_string_lossy().into(),
                            archive: format!("work/{}", roots.len()),
                        });
                    }
                }
            }
        }
    }
    for r in &roots {
        let source = if Path::new(&r.source).exists() {
            Path::new(&r.source).canonicalize().map_err(error)?
        } else {
            PathBuf::from(&r.source)
        };
        if destination.starts_with(source) {
            return Err("请将备份保存到 Kivio 数据目录和项目/工作目录之外".into());
        }
    }
    let temp = parent.join(format!(".backup-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let file = File::create(&temp).map_err(error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(error)?;
        }
        let mut zip = zip::ZipWriter::new(file);
        let mut manifest = Manifest {
            app: "kivio".into(),
            kind: "full-backup".into(),
            version: VERSION,
            roots,
            files: BTreeMap::new(),
        };
        let mut total = 0u64;
        for r in &manifest.roots {
            let source = Path::new(&r.source);
            for path in files(source, r.archive == "app")? {
                let name = archive_name(&r.archive, &path)?;
                let full = source.join(&path);
                let before = digest(&full)?;
                total = total.checked_add(before.bytes).ok_or("备份过大")?;
                if total > MAX_BYTES {
                    return Err("备份超过 128 GiB 上限".into());
                }
                let mut options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .large_file(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    options = options.unix_permissions(
                        fs::metadata(&full).map_err(error)?.permissions().mode() & 0o777,
                    );
                }
                zip.start_file(&name, options).map_err(error)?;
                let mut input = File::open(&full).map_err(error)?;
                let mut copied_hash = Sha256::new();
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    let count = input.read(&mut buffer).map_err(error)?;
                    if count == 0 {
                        break;
                    }
                    zip.write_all(&buffer[..count]).map_err(error)?;
                    copied_hash.update(&buffer[..count]);
                }
                if format!("{:x}", copied_hash.finalize()) != before.sha256
                    || digest(&full)? != before
                {
                    return Err("备份期间数据发生变化，请停止任务后重试".into());
                }
                manifest.files.insert(name, before);
                if manifest.files.len() > MAX_FILES {
                    return Err("备份文件数量超过上限".into());
                }
            }
        }
        // Detect additions, deletions and updates across the entire snapshot.
        let mut verified = BTreeMap::new();
        for r in &manifest.roots {
            for path in files(Path::new(&r.source), r.archive == "app")? {
                verified.insert(
                    archive_name(&r.archive, &path)?,
                    digest(&Path::new(&r.source).join(path))?,
                );
            }
        }
        if verified != manifest.files {
            return Err("备份期间数据发生变化，请停止任务后重试".into());
        }
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o600);
        zip.start_file("manifest.json", options).map_err(error)?;
        zip.write_all(&serde_json::to_vec(&manifest).map_err(error)?)
            .map_err(error)?;
        zip.start_file("interface.json", options).map_err(error)?;
        zip.write_all(&serde_json::to_vec(ui).map_err(error)?)
            .map_err(error)?;
        zip.finish().map_err(error)?.sync_all().map_err(error)?;
        // Existing backups survive a failed export, including Windows replacement.
        fs::rename(&temp, &destination).map_err(error)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn restore_dir(root: &Path) -> PathBuf {
    root.with_file_name(format!(
        "{}.restore",
        root.file_name().unwrap().to_string_lossy()
    ))
}
fn rollback_dir(root: &Path) -> PathBuf {
    root.with_file_name(format!(
        "{}.before-restore",
        root.file_name().unwrap().to_string_lossy()
    ))
}

fn rewrite_paths(value: &mut Value, mappings: &[(String, String)], key: &str) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                rewrite_paths(value, mappings, key);
            }
        }
        Value::Array(items) => {
            for value in items {
                rewrite_paths(value, mappings, key);
            }
        }
        Value::String(value)
            if key == "path"
                || key.ends_with("_path")
                || key.ends_with("Path")
                || key.ends_with("_paths")
                || key.ends_with("Paths")
                || matches!(
                    key,
                    "workingDirectory"
                        | "working_directory"
                        | "cwd"
                        | "skill_cwd"
                        | "directory"
                        | "command"
                        | "args"
                ) =>
        {
            for (source, target) in mappings {
                if value == source
                    || value
                        .strip_prefix(source)
                        .is_some_and(|tail| tail.starts_with('/') || tail.starts_with('\\'))
                {
                    *value = format!("{target}{}", &value[source.len()..]).replace('\\', "/");
                    break;
                }
            }
        }
        _ => {}
    }
}

/// Only application-owned records may be rewritten. Attachments, project files,
/// plugin content and native CLI history are opaque, even when named *.json.
/// The flag identifies records whose execution switches must be disabled on import.
fn restore_metadata_policy(relative: &Path) -> Option<bool> {
    let parts: Vec<_> = relative.iter().map(|part| part.to_str()).collect();
    match parts.as_slice() {
        [Some("settings.json")] => Some(false),
        [Some("conversations" | "lens-history"), Some(file)] if file.ends_with(".json") => {
            Some(false)
        }
        [Some("artifacts"), Some("records"), Some(file)] if file.ends_with(".json") => Some(false),
        [Some("media-station"), Some(_), Some("job.json")] => Some(false),
        [Some("plugins"), Some(_), Some("meta.json")]
        | [Some("plugins"), Some("packages"), Some(_), Some("record.json")]
        | [Some("plugins"), Some("workflow-hooks.json")]
        | [Some("scheduled_tasks"), Some("tasks.json")] => Some(true),
        [Some("automations"), Some(file)] if file.ends_with(".json") => Some(true),
        _ => None,
    }
}

fn stage_restore(
    root: &Path,
    archive: &Path,
    trusted: bool,
    local: &Settings,
) -> Result<(), String> {
    let stage = restore_dir(root);
    if stage.exists() {
        return Err("已有待恢复备份，请先重启 Kivio 完成恢复".into());
    }
    let work = stage.with_extension(format!("preparing-{}", uuid::Uuid::new_v4()));
    private_dir(&work)?;
    let result = (|| {
        let mut zip = zip::ZipArchive::new(File::open(archive).map_err(error)?).map_err(error)?;
        if zip.len() > MAX_FILES + 2 {
            return Err("备份文件数量超过上限".into());
        }
        let manifest: Manifest = {
            let entry = zip.by_name("manifest.json").map_err(error)?;
            if entry.size() > 128 * 1024 * 1024 {
                return Err("备份清单过大".into());
            }
            serde_json::from_reader(entry).map_err(error)?
        };
        if manifest.app != "kivio" || manifest.kind != "full-backup" || manifest.version != VERSION
        {
            return Err("不支持的完全备份格式或版本".into());
        }
        if manifest.roots.first().is_none_or(|r| r.archive != "app") {
            return Err("备份缺少应用数据".into());
        }
        let mut prefixes = HashSet::new();
        for r in &manifest.roots {
            portable_path(&r.archive)?;
            if !(r.archive == "app"
                || (r.archive.starts_with("work/") && r.archive[5..].parse::<u32>().is_ok()))
                || !prefixes.insert(&r.archive)
            {
                return Err("备份数据根目录无效".into());
            }
        }
        let restored_files = format!("restored-files/{}", uuid::Uuid::new_v4());
        private_dir(&work.join("app"))?;
        for r in manifest.roots.iter().filter(|r| r.archive != "app") {
            private_dir(&work.join("app").join(&restored_files).join(&r.archive[5..]))?;
        }
        let mut seen = HashSet::new();
        let mut destinations = HashSet::new();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(error)?;
            let name = entry.name().to_string();
            portable_path(&name)?;
            if !seen.insert(name.clone()) {
                return Err("备份包含重复文件".into());
            }
            if name == "manifest.json" || name == "interface.json" {
                continue;
            }
            let record = manifest.files.get(&name).ok_or("备份存在清单之外的文件")?;
            if entry.is_dir() || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                return Err("备份包含链接或非法目录条目".into());
            }
            total = total.checked_add(entry.size()).ok_or("备份过大")?;
            if total > MAX_BYTES || entry.size() != record.bytes {
                return Err("备份大小校验失败".into());
            }
            let relative = if let Some(path) = name.strip_prefix("app/") {
                let top = path.split('/').next().unwrap_or_default();
                if excluded(top) {
                    return Err("备份包含不可恢复的运行时文件".into());
                }
                portable_path(path)?
            } else {
                let r = manifest
                    .roots
                    .iter()
                    .find(|r| name.starts_with(&format!("{}/", r.archive)))
                    .ok_or("备份文件目录无效")?;
                PathBuf::from(&restored_files)
                    .join(&r.archive[5..])
                    .join(portable_path(&name[r.archive.len() + 1..])?)
            };
            if !destinations.insert(relative.to_string_lossy().to_lowercase()) {
                return Err("备份包含冲突的文件路径".into());
            }
            let target = work.join("app").join(relative);
            private_dir(target.parent().unwrap())?;
            let mut file = File::create(&target).map_err(error)?;
            let copied = std::io::copy(&mut (&mut entry).take(record.bytes + 1), &mut file)
                .map_err(error)?;
            file.sync_all().map_err(error)?;
            if copied != record.bytes || digest(&target)? != *record {
                return Err(format!("备份文件校验失败：{name}"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(
                    entry.unix_mode().unwrap_or(0o600) & 0o700,
                ))
                .map_err(error)?;
            }
        }
        if manifest.files.keys().any(|name| !seen.contains(name)) {
            return Err("备份缺少文件".into());
        }
        let app = work.join("app");
        let mut mappings: Vec<_> = manifest
            .roots
            .iter()
            .map(|r| {
                (
                    r.source.clone(),
                    if r.archive == "app" {
                        root.to_path_buf()
                    } else {
                        root.join(&restored_files).join(&r.archive[5..])
                    }
                    .to_string_lossy()
                    .to_string(),
                )
            })
            .collect();
        mappings.sort_by_key(|(source, _)| std::cmp::Reverse(source.len()));
        for relative in files(&app, true)? {
            let Some(execution_settings) = restore_metadata_policy(&relative) else {
                continue;
            };
            let path = app.join(&relative);
            let mut value = read_json(&path)?;
            rewrite_paths(&mut value, &mappings, "");
            if !trusted && execution_settings {
                disable_enabled(&mut value);
            }
            write_json(&path, &value)?;
        }
        let settings_path = app.join("settings.json");
        let mut document = read_json(&settings_path)?;
        let mut settings: Settings =
            serde_json::from_value(document.get("settings").cloned().ok_or("备份缺少设置")?)
                .map_err(error)?;
        crate::commands::prepare_imported_settings(
            &mut settings,
            trusted,
            &local.chat.external_cli_agents,
        );
        document["settings"] =
            serde_json::to_value(crate::settings::sanitize_settings(settings)).map_err(error)?;
        write_json(&settings_path, &document)?;
        let ui: BTreeMap<String, String> = match zip.by_name("interface.json") {
            Ok(entry) if entry.size() <= 16 * 1024 * 1024 => {
                serde_json::from_reader(entry).map_err(error)?
            }
            _ => return Err("备份缺少有效界面状态".into()),
        };
        write_json(
            &app.join(".backup-interface.json"),
            &serde_json::json!({ "id": uuid::Uuid::new_v4().to_string(), "values": ui }),
        )?;
        write_json(&work.join("ready.json"), &true)?;
        fs::rename(&work, &stage).map_err(error)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&work);
    }
    result
}
fn disable_enabled(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if let Some(enabled) = fields.get_mut("enabled") {
                *enabled = Value::Bool(false);
            }
            for v in fields.values_mut() {
                disable_enabled(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                disable_enabled(v);
            }
        }
        _ => {}
    }
}

/// Startup recovery uses filesystem state as its journal. If interrupted between
/// the two renames, the next launch completes the prepared swap or restores old data.
pub(crate) fn apply_pending(root: &Path) -> Result<(), String> {
    let stage = restore_dir(root);
    let rollback = rollback_dir(root);
    if !stage.join("ready.json").exists() {
        if !root.exists() && rollback.exists() {
            fs::rename(&rollback, root).map_err(error)?;
        }
        return Ok(());
    }
    if stage.join("app").exists() {
        if root.exists() {
            if rollback.exists() {
                let previous =
                    rollback.with_extension(format!("before-restore-{}", uuid::Uuid::new_v4()));
                fs::rename(&rollback, previous).map_err(error)?;
            }
            fs::rename(root, &rollback).map_err(error)?;
        }
        if let Err(e) = fs::rename(stage.join("app"), root) {
            if !root.exists() && rollback.exists() {
                fs::rename(&rollback, root).map_err(|rollback_error| {
                    format!("恢复失败：{e}；回滚失败：{rollback_error}")
                })?;
            }
            return Err(format!("恢复未完成，原数据已保留：{e}"));
        }
    }
    if !root.exists() {
        if rollback.exists() {
            fs::rename(&rollback, root).map_err(error)?;
        } else {
            return Err("恢复目录丢失，无法完成恢复".into());
        }
    }
    fs::remove_dir_all(stage).map_err(error)?;
    // Keep the previous installation as a recovery copy, never silently delete it.
    Ok(())
}

#[tauri::command]
pub(crate) async fn export_full_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    interface: BTreeMap<String, String>,
) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(error)?;
    crate::provider_oauth::export_credentials(&state.settings_read())?;
    let workspace = PathBuf::from(
        &state
            .settings_read()
            .chat_tools
            .native_tools
            .working_directory,
    );
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = OPERATION.try_lock().map_err(|_| "备份或恢复正在进行")?;
        export_to(&root, &workspace, Path::new(&path), &interface)
    })
    .await
    .map_err(error)?
}
#[tauri::command]
pub(crate) async fn import_full_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    trusted: bool,
) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(error)?;
    let local = state.settings_read().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = OPERATION.try_lock().map_err(|_| "备份或恢复正在进行")?;
        stage_restore(&root, Path::new(&path), trusted, &local)
    })
    .await
    .map_err(error)?
}
#[tauri::command]
pub(crate) fn backup_interface(app: AppHandle) -> Result<Option<Value>, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(error)?
        .join(".backup-interface.json");
    if path.exists() {
        read_json(&path).map(Some)
    } else {
        Ok(None)
    }
}
#[tauri::command]
pub(crate) fn backup_restore_pending(app: AppHandle) -> Result<bool, String> {
    Ok(restore_dir(&app.path().app_data_dir().map_err(error)?)
        .join("ready.json")
        .exists())
}
#[tauri::command]
pub(crate) fn restart_after_restore(app: AppHandle) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(error)?;
    if !restore_dir(&root).join("ready.json").exists() {
        return Err("没有待恢复备份".into());
    }
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, workspace: &Path) {
        private_dir(&root.join("conversations")).unwrap();
        private_dir(workspace).unwrap();
        let mut settings = Settings::default();
        settings.chat_tools.native_tools.working_directory = workspace.to_string_lossy().into();
        write_json(
            &root.join("settings.json"),
            &serde_json::json!({"settings": settings}),
        )
        .unwrap();
        write_json(&root.join("conversations/conv_one.json"), &serde_json::json!({"id":"conv_one","messages":[{"content":"original conversation","attachments":[{"path":workspace.join("file.txt")}]}]})).unwrap();
        fs::write(workspace.join("file.txt"), b"attachment bytes").unwrap();
        private_dir(&root.join("scheduled_tasks")).unwrap();
        write_json(
            &root.join("scheduled_tasks/tasks.json"),
            &serde_json::json!([{"id":"task","enabled":true}]),
        )
        .unwrap();
        private_dir(&root.join("knowledge_base/kb_one")).unwrap();
        let db = rusqlite::Connection::open(root.join("knowledge_base/kb_one/store.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE docs(text TEXT); INSERT INTO docs VALUES ('knowledge document');",
        )
        .unwrap();
    }
    #[test]
    fn restore_preserves_user_json_and_only_updates_owned_metadata() {
        for trusted in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("source");
            let workspace = temp.path().join("workspace");
            fixture(&root, &workspace);
            let content = format!(
                "{{\n  \"path\": {:?},\n  \"enabled\": true\n}}\n",
                workspace.join("file.txt").to_string_lossy()
            );
            let user_files = [
                (
                    "conversations/conv_one_attachments/input.json",
                    content.as_bytes(),
                ),
                (
                    "conversations/conv_one_attachments/broken.json",
                    b"{ incomplete JSON".as_slice(),
                ),
                (
                    "conversations/conv_one_attachments/workflow-hooks.json",
                    content.as_bytes(),
                ),
                (
                    "plugins/packages/demo/content/meta.json",
                    content.as_bytes(),
                ),
                (
                    "plugins/packages/demo/content/record.json",
                    content.as_bytes(),
                ),
                ("local-project/package.json", content.as_bytes()),
            ];
            for (name, bytes) in user_files {
                let path = root.join(name);
                private_dir(path.parent().unwrap()).unwrap();
                fs::write(path, bytes).unwrap();
            }
            fs::write(workspace.join("config.json"), &content).unwrap();
            let plugin = root.join("plugins/packages/demo/record.json");
            write_json(
                &plugin,
                &serde_json::json!({"enabled":true,"path":workspace.join("file.txt")}),
            )
            .unwrap();
            let archive = temp.path().join("backup.kivio");
            export_to(&root, &workspace, &archive, &BTreeMap::new()).unwrap();
            let dest = temp.path().join("target");
            stage_restore(&dest, &archive, trusted, &Settings::default()).unwrap();
            apply_pending(&dest).unwrap();
            for (name, bytes) in user_files {
                assert_eq!(
                    fs::read(dest.join(name)).unwrap(),
                    bytes,
                    "{name}, trusted={trusted}"
                );
            }
            let settings = read_json(&dest.join("settings.json")).unwrap();
            let restored: Settings = serde_json::from_value(settings["settings"].clone()).unwrap();
            let restored_workspace = Path::new(&restored.chat_tools.native_tools.working_directory);
            assert_eq!(
                fs::read(restored_workspace.join("config.json")).unwrap(),
                content.as_bytes()
            );
            let plugin = read_json(&dest.join("plugins/packages/demo/record.json")).unwrap();
            assert_eq!(plugin["enabled"], trusted);
            assert_eq!(
                plugin["path"],
                restored_workspace
                    .join("file.txt")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            assert_eq!(
                read_json(&dest.join("scheduled_tasks/tasks.json")).unwrap()[0]["enabled"],
                trusted
            );
        }
    }

    #[test]
    fn full_round_trip_relocates_files_preserves_history_and_commits_only_on_restart() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let root = source.path().join("app");
        let workspace = source.path().join("workspace");
        fixture(&root, &workspace);
        let project = source.path().join("project");
        private_dir(&project).unwrap();
        fs::write(project.join("main.txt"), b"project file").unwrap();
        write_json(
            &root.join("conversations/projects.json"),
            &serde_json::json!({"projects":[{"id":"proj_one","root_path":project}]}),
        )
        .unwrap();
        let archive = source.path().join("backup.kivio");
        export_to(
            &root,
            &workspace,
            &archive,
            &BTreeMap::from([("kivio-sidebar".into(), "flat".into())]),
        )
        .unwrap();
        let dest = target.path().join("app");
        private_dir(&dest).unwrap();
        fs::write(dest.join("previous.txt"), "keep until restart").unwrap();
        stage_restore(&dest, &archive, false, &Settings::default()).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("previous.txt")).unwrap(),
            "keep until restart"
        );
        assert!(!dest.join("conversations").exists());
        apply_pending(&dest).unwrap();
        let conversation = read_json(&dest.join("conversations/conv_one.json")).unwrap();
        assert_eq!(
            conversation["messages"][0]["content"],
            "original conversation"
        );
        let path = conversation["messages"][0]["attachments"][0]["path"]
            .as_str()
            .unwrap();
        assert!(Path::new(path).starts_with(&dest));
        assert_eq!(fs::read(path).unwrap(), b"attachment bytes");
        assert_eq!(
            read_json(&dest.join("scheduled_tasks/tasks.json")).unwrap()[0]["enabled"],
            false
        );
        assert!(rollback_dir(&dest).join("previous.txt").exists());
        assert!(!dest.join("previous.txt").exists());
        let projects = read_json(&dest.join("conversations/projects.json")).unwrap();
        let project_path = Path::new(projects["projects"][0]["root_path"].as_str().unwrap());
        assert!(project_path.starts_with(&dest));
        assert_eq!(
            fs::read(project_path.join("main.txt")).unwrap(),
            b"project file"
        );
        assert_eq!(fs::read(project.join("main.txt")).unwrap(), b"project file");
        let db = rusqlite::Connection::open(dest.join("knowledge_base/kb_one/store.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT text FROM docs", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "knowledge document"
        );
        apply_pending(&dest).unwrap(); // Completed restore is idempotent.
    }
    #[test]
    fn restores_an_unused_workspace_under_the_new_data_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let workspace = temp.path().join("unused-workspace");
        private_dir(&root).unwrap();
        let mut settings = Settings::default();
        settings.chat_tools.native_tools.working_directory = workspace.to_string_lossy().into();
        write_json(
            &root.join("settings.json"),
            &serde_json::json!({"settings":settings}),
        )
        .unwrap();
        let archive = temp.path().join("backup.kivio");
        export_to(&root, &workspace, &archive, &BTreeMap::new()).unwrap();
        let target = temp.path().join("target");
        stage_restore(&target, &archive, true, &Settings::default()).unwrap();
        apply_pending(&target).unwrap();
        let document = read_json(&target.join("settings.json")).unwrap();
        let restored: Settings = serde_json::from_value(document["settings"].clone()).unwrap();
        let destination = Path::new(&restored.chat_tools.native_tools.working_directory);
        assert!(destination.starts_with(&target));
        assert!(destination.is_dir());
        assert!(!workspace.exists());
    }

    #[test]
    fn rejects_corruption_unsafe_entries_and_future_versions_without_mutating_destination() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let workspace = temp.path().join("workspace");
        fixture(&source, &workspace);
        let original = temp.path().join("original.kivio");
        export_to(&source, &workspace, &original, &BTreeMap::new()).unwrap();
        let destination = temp.path().join("target");
        private_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), "unchanged").unwrap();
        for mode in ["corrupt", "traversal", "version", "duplicate"] {
            let archive = temp.path().join(format!("{mode}.kivio"));
            let mut input = zip::ZipArchive::new(File::open(&original).unwrap()).unwrap();
            let mut output = zip::ZipWriter::new(File::create(&archive).unwrap());
            for i in 0..input.len() {
                let mut entry = input.by_index(i).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                let mut name = entry.name().to_string();
                if name == "app/conversations/conv_one.json" {
                    if mode == "corrupt" {
                        bytes[0] = b'!';
                    }
                    if mode == "traversal" {
                        name = "app/../../escape".into();
                    }
                }
                if mode == "version" && name == "manifest.json" {
                    let mut v: Value = serde_json::from_slice(&bytes).unwrap();
                    v["version"] = 999.into();
                    bytes = serde_json::to_vec(&v).unwrap();
                }
                output
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                output.write_all(&bytes).unwrap();
            }
            if mode == "duplicate" {
                output
                    .start_file("app/extra", zip::write::SimpleFileOptions::default())
                    .unwrap();
                output.write_all(b"unexpected").unwrap();
            }
            output.finish().unwrap();
            assert!(
                stage_restore(&destination, &archive, true, &Settings::default()).is_err(),
                "{mode}"
            );
            assert_eq!(
                fs::read_to_string(destination.join("sentinel")).unwrap(),
                "unchanged"
            );
            assert!(!restore_dir(&destination).exists());
            assert!(!temp.path().join("escape").exists());
        }
    }
    #[test]
    fn restart_finishes_interrupted_swap_and_failed_export_preserves_old_archive() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        let workspace = temp.path().join("workspace");
        fixture(&root, &workspace);
        let archive = temp.path().join("backup.kivio");
        export_to(&root, &workspace, &archive, &BTreeMap::new()).unwrap();
        stage_restore(&root, &archive, true, &Settings::default()).unwrap();
        fs::rename(&root, rollback_dir(&root)).unwrap();
        apply_pending(&root).unwrap();
        assert_eq!(
            read_json(&root.join("scheduled_tasks/tasks.json")).unwrap()[0]["enabled"],
            true
        );
        let previous = fs::read(&archive).unwrap();
        fs::write(root.join("conversations/projects.json"), b"invalid JSON").unwrap();
        assert!(export_to(&root, &workspace, &archive, &BTreeMap::new()).is_err());
        assert_eq!(fs::read(&archive).unwrap(), previous);
    }
}
