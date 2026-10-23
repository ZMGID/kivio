//! Platform commands for the shared model database owner.
include!("model_database_data.rs");
use std::time::Duration;
use tauri::Emitter;

#[tauri::command]
pub(crate) fn get_model_database() -> Snapshot {
    (*snapshot()).clone()
}

#[tauri::command]
pub(crate) async fn update_model_database(app: tauri::AppHandle) -> Result<Snapshot, String> {
    static UPDATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = UPDATE
        .try_lock()
        .map_err(|_| "A model database update is already running")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(URL)
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|e| format!("Download model database: {e}"))?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BYTES as u64)
    {
        return Err("Model database is too large".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("Model database is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(bytes).map_err(|_| "Model database is not UTF-8")?;
    let dir = crate::app_data::app_data_dir().ok_or("Application data directory is unavailable")?;
    let result = tauri::async_runtime::spawn_blocking(move || store().install(&text, &dir))
        .await
        .map_err(|e| e.to_string())??;
    // Readers observe the committed snapshot. An IPC response also refreshes the initiating window.
    if let Err(error) = app.emit("model-database-changed", &*result) {
        eprintln!("[model_database] notification: {error}");
    }
    Ok((*result).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database(version: &str, context: u64) -> String {
        serde_json::json!({"_meta":{"version":version,"schemaVersion":1},"sample-1":{"displayName":"Sample","contextWindow":context,"capabilities":{"vision":true},"pricing":{"input":2}}}).to_string()
    }
    #[test]
    fn update_persists_refreshes_indexes_preserves_overlays_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(USER_FILE),
            r#"{"SAMPLE-1":{"pricing":{"input":9}},"private-model":{"contextWindow":123}}"#,
        )
        .unwrap();
        let bundled = database("2026-10-13.1", 100);
        let store = DatabaseStore::load(&bundled, Some(dir.path()));
        let old = store.snapshot();
        let mut incoming = validate(&database("2026-10-13.2", 200)).unwrap();
        incoming.insert("new-2.1".into(), incoming["sample-1"].clone());
        let updated = store
            .install(&serde_json::to_string(&incoming).unwrap(), dir.path())
            .unwrap();
        assert_eq!(updated.database["sample-1"]["contextWindow"], 200);
        assert_eq!(updated.database["sample-1"]["pricing"]["input"], 9);
        assert_eq!(updated.database["private-model"]["contextWindow"], 123);
        assert_eq!(updated.exact_norm["new-2-1"], "new-2.1");
        assert_eq!(old.database["sample-1"]["contextWindow"], 100);
        assert_eq!(updated.revision, 1);
        assert_eq!(
            DatabaseStore::load(&bundled, Some(dir.path()))
                .snapshot()
                .version,
            "2026-10-13.2"
        );
        assert_eq!(store.install(&bundled, dir.path()).unwrap().revision, 1);
        assert_eq!(
            DatabaseStore::load(&database("2026-10-14.1", 300), Some(dir.path()))
                .snapshot()
                .database["sample-1"]["contextWindow"],
            300
        );
    }
    #[test]
    fn invalid_download_and_write_failure_leave_last_good_data_intact() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = database("2026-10-13.1", 100);
        let store = DatabaseStore::load(&bundled, Some(dir.path()));
        store
            .install(&database("2026-10-13.2", 200), dir.path())
            .unwrap();
        let saved = std::fs::read_to_string(dir.path().join(CACHE_FILE)).unwrap();
        for invalid in [
            "{}".into(),
            "broken JSON".into(),
            database("2026-10-13.3", 300).replace("\"schemaVersion\":1", "\"schemaVersion\":2"),
            database("2026-10-13.3", 300).replace("\"vision\":true", "\"vision\":\"yes\""),
        ] {
            assert!(store.install(&invalid, dir.path()).is_err());
            assert_eq!(store.snapshot().version, "2026-10-13.2");
            assert_eq!(
                std::fs::read_to_string(dir.path().join(CACHE_FILE)).unwrap(),
                saved
            );
        }
        let blocked = dir.path().join("file-not-directory");
        std::fs::write(&blocked, "keep").unwrap();
        assert!(store
            .install(&database("2026-10-13.3", 300), &blocked)
            .is_err());
        assert_eq!(store.snapshot().version, "2026-10-13.2");
        std::fs::write(dir.path().join(CACHE_FILE), "truncated").unwrap();
        assert_eq!(
            DatabaseStore::load(&bundled, Some(dir.path()))
                .snapshot()
                .version,
            "2026-10-13.1"
        );
    }
    #[test]
    fn bundled_catalog_is_valid_and_version_revisions_sort_numerically() {
        validate(BUNDLED).unwrap();
        assert!(version_key("2026-10-13.10").unwrap() > version_key("2026-10-13.9").unwrap());
    }
}
