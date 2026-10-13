//! Official model data lifecycle. Downloaded data never shares the user's overlay file.
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, OnceLock, RwLock},
    time::Duration,
};
use tauri::Emitter;

const BUNDLED: &str = include_str!("../../../src/data/modelDatabase.json");
const URL: &str = "https://raw.githubusercontent.com/ZMGID/kivio-models/main/v1/modelDatabase.json";
const CACHE_FILE: &str = "modelDatabase.official.v1.json";
const USER_FILE: &str = "modelDatabase.json";
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub version: String,
    pub database: Map<String, Value>,
    #[serde(skip)]
    pub exact_norm: HashMap<String, String>,
    #[serde(skip)]
    pub pairs: Vec<(String, String)>,
}

fn version_key(value: &str) -> Result<Vec<u32>, String> {
    let parts: Vec<_> = value.split(['-', '.']).collect();
    if parts.len() != 4 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err("Invalid model database version; expected YYYY-MM-DD.N".into());
    }
    let numbers = parts
        .iter()
        .map(|p| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Invalid model database version".into());
            }
            p.parse::<u32>()
                .map_err(|_| "Invalid model database version".to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !(1..=12).contains(&numbers[1]) || !(1..=31).contains(&numbers[2]) || numbers[3] == 0 {
        return Err("Invalid model database version".into());
    }
    Ok(numbers)
}

fn validate(text: &str) -> Result<Map<String, Value>, String> {
    if text.len() > MAX_BYTES {
        return Err("Model database is too large".into());
    }
    let data: Map<String, Value> =
        serde_json::from_str(text).map_err(|e| format!("Invalid model database JSON: {e}"))?;
    let meta = data
        .get("_meta")
        .ok_or("Model database metadata is missing")?;
    if meta.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported model database format; update Kivio first".into());
    }
    version_key(
        meta.get("version")
            .and_then(Value::as_str)
            .ok_or("Model database version is missing")?,
    )?;
    if data.len() < 2 {
        return Err("Model database is empty".into());
    }
    for (id, entry) in data.iter().filter(|(id, _)| id.as_str() != "_meta") {
        let invalid = || format!("Invalid model database entry: {id}");
        if id.is_empty() || id.trim() != id || id.to_ascii_lowercase() != *id {
            return Err(invalid());
        }
        let entry = entry.as_object().ok_or_else(invalid)?;
        if !entry
            .get("displayName")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
        {
            return Err(invalid());
        }
        for field in ["contextWindow", "maxOutput", "dimensions"] {
            if let Some(v) = entry.get(field) {
                if !v.as_u64().is_some_and(|n| n <= u32::MAX as u64) {
                    return Err(invalid());
                }
            } else if field == "contextWindow" {
                return Err(invalid());
            }
        }
        if !entry
            .get("capabilities")
            .and_then(Value::as_object)
            .is_some_and(|caps| caps.values().all(Value::is_boolean))
        {
            return Err(invalid());
        }
        if let Some(v) = entry.get("pricing") {
            let prices = v.as_object().ok_or_else(invalid)?;
            if !prices
                .values()
                .all(|p| p.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
            {
                return Err(invalid());
            }
        }
        if let Some(v) = entry.get("reasoningEfforts") {
            if !v.as_array().is_some_and(|levels| {
                levels.iter().all(|l| {
                    l.as_str().is_some_and(|l| {
                        ["minimal", "low", "medium", "high", "xhigh", "max"].contains(&l)
                    })
                })
            }) {
                return Err(invalid());
            }
        }
        for field in ["thinkingOff", "multilingual"] {
            if entry.get(field).is_some_and(|v| !v.is_boolean()) {
                return Err(invalid());
            }
        }
        if entry
            .get("temperature")
            .is_some_and(|v| !v.as_f64().is_some_and(|n| (0.0..=2.0).contains(&n)))
        {
            return Err(invalid());
        }
    }
    Ok(data)
}

pub(super) fn merge_user_entries(base: &mut Map<String, Value>, user: Map<String, Value>) {
    for (key, value) in user {
        let key = key.trim().to_ascii_lowercase();
        if key.is_empty() || key == "_meta" {
            continue;
        }
        match (base.get_mut(&key).and_then(Value::as_object_mut), value) {
            (Some(existing), Value::Object(fields)) => existing.extend(fields),
            (_, Value::Object(fields)) => {
                base.insert(key, Value::Object(fields));
            }
            _ => {}
        }
    }
}

impl Snapshot {
    fn new(mut database: Map<String, Value>, dir: Option<&Path>, revision: u64) -> Self {
        let version = database["_meta"]["version"].as_str().unwrap().to_owned();
        if let Some(user) = dir
            .and_then(|dir| std::fs::read_to_string(dir.join(USER_FILE)).ok())
            .and_then(|text| serde_json::from_str::<Map<String, Value>>(&text).ok())
        {
            merge_user_entries(&mut database, user);
        }
        let mut exact_norm = HashMap::new();
        let pairs = database
            .keys()
            .filter(|key| key.as_str() != "_meta")
            .map(|key| {
                let norm = key.replace('.', "-");
                exact_norm
                    .entry(norm.clone())
                    .or_insert_with(|| key.clone());
                (key.clone(), norm)
            })
            .collect();
        Self {
            revision,
            version,
            database,
            exact_norm,
            pairs,
        }
    }
}

struct DatabaseStore(RwLock<Arc<Snapshot>>);
impl DatabaseStore {
    fn load(bundled: &str, dir: Option<&Path>) -> Self {
        let mut database = validate(bundled).expect("Bundled model database must pass validation");
        if let Some(cached) = dir
            .and_then(|d| std::fs::read_to_string(d.join(CACHE_FILE)).ok())
            .and_then(|t| validate(&t).ok())
        {
            if version_key(cached["_meta"]["version"].as_str().unwrap()).unwrap()
                > version_key(database["_meta"]["version"].as_str().unwrap()).unwrap()
            {
                database = cached;
            }
        }
        Self(RwLock::new(Arc::new(Snapshot::new(database, dir, 0))))
    }
    fn snapshot(&self) -> Arc<Snapshot> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn install(&self, text: &str, dir: &Path) -> Result<Arc<Snapshot>, String> {
        let database = validate(text)?;
        let mut current = self.0.write().map_err(|_| "Model database lock failed")?;
        let incoming = database["_meta"]["version"].as_str().unwrap();
        if version_key(incoming)? <= version_key(&current.version)? {
            return Ok(current.clone());
        }
        let next = Arc::new(Snapshot::new(database, Some(dir), current.revision + 1));
        super::storage::atomic_write(&dir.join(CACHE_FILE), text, "model database")?;
        *current = next.clone();
        Ok(next)
    }
}

fn store() -> &'static DatabaseStore {
    static STORE: OnceLock<DatabaseStore> = OnceLock::new();
    STORE.get_or_init(|| DatabaseStore::load(BUNDLED, crate::app_data::app_data_dir().as_deref()))
}
pub(crate) fn snapshot() -> Arc<Snapshot> {
    store().snapshot()
}

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
