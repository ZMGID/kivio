//! Native backup host. Configuration recovery is journaled before startup,
//! while archive validation and trust policy stay in the shared backup owner.
use crate::{
    backup,
    backup_configuration::SetCatalogBackup,
    settings::{Settings, sanitize_settings},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
fn read(path: &Path, fallback: Value) -> Result<Value, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(fallback),
        Err(e) => Err(e.to_string()),
    }
}
pub fn snapshot(root: &Path) -> Result<Settings, String> {
    let mut value =
        serde_json::to_value(crate::native_preferences::load(root)?).map_err(|e| e.to_string())?;
    let providers = read(&root.join("conversations/providers.json"), json!({}))?;
    for key in ["providers", "defaultModels"] {
        if let Some(v) = providers.get(key) {
            value[key] = v.clone();
        }
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}
pub fn export_configuration(root: &Path, destination: &Path) -> Result<(), String> {
    let _guard = backup::OPERATION
        .try_lock()
        .map_err(|_| "备份或恢复正在进行")?;
    let settings = normalize(snapshot(root)?);
    let sets = read(&root.join("conversations/sets.json"), json!({"sets":[]}))?;
    let assistants = read(
        &root.join("conversations/assistants.json"),
        json!({"assistants":[]}),
    )?;
    let mut catalog: SetCatalogBackup =
        serde_json::from_value(json!({"sets":sets["sets"],"assistants":assistants["assistants"]}))
            .map_err(|e| e.to_string())?;
    catalog.validate()?;
    let oauth = crate::provider_oauth::export_credentials(&settings.providers)?;
    let value = json!({"app":"kivio", "type":"settings-backup", "version":1, "settings":settings,"sets":catalog,"providerOAuth":oauth});
    crate::file_io::atomic_write(
        destination,
        &serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
        "configuration backup",
    )
}
fn pending_path(root: &Path) -> PathBuf {
    root.with_file_name(format!(
        "{}.configuration-restore.json",
        root.file_name().unwrap_or_default().to_string_lossy()
    ))
}
pub fn pending(root: &Path) -> bool {
    pending_path(root).exists() || backup::restore_dir(root).join("ready.json").exists()
}
pub fn stage_configuration(root: &Path, source: &Path, trusted: bool) -> Result<(), String> {
    let _guard = backup::OPERATION
        .try_lock()
        .map_err(|_| "备份或恢复正在进行")?;
    if pending(root) {
        return Err("已有待恢复备份，请先重启完成恢复".into());
    }
    if std::fs::metadata(source).map_err(|e| e.to_string())?.len() > 512 * 1024 * 1024 {
        return Err("配置备份超过 512 MiB 上限".into());
    }
    let value = read(source, Value::Null)?;
    if value["app"] != "kivio" || value["type"] != "settings-backup" || value["version"] != 1 {
        return Err("不支持的 Kivio 配置备份格式或版本".into());
    }
    let local = snapshot(root)?;
    let mut settings: Settings =
        serde_json::from_value(value["settings"].clone()).map_err(|e| e.to_string())?;
    backup::prepare_imported_settings(&mut settings, trusted, &local.chat.external_cli_agents);
    let mut settings = normalize(settings);
    let mut files = BTreeMap::new();
    if let Some(catalog) = value.get("sets") {
        let mut catalog: SetCatalogBackup =
            serde_json::from_value(catalog.clone()).map_err(|e| e.to_string())?;
        catalog.validate()?;
        let old_sets = read(&root.join("conversations/sets.json"), json!({"sets":[]}))?;
        let old_assistants = read(
            &root.join("conversations/assistants.json"),
            json!({"assistants":[]}),
        )?;
        let mut old: SetCatalogBackup = serde_json::from_value(
            json!({"sets":old_sets["sets"], "assistants":old_assistants["assistants"]}),
        )
        .map_err(|e| e.to_string())?;
        old.sets
            .retain(|s| !catalog.sets.iter().any(|new| new.id == s.id));
        old.assistants
            .retain(|s| !catalog.assistants.iter().any(|new| new.id == s.id));
        catalog.sets.extend(old.sets);
        catalog.assistants.extend(old.assistants);
        files.insert(
            "conversations/sets.json".to_owned(),
            json!({"sets":catalog.sets}),
        );
        files.insert(
            "conversations/assistants.json".to_owned(),
            json!({"assistants":catalog.assistants}),
        );
    }
    let imported = crate::provider_oauth::import_credentials(
        &mut settings.providers,
        value.get("providerOAuth").unwrap_or(&json!({})),
    )?;
    let prepared = (|| {
        let mut doc = read(&root.join("settings.json"), json!({}))?;
        doc["settings"] = serde_json::to_value(&settings).map_err(|e| e.to_string())?;
        files.insert("settings.json".to_owned(), doc);
        let mut providers = read(&root.join("conversations/providers.json"), json!({}))?;
        providers["providers"] =
            serde_json::to_value(&settings.providers).map_err(|e| e.to_string())?;
        providers["defaultModels"] =
            serde_json::to_value(&settings.default_models).map_err(|e| e.to_string())?;
        files.insert("conversations/providers.json".to_owned(), providers);
        crate::file_io::atomic_write(
            &pending_path(root),
            &serde_json::to_string(&json!({"id":uuid::Uuid::new_v4().to_string(),"files":files}))
                .map_err(|e| e.to_string())?,
            "configuration restore",
        )
    })();
    if prepared.is_err() {
        crate::provider_oauth::discard_imported_credentials(&imported);
    }
    prepared
}
/// Replay is idempotent. The pending journal remains until every file has been
/// durably replaced. The original file set is retained outside the data root.
pub fn apply_pending(root: &Path) -> Result<(), String> {
    backup::apply_pending(root)?;
    let journal = pending_path(root);
    if !journal.exists() {
        return Ok(());
    }
    let transaction = read(&journal, Value::Null)?;
    let id = transaction["id"]
        .as_str()
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .ok_or("无效配置恢复事务编号")?;
    let files: BTreeMap<String, Value> =
        serde_json::from_value(transaction["files"].clone()).map_err(|e| e.to_string())?;
    let allowed = [
        "settings.json",
        "conversations/providers.json",
        "conversations/sets.json",
        "conversations/assistants.json",
    ];
    if files.keys().any(|key| !allowed.contains(&key.as_str()))
        || !files.contains_key("settings.json")
    {
        return Err("无效配置恢复事务".into());
    }
    let recovery = root.with_file_name(format!(
        "{}.configuration-before-restore-{id}",
        root.file_name().unwrap_or_default().to_string_lossy()
    ));
    if !recovery.exists() {
        let mut originals = BTreeMap::new();
        for path in files.keys() {
            originals.insert(path, read(&root.join(path), Value::Null)?);
        }
        crate::file_io::atomic_write(
            &recovery,
            &serde_json::to_string(&originals).map_err(|e| e.to_string())?,
            "configuration recovery",
        )?;
    }
    for (path, value) in files {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        crate::file_io::atomic_write(
            &target,
            &serde_json::to_string(&value).map_err(|e| e.to_string())?,
            "configuration restore",
        )?;
    }
    std::fs::remove_file(journal).map_err(|e| e.to_string())
}
pub fn export_full(root: &Path, destination: &Path) -> Result<Vec<String>, String> {
    let _guard = backup::OPERATION
        .try_lock()
        .map_err(|_| "备份或恢复正在进行")?;
    if pending(root) {
        return Err("请先重启完成恢复".into());
    }
    let settings = snapshot(root)?;
    crate::provider_oauth::export_credentials(&settings.providers)?;
    backup::export_to(
        root,
        &PathBuf::from(settings.chat_tools.native_tools.working_directory),
        destination,
        &Default::default(),
    )
}
pub fn stage_full(root: &Path, source: &Path, trusted: bool) -> Result<(), String> {
    let _guard = backup::OPERATION
        .try_lock()
        .map_err(|_| "备份或恢复正在进行")?;
    if pending(root) {
        return Err("请先重启完成恢复".into());
    }
    backup::stage_restore(root, source, trusted, &snapshot(root)?, normalize)
}

// The native host can retain configuration for CLI definitions whose execution
// adapters are still assembled elsewhere. Never erase those during restore.
fn normalize(settings: Settings) -> Settings {
    let cli = settings.chat.external_cli_agents.clone();
    let mut settings = sanitize_settings(settings);
    for (id, config) in cli {
        if crate::external_agents::registry::get_agent_def(&id).is_none() {
            settings.chat.external_cli_agents.insert(id, config);
        }
    }
    settings
}
