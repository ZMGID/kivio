// Official model data lifecycle. Downloaded data never shares the user's overlay file.
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, OnceLock, RwLock},
};

const BUNDLED: &str = include_str!("../../../src/data/modelDatabase.json");
pub(super) const URL: &str =
    "https://raw.githubusercontent.com/ZMGID/kivio-models/main/v1/modelDatabase.json";
const CACHE_FILE: &str = "modelDatabase.official.v1.json";
const USER_FILE: &str = "modelDatabase.json";
pub(super) const MAX_BYTES: usize = 4 * 1024 * 1024;

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

pub(super) fn version_key(value: &str) -> Result<Vec<u32>, String> {
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

pub(super) fn validate(text: &str) -> Result<Map<String, Value>, String> {
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

pub(super) struct DatabaseStore(RwLock<Arc<Snapshot>>);
impl DatabaseStore {
    pub(super) fn load(bundled: &str, dir: Option<&Path>) -> Self {
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
    pub(super) fn snapshot(&self) -> Arc<Snapshot> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub(super) fn install(&self, text: &str, dir: &Path) -> Result<Arc<Snapshot>, String> {
        let database = validate(text)?;
        let mut current = self.0.write().map_err(|_| "Model database lock failed")?;
        let incoming = database["_meta"]["version"].as_str().unwrap();
        if version_key(incoming)? <= version_key(&current.version)? {
            return Ok(current.clone());
        }
        let next = Arc::new(Snapshot::new(database, Some(dir), current.revision + 1));
        crate::chat::storage::atomic_write(&dir.join(CACHE_FILE), text, "model database")?;
        *current = next.clone();
        Ok(next)
    }
}

static STORE: OnceLock<DatabaseStore> = OnceLock::new();
static DATA_DIRECTORY: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Bind the process-wide model store before its first read (native custom roots).
pub fn configure_directory(dir: &Path) -> Result<(), String> {
    if let Some(current) = DATA_DIRECTORY.get() {
        return if current == dir { Ok(()) } else { Err("Model database directory already configured".into()) };
    }
    if STORE.get().is_some() && crate::app_data::app_data_dir().as_deref() != Some(dir) {
        return Err("Model database was initialized in another directory".into());
    }
    match DATA_DIRECTORY.set(dir.to_path_buf()) {
        Ok(()) => Ok(()),
        Err(_) if DATA_DIRECTORY.get().is_some_and(|current| current == dir) => Ok(()),
        Err(_) => Err("Model database directory already configured".into()),
    }
}
pub(super) fn store() -> &'static DatabaseStore {
    STORE.get_or_init(|| {
        let directory = DATA_DIRECTORY.get().cloned().or_else(crate::app_data::app_data_dir);
        DatabaseStore::load(BUNDLED, directory.as_deref())
    })
}
pub(crate) fn snapshot() -> Arc<Snapshot> {
    store().snapshot()
}

/// Shared update lifecycle; hosts only supply their data directory and notifications.
pub(crate) async fn update_snapshot(dir: std::path::PathBuf) -> Result<Arc<Snapshot>, String> {
    configure_directory(&dir)?;
    static UPDATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = UPDATE
        .try_lock()
        .map_err(|_| "A model database update is already running")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
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
    let result = tokio::task::spawn_blocking(move || store().install(&text, &dir))
        .await
        .map_err(|e| e.to_string())??;
    Ok(result)
}

pub fn current_version() -> String { snapshot().version.clone() }
pub async fn update_official(dir: std::path::PathBuf) -> Result<String, String> {
    Ok(update_snapshot(dir).await?.version.clone())
}
