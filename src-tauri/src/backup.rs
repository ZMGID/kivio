pub(crate) mod chat_archive;
pub mod webdav;

#[tauri::command]
pub(crate) async fn export_chat_backup(app: tauri::AppHandle, path: String) -> Result<(), String> {
    crate::chat::repository::repository(&app)
        .export_chat_backup(&app, path.into())
        .await
        .map_err(crate::chat::repository::repository_error)
}

#[tauri::command]
pub(crate) async fn restore_chat_backup(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri::Manager;
    if app
        .state::<crate::state::AppState>()
        .chat_runtime()
        .has_running_chats()
    {
        return Err("请等待聊天完成或停止生成后再恢复 / Stop active chats before restoring".into());
    }
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || chat_archive::prepare(&root, Path::new(&path)))
        .await
        .map_err(|e| e.to_string())??;
    app.restart();
}

use std::io::{Read, Write};
use std::{fs, path::Path};

const CONFIG_FILES: [&str; 3] = [
    "settings.json",
    "conversations/assistants.json",
    "conversations/sets.json",
];
const MAX_CONFIG_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn export_configuration(root: &Path, path: &Path) -> Result<(), String> {
    let _catalog = crate::chat::storage::catalog_mutation_lock();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipWriter::new(temporary.as_file_mut());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in CONFIG_FILES {
        let bytes = match fs::read(root.join(name)) {
            Ok(bytes) => bytes,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && name == "settings.json" =>
            {
                serde_json::to_vec(&serde_json::json!({
                    "settings": crate::settings::Settings::default()
                }))
                .map_err(|e| e.to_string())?
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && name != "settings.json" =>
            {
                if name.ends_with("assistants.json") {
                    b"{\"assistants\":[]}".to_vec()
                } else {
                    b"{\"sets\":[]}".to_vec()
                }
            }
            Err(error) => return Err(error.to_string()),
        };
        archive
            .start_file(name, options)
            .map_err(|e| e.to_string())?;
        archive.write_all(&bytes).map_err(|e| e.to_string())?;
        if name == "settings.json" {
            let document: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let settings =
                serde_json::from_value(document["settings"].clone()).map_err(|e| e.to_string())?;
            let credentials = crate::provider_oauth::export_credentials(&settings)?;
            archive
                .start_file("provider-oauth.json", options)
                .map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut archive, &credentials).map_err(|e| e.to_string())?;
        }
    }
    archive.finish().map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn read_configuration(path: &Path) -> Result<serde_json::Value, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut signature = [0; 2];
    file.read_exact(&mut signature).map_err(|e| e.to_string())?;
    if signature != *b"PK" {
        return serde_json::from_reader(
            fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(MAX_CONFIG_BYTES),
        )
        .map_err(|e| e.to_string());
    }
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut remaining = MAX_CONFIG_BYTES;
    let mut read = |name: &str| -> Result<serde_json::Value, String> {
        let entry = archive.by_name(name).map_err(|e| format!("{name}: {e}"))?;
        let mut bytes = Vec::new();
        entry
            .take(remaining + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        remaining = remaining
            .checked_sub(bytes.len() as u64)
            .ok_or("配置备份超过 64 MiB")?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{name}: {e}"))
    };
    // Read only fixed entries; archive paths are never extracted onto the filesystem.
    Ok(serde_json::json!({
        "app": "kivio", "type": "settings-backup", "version": 1,
        "settings": read("settings.json")?["settings"],
        "sets": {
            "assistants": read("conversations/assistants.json")?["assistants"],
            "sets": read("conversations/sets.json")?["sets"],
        },
        "providerOAuth": read("provider-oauth.json")?,
    }))
}

pub(crate) fn recover_at_startup(root: &Path) -> Result<(), String> {
    chat_archive::restore_at_startup(root)?;
    if root.exists() {
        return Ok(());
    }
    let name = root
        .file_name()
        .ok_or("Invalid app data directory")?
        .to_string_lossy();
    let rollback = root.with_file_name(format!("{name}.before-restore"));
    if rollback.exists() {
        return fs::rename(rollback, root).map_err(|error| error.to_string());
    }
    if root
        .with_file_name(format!("{name}.restore"))
        .join("app")
        .exists()
    {
        return Err("旧版完全恢复已停用，当前数据目录缺失。请手动找回原数据目录。 / Legacy full restore is disabled and the current data directory is missing. Recover the original data directory manually.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_zip_preserves_files_and_excludes_conversations() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("conversations")).unwrap();
        let settings = serde_json::json!({"settings": crate::settings::Settings::default()});
        let raw = serde_json::to_vec(&settings).unwrap();
        fs::write(root.path().join("settings.json"), &raw).unwrap();
        fs::write(
            root.path().join("conversations/private.json"),
            b"private chat",
        )
        .unwrap();
        let path = root.path().join("backup.zip");
        export_configuration(root.path(), &path).unwrap();
        let mut archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(archive.len(), 4);
        let mut stored = Vec::new();
        archive
            .by_name("settings.json")
            .unwrap()
            .read_to_end(&mut stored)
            .unwrap();
        assert_eq!(stored, raw);
        assert!(archive.by_name("conversations/private.json").is_err());
        let imported = read_configuration(&path).unwrap();
        assert_eq!(imported["settings"], settings["settings"]);
        assert_eq!(imported["sets"]["assistants"], serde_json::json!([]));
        let legacy = root.path().join("legacy.json");
        fs::write(&legacy, serde_json::to_vec(&imported).unwrap()).unwrap();
        assert_eq!(read_configuration(&legacy).unwrap(), imported);
        fs::write(root.path().join("settings.json"), b"broken").unwrap();
        assert!(export_configuration(root.path(), &path).is_err());
        assert_eq!(read_configuration(&path).unwrap(), imported);
    }

    #[test]
    fn incomplete_archive_is_rejected_without_extracting_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("invalid.zip");
        let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        archive
            .start_file("../settings.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"{}").unwrap();
        archive.finish().unwrap();
        assert!(read_configuration(&path).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn startup_only_rolls_back_missing_current_data_and_is_repeatable() {
        for current in [false, true] {
            for rollback in [false, true] {
                for staged in [false, true] {
                    for ready in [false, true] {
                        let temp = tempfile::tempdir().unwrap();
                        let root = temp.path().join("kivio");
                        let old = temp.path().join("kivio.before-restore");
                        let stage = temp.path().join("kivio.restore");
                        fs::create_dir(&stage).unwrap();
                        fs::write(stage.join("marker"), b"preserve-stage").unwrap();
                        if current {
                            fs::create_dir(&root).unwrap();
                            fs::write(root.join("data"), b"current").unwrap();
                        }
                        if rollback {
                            fs::create_dir(&old).unwrap();
                            fs::write(old.join("data"), b"original").unwrap();
                        }
                        if staged {
                            fs::create_dir(stage.join("app")).unwrap();
                            fs::write(stage.join("app/data"), b"foreign").unwrap();
                        }
                        if ready {
                            fs::write(stage.join("ready.json"), b"true").unwrap();
                        }
                        for _ in 0..2 {
                            let result = recover_at_startup(&root);
                            assert_eq!(result.is_err(), !current && !rollback && staged);
                            assert_eq!(root.exists(), current || rollback);
                            if current || rollback {
                                assert_eq!(
                                    fs::read(root.join("data")).unwrap(),
                                    if current {
                                        b"current".as_slice()
                                    } else {
                                        b"original".as_slice()
                                    }
                                );
                            }
                            assert_eq!(old.exists(), current && rollback);
                            if old.exists() {
                                assert_eq!(fs::read(old.join("data")).unwrap(), b"original");
                            }
                            assert_eq!(fs::read(stage.join("marker")).unwrap(), b"preserve-stage");
                            assert_eq!(stage.join("ready.json").exists(), ready);
                            assert_eq!(stage.join("app").exists(), staged);
                            if staged {
                                assert_eq!(fs::read(stage.join("app/data")).unwrap(), b"foreign");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn fresh_install_stays_missing_until_normal_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("kivio");
        recover_at_startup(&root).unwrap();
        recover_at_startup(&root).unwrap();
        assert!(!root.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
