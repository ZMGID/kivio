//! Portable set snapshots. Called under the conversation repository's exclusive barrier.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

const MAX_BACKUP_BYTES: u64 = 512 * 1024 * 1024;

/// The set catalog travels with settings so restoring preferences also restores
/// their named personas and default assistants.
#[derive(Serialize, Deserialize)]
pub(crate) struct SetCatalogBackup {
    sets: Vec<ChatSet>,
    assistants: Vec<ChatAssistant>,
}

pub(crate) fn export_catalog_in(root: &Path) -> Result<SetCatalogBackup, String> {
    let _catalog = catalog_mutation_lock();
    let dir = root.join("conversations");
    let sets: ChatSetIndex = read_json(&dir.join("sets.json"))?;
    let assistants: ChatAssistantIndex = read_json(&dir.join("assistants.json"))?;
    let ids: HashSet<_> = sets
        .sets
        .iter()
        .filter_map(|set| set.default_assistant_id.as_deref())
        .collect();
    Ok(SetCatalogBackup {
        assistants: assistants
            .assistants
            .into_iter()
            .filter(|assistant| ids.contains(assistant.id.as_str()))
            .collect(),
        sets: sets.sets,
    })
}

pub(crate) fn import_catalog_in(root: &Path, mut backup: SetCatalogBackup) -> Result<(), String> {
    let _catalog = catalog_mutation_lock();
    let dir = root.join("conversations");
    let mut ids = HashSet::new();
    for set in &mut backup.sets {
        sets::validate_set_id(&set.id)?;
        set.name = sets::normalize_set_name(&set.name)?;
        if !ids.insert(set.id.clone()) {
            return Err("备份包含重复集".into());
        }
    }
    let existing: ChatSetIndex = read_json(&dir.join("sets.json"))?;
    backup.sets.extend(
        existing
            .sets
            .into_iter()
            .filter(|set| !ids.contains(&set.id)),
    );
    let mut assistants: ChatAssistantIndex = read_json(&dir.join("assistants.json"))?;
    let assistant_ids: HashSet<_> = backup
        .assistants
        .iter()
        .map(|assistant| assistant.id.clone())
        .collect();
    assistants
        .assistants
        .retain(|assistant| !assistant_ids.contains(&assistant.id));
    assistants.assistants.extend(backup.assistants);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    commit_import(
        &[],
        &[
            (dir.join("assistants.json"), json(&assistants)?),
            (
                dir.join("sets.json"),
                json(&ChatSetIndex { sets: backup.sets })?,
            ),
        ],
    )
}

#[derive(Serialize, Deserialize)]
struct SetBackup {
    app: String,
    version: u32,
    #[serde(rename = "type")]
    kind: String,
    set: ChatSet,
    assistants: Vec<ChatAssistant>,
    conversations: Vec<Conversation>,
    pins: Vec<ConversationPin>,
    files: Vec<BackupFile>,
    // Native sessions remain owned by the CLI. Preserve Kivio's binding records,
    // and refuse to duplicate an existing bound conversation on restore.
    bindings: HashMap<String, HashMap<String, Value>>,
}

#[derive(Serialize, Deserialize)]
struct BackupFile {
    conversation_id: String,
    path: String,
    absolute: bool,
    data: String,
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn file_paths(conversation: &Conversation) -> HashSet<String> {
    let mut paths = HashSet::new();
    for message in &conversation.messages {
        for model_message in &message.model_messages {
            for part in &model_message.content {
                if let crate::chat::model::MessagePart::Image {
                    path: Some(path), ..
                }
                | crate::chat::model::MessagePart::Video {
                    path: Some(path), ..
                } = part
                {
                    paths.insert(path.clone());
                }
            }
        }
        fn collect_uris(value: &Value, paths: &mut HashSet<String>) {
            match value {
                Value::String(raw) => {
                    if let Some(uri) = raw.strip_prefix("kivio-attachment://") {
                        if let Some((_, filename)) = uri.rsplit_once('/') {
                            paths.insert(filename.to_string());
                        }
                    }
                }
                Value::Array(values) => values.iter().for_each(|v| collect_uris(v, paths)),
                Value::Object(values) => values.values().for_each(|v| collect_uris(v, paths)),
                _ => {}
            }
        }
        for message in &message.api_messages {
            collect_uris(message, &mut paths);
        }
        for attachment in &message.attachments {
            if attachment.attachment_type != "folder" && !attachment.path.starts_with("memory://") {
                paths.insert(attachment.path.clone());
            }
        }
        for artifact in message
            .artifacts
            .iter()
            .chain(message.tool_calls.iter().flat_map(|t| &t.artifacts))
        {
            if let Some(path) = &artifact.path {
                paths.insert(path.clone());
            }
        }
    }
    paths
}

fn resolve_file(dir: &Path, id: &str, raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if raw.is_empty() || raw == "." || raw == ".." || raw.contains(['/', '\\', ':']) {
        return Err(format!("Invalid attachment path: {raw}"));
    }
    Ok(dir.join(format!("{id}_attachments")).join(path))
}

pub(crate) fn export_in(root: &Path, set_id: &str, path: &Path) -> Result<(), String> {
    let _catalog = catalog_mutation_lock();
    sets::validate_set_id(set_id)?;
    let dir = root.join("conversations");
    let index: ChatSetIndex = read_json(&dir.join("sets.json"))?;
    let set = index
        .sets
        .into_iter()
        .find(|s| s.id == set_id)
        .ok_or("集不存在")?;
    let mut conversations = Vec::new();
    // Read actual files, including archived chats and chats absent from a stale index.
    for id in index::conversation_file_ids_in_dir(&dir)? {
        let conversation = read_conversation_file(&dir.join(format!("{id}.json")), &id)?;
        if conversation.set_id.as_deref() == Some(set_id) {
            conversations.push(conversation);
        }
    }
    let mut assistant_ids: HashSet<_> = conversations
        .iter()
        .filter_map(|c| c.assistant_id.clone())
        .collect();
    assistant_ids.extend(set.default_assistant_id.clone());
    let assistants: ChatAssistantIndex = read_json(&dir.join("assistants.json"))?;
    let assistants = assistants
        .assistants
        .into_iter()
        .filter(|a| assistant_ids.contains(&a.id))
        .collect();
    let mut pins: HashMap<String, Vec<ConversationPin>> =
        read_json(&dir.join("conversation-pins.json"))?;
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut bindings = HashMap::new();
    for conversation in &conversations {
        validate_conversation_id(&conversation.id)?;
        let mut paths = file_paths(conversation);
        let attachment_dir = dir.join(format!("{}_attachments", conversation.id));
        if attachment_dir.exists() {
            for entry in fs::read_dir(&attachment_dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    paths.insert(
                        entry
                            .file_name()
                            .to_str()
                            .ok_or("附件文件名不是 UTF-8")?
                            .to_string(),
                    );
                }
            }
        }
        for raw in paths {
            let source = resolve_file(&dir, &conversation.id, &raw)?;
            total += fs::metadata(&source)
                .map_err(|e| format!("附件无法备份 {}: {e}", source.display()))?
                .len();
            if total > MAX_BACKUP_BYTES / 2 {
                return Err("集附件过大（上限 256 MiB）".into());
            }
            let data =
                fs::read(&source).map_err(|e| format!("附件无法备份 {}: {e}", source.display()))?;
            files.push(BackupFile {
                conversation_id: conversation.id.clone(),
                absolute: Path::new(&raw).is_absolute(),
                path: raw,
                data: STANDARD.encode(data),
            });
        }
        let mut records = HashMap::new();
        for prefix in ["", "live-", "imported-"] {
            let file = root
                .join("external-agent-sessions")
                .join(format!("{prefix}{}.json", conversation.id));
            if file.exists() {
                records.insert(prefix.to_string(), read_json::<Value>(&file)?);
            }
        }
        if !records.is_empty() {
            bindings.insert(conversation.id.clone(), records);
        }
    }
    let backup = SetBackup {
        app: "kivio".into(),
        version: 1,
        kind: "set-backup".into(),
        set,
        assistants,
        conversations,
        pins: pins.remove(set_id).unwrap_or_default(),
        files,
        bindings,
    };
    let raw = json(&backup)?;
    if raw.len() as u64 > MAX_BACKUP_BYTES {
        return Err("集备份过大（上限 512 MiB）".into());
    }
    atomic_write(path, &raw, "set backup")
}

fn unique_name(name: &str, existing: &HashSet<String>, limit: usize) -> String {
    if !existing.contains(name) {
        return name.into();
    }
    for i in 2.. {
        let suffix = format!(" ({i})");
        let candidate = format!(
            "{}{}",
            name.chars()
                .take(limit.saturating_sub(suffix.len()))
                .collect::<String>(),
            suffix
        );
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

// Replace exact stored path values only; never rewrite user prose or substrings.
fn rewrite_paths(value: &mut Value, paths: &HashMap<String, String>) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|v| rewrite_paths(v, paths)),
        Value::Object(items) => {
            for (key, value) in items {
                if matches!(key.as_str(), "path" | "file_path" | "url") {
                    if let Some(next) = value.as_str().and_then(|s| paths.get(s)) {
                        *value = Value::String(next.clone());
                    }
                } else {
                    rewrite_paths(value, paths);
                }
            }
        }
        _ => {}
    }
}

fn binding_key(record: &Value) -> Option<(String, String)> {
    let agent = record
        .get("agentId")
        .or_else(|| record.get("agent_id"))?
        .as_str()?;
    let session = record
        .get("sessionId")
        .or_else(|| record.get("session_id"))
        .or_else(|| record.get("native_id"))?
        .as_str()?;
    if agent.trim().is_empty() || session.trim().is_empty() {
        return None;
    }
    Some((agent.to_string(), session.to_string()))
}

pub(crate) fn import_in(root: &Path, path: &Path) -> Result<ChatSet, String> {
    let _catalog = catalog_mutation_lock();
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_BACKUP_BYTES {
        return Err("集备份过大".into());
    }
    let mut backup: SetBackup = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("集备份无法解析: {e}"))?;
    if backup.app != "kivio" || backup.kind != "set-backup" || backup.version != 1 {
        return Err("不支持的集备份格式或版本".into());
    }
    sets::validate_set_id(&backup.set.id)?;
    backup.set.name = sets::normalize_set_name(&backup.set.name)?;
    let dir = root.join("conversations");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut sets: ChatSetIndex = read_json(&dir.join("sets.json"))?;
    let mut assistants: ChatAssistantIndex = read_json(&dir.join("assistants.json"))?;
    let (mut index, _) = index::load_index_or_scan_in_dir(&dir)?;
    let mut pins: HashMap<String, Vec<ConversationPin>> =
        read_json(&dir.join("conversation-pins.json"))?;
    let mut ids = HashMap::new();
    for conversation in &backup.conversations {
        validate_conversation_id(&conversation.id)?;
        if conversation.set_id.as_deref() != Some(&backup.set.id)
            || ids.contains_key(&conversation.id)
        {
            return Err("备份包含重复或不属于该集的对话".into());
        }
        let bound = backup.bindings.contains_key(&conversation.id);
        let next = if bound {
            conversation.id.clone()
        } else {
            format!("conv_{}", uuid::Uuid::new_v4())
        };
        if bound
            && (dir.join(format!("{next}.json")).exists()
                || ["", "live-", "imported-"].iter().any(|p| {
                    root.join("external-agent-sessions")
                        .join(format!("{p}{next}.json"))
                        .exists()
                }))
        {
            return Err(
                "备份中的外部 CLI 对话仍存在，不能复制原生会话绑定。请在恢复原对话时导入。".into(),
            );
        }
        ids.insert(conversation.id.clone(), next);
    }
    let session_dir = root.join("external-agent-sessions");
    let mut native_bindings = HashSet::new();
    if session_dir.exists() {
        for entry in fs::read_dir(&session_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(key) = binding_key(&read_json::<Value>(&entry.path())?) {
                    native_bindings.insert(key);
                }
            }
        }
    }
    let mut incoming_bindings = HashMap::new();
    for (id, records) in &backup.bindings {
        let conversation = backup
            .conversations
            .iter()
            .find(|c| &c.id == id)
            .ok_or("会话绑定所属对话不存在")?;
        if !conversation.agent_runtime.is_external() || records.is_empty() {
            return Err("备份中的 CLI 会话绑定无效".into());
        }
        for (prefix, record) in records {
            match prefix.as_str() {
                "live-" => {
                    serde_json::from_value::<crate::external_agents::session::LiveSessionHandle>(
                        record.clone(),
                    )
                    .map_err(|e| format!("CLI 绑定无法解析: {e}"))?;
                }
                "imported-" => {
                    serde_json::from_value::<crate::external_agents::import::ImportRecord>(
                        record.clone(),
                    )
                    .map_err(|e| format!("CLI 导入记录无法解析: {e}"))?;
                }
                "" => {
                    serde_json::from_value::<crate::external_agents::types::ExternalAgentSession>(
                        record.clone(),
                    )
                    .map_err(|e| format!("CLI 会话无法解析: {e}"))?;
                }
                _ => return Err("无效的会话绑定".into()),
            }

            let key = binding_key(record).ok_or("备份中的 CLI 会话绑定无效")?;
            if conversation.agent_runtime.external_agent_id.as_deref() != Some(key.0.as_str()) {
                return Err("备份中的 CLI 会话与代理不匹配".into());
            }
            if native_bindings.contains(&key)
                || incoming_bindings
                    .insert(key, id)
                    .is_some_and(|previous| previous != id)
            {
                return Err("该 CLI 原生会话已有绑定，不能重复恢复".into());
            }
            if record
                .get("conversationId")
                .or_else(|| record.get("conversation_id"))
                .and_then(Value::as_str)
                .is_some_and(|record_id| record_id != id)
            {
                return Err("备份中的 CLI 对话标识不匹配".into());
            }
            if let Some(cwd) = record.get("cwd").and_then(Value::as_str) {
                if !Path::new(cwd).is_dir() {
                    return Err(format!("CLI 原工作目录不存在：{cwd}"));
                }
            }
        }
    }
    let mut new_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut mapped_paths: HashMap<String, HashMap<String, String>> = HashMap::new();
    for file in &backup.files {
        let id = ids.get(&file.conversation_id).ok_or("附件所属对话不存在")?;
        let extension = Path::new(&file.path)
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("bin");
        // Keep relative filenames: model replay uses kivio-attachment:// URIs.
        if !file.absolute && Path::new(&file.path).is_absolute() {
            return Err("无效的相对附件路径".into());
        }
        let attachment_dir = dir.join(format!("{id}_attachments"));
        if fs::symlink_metadata(&attachment_dir).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("恢复目标附件目录不能是符号链接".into());
        }
        let target = if file.absolute {
            dir.join(format!("{id}_attachments")).join(format!(
                "{}.{}",
                uuid::Uuid::new_v4(),
                extension
            ))
        } else {
            resolve_file(&dir, id, &file.path)?
        };
        let data = STANDARD
            .decode(&file.data)
            .map_err(|e| format!("附件已损坏: {e}"))?;
        if mapped_paths
            .entry(file.conversation_id.clone())
            .or_default()
            .insert(
                file.path.clone(),
                if file.absolute {
                    target.to_string_lossy().into_owned()
                } else {
                    file.path.clone()
                },
            )
            .is_some()
        {
            return Err("备份包含重复附件".into());
        }
        new_files.push((target, data));
    }
    for conversation in &backup.conversations {
        for path in file_paths(conversation) {
            if !mapped_paths
                .get(&conversation.id)
                .is_some_and(|m| m.contains_key(&path))
            {
                return Err(format!("备份缺少附件: {path}"));
            }
        }
    }
    for (id, records) in &backup.bindings {
        let next = ids.get(id).ok_or("会话绑定所属对话不存在")?;
        for (prefix, record) in records {
            if !["", "live-", "imported-"].contains(&prefix.as_str()) {
                return Err("无效的会话绑定".into());
            }
            let mut record = record.clone();
            // Keep embedded conversation identities aligned with the restored filename.
            for key in ["conversationId", "conversation_id"] {
                if let Some(value) = record.get_mut(key) {
                    *value = Value::String(next.clone());
                }
            }
            new_files.push((
                root.join("external-agent-sessions")
                    .join(format!("{prefix}{next}.json")),
                json(&record)?.into_bytes(),
            ));
        }
    }
    let mut assistant_ids = HashMap::new();
    let mut assistant_names: HashSet<_> = assistants
        .assistants
        .iter()
        .map(|a| a.name.clone())
        .collect();
    for mut assistant in backup.assistants {
        let old = assistant.id.clone();
        if assistant_ids.contains_key(&old) {
            return Err("备份包含重复助手".into());
        }
        assistant.id = format!("asst_{}", uuid::Uuid::new_v4());
        assistant.name = unique_name(&assistant.name, &assistant_names, 64);
        assistant.built_in = false;
        assistant.source = "imported".into();
        assistant_names.insert(assistant.name.clone());
        assistant_ids.insert(old, assistant.id.clone());
        assistants.assistants.push(assistant);
    }
    backup.set.id = format!("set_{}", uuid::Uuid::new_v4());
    backup.set.name = unique_name(
        &backup.set.name,
        &sets.sets.iter().map(|s| s.name.clone()).collect(),
        80,
    );
    backup.set.default_assistant_id = backup
        .set
        .default_assistant_id
        .and_then(|id| assistant_ids.get(&id).cloned());
    for mut conversation in backup.conversations {
        let paths = mapped_paths.remove(&conversation.id).unwrap_or_default();
        conversation.id = ids[&conversation.id].clone();
        conversation.revision = 1;
        conversation.set_id = Some(backup.set.id.clone());
        conversation.project_id = None;
        conversation.folder = None;
        conversation.assistant_id = conversation
            .assistant_id
            .and_then(|id| assistant_ids.get(&id).cloned());
        if let Some(snapshot) = &mut conversation.assistant_snapshot {
            if let Some(id) = assistant_ids.get(&snapshot.id) {
                snapshot.id = id.clone();
            }
        }
        if let Some(fork) = &mut conversation.forked_from {
            if let Some(id) = ids.get(&fork.conversation_id) {
                fork.conversation_id = id.clone();
            }
        }
        if let Some(goal) = &mut conversation.goal_state {
            if crate::chat::goal::is_running(goal.status) {
                goal.status = crate::chat::GoalStatus::Paused;
                goal.active_run_id = None;
            }
        }
        let mut value = serde_json::to_value(&conversation).map_err(|e| e.to_string())?;
        rewrite_paths(&mut value, &paths);
        let conversation: Conversation =
            serde_json::from_value(value).map_err(|e| e.to_string())?;
        index
            .conversations
            .push(ConversationListItem::from(&conversation));
        new_files.push((
            dir.join(format!("{}.json", conversation.id)),
            json(&conversation)?.into_bytes(),
        ));
    }
    pins.insert(
        backup.set.id.clone(),
        backup
            .pins
            .into_iter()
            .filter_map(|mut pin| {
                pin.id = ids.get(&pin.id)?.clone();
                Some(pin)
            })
            .collect(),
    );
    sets.sets.insert(0, backup.set.clone());
    let catalogs = [
        (dir.join("assistants.json"), json(&assistants)?),
        (dir.join("conversation-pins.json"), json(&pins)?),
        (dir.join("index.json"), json(&index)?),
        (dir.join("sets.json"), json(&sets)?),
    ];
    commit_import(&new_files, &catalogs)?;
    index::forget_index_cache(&dir);
    Ok(backup.set)
}

/// New identities never overwrite data. Catalog writes are rolled back on a reported
/// failure; publish the set last, after all of its content is durably written.
fn commit_import(
    files: &[(PathBuf, Vec<u8>)],
    catalogs: &[(PathBuf, String)],
) -> Result<(), String> {
    let originals: Vec<_> = catalogs
        .iter()
        .map(|(path, _)| match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        })
        .collect::<Result<_, _>>()?;
    let mut created = Vec::new();
    let mut written_catalogs = 0;
    let result = (|| {
        for (path, bytes) in files {
            fs::create_dir_all(path.parent().ok_or("无效路径")?).map_err(|e| e.to_string())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            created.push(path.clone());
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        for (path, content) in catalogs {
            atomic_write(path, content, "set import")?;
            written_catalogs += 1;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut errors = vec![error];
        for i in (0..written_catalogs).rev() {
            let path = &catalogs[i].0;
            let restored = match &originals[i] {
                Some(bytes) => atomic_write(
                    path,
                    std::str::from_utf8(bytes).map_err(|e| e.to_string())?,
                    "restore catalog",
                ),
                None => fs::remove_file(path).map_err(|e| e.to_string()),
            };
            if let Err(e) = restored {
                errors.push(e);
            }
        }
        for path in created.iter().rev() {
            if let Err(e) = fs::remove_file(path) {
                errors.push(e.to_string());
            }
            if let Some(parent) = path.parent().filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with("_attachments"))
            }) {
                let _ = fs::remove_dir(parent);
            }
        }
        return Err(errors.join("; "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(root: &Path) {
        let dir = root.join("conversations");
        fs::create_dir_all(dir.join("conv_one_attachments")).unwrap();
        fs::write(
            dir.join("conv_one_attachments/note.txt"),
            b"attachment bytes",
        )
        .unwrap();
        fs::write(
            dir.join("conv_one_attachments/msgimg-test.png"),
            b"model image bytes",
        )
        .unwrap();
        let values = [
            (
                "sets.json",
                json!({"sets":[{"id":"set_one","name":"写作","system_prompt":"保留提示词","default_assistant_id":"asst_one","color":"#abcdef","created_at":1,"updated_at":2}]}),
            ),
            (
                "assistants.json",
                json!({"assistants":[{"id":"asst_one","name":"编辑","system_prompt":"保留助手提示词","created_at":1,"updated_at":2}]}),
            ),
            (
                "conversation-pins.json",
                json!({"set_one":[{"id":"conv_one","row":3}]}),
            ),
            (
                "conv_one.json",
                json!({"id":"conv_one","title":"归档对话","provider_id":"p","model":"m","set_id":"set_one","assistant_id":"asst_one","archived":true,"created_at":1,"updated_at":2,"messages":[{"id":"msg_one","role":"user","content":"note.txt","timestamp":1,"attachments":[{"id":"file_one","type":"file","name":"note.txt","path":"note.txt"},{"id":"memory_one","type":"file","name":"文本","path":"memory://one","content":"内存正文"}],"model_messages":[{"role":"user","content":[{"type":"image","mime_type":"image/png","path":"msgimg-test.png"}]}],"api_messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"kivio-attachment://image/png/msgimg-test.png"}}]}]}]}),
            ),
            (
                "conv_unrelated.json",
                json!({"id":"conv_unrelated","title":"其他集","provider_id":"p","model":"m","created_at":1,"updated_at":2,"messages":[]}),
            ),
        ];
        for (name, value) in values {
            fs::write(dir.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        }
    }

    #[test]
    fn settings_catalog_round_trip_restores_sets_and_default_assistants_in_order() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let mut sets: ChatSetIndex =
            read_json(&source.path().join("conversations/sets.json")).unwrap();
        let mut second = sets.sets[0].clone();
        second.id = "set_two".into();
        second.name = "阅读".into();
        sets.sets.push(second);
        fs::write(
            source.path().join("conversations/sets.json"),
            json(&sets).unwrap(),
        )
        .unwrap();
        let exported = export_catalog_in(source.path()).unwrap();
        let raw = serde_json::to_vec(&exported).unwrap();
        let target = tempfile::tempdir().unwrap();
        import_catalog_in(target.path(), serde_json::from_slice(&raw).unwrap()).unwrap();
        // Importing twice updates the same identities instead of multiplying sets.
        import_catalog_in(target.path(), serde_json::from_slice(&raw).unwrap()).unwrap();
        let restored: ChatSetIndex =
            read_json(&target.path().join("conversations/sets.json")).unwrap();
        assert_eq!(
            restored
                .sets
                .iter()
                .map(|set| set.name.as_str())
                .collect::<Vec<_>>(),
            ["写作", "阅读"]
        );
        assert_eq!(restored.sets[0].system_prompt, "保留提示词");
        assert_eq!(
            restored.sets[0].default_assistant_id.as_deref(),
            Some("asst_one")
        );
        let assistants: ChatAssistantIndex =
            read_json(&target.path().join("conversations/assistants.json")).unwrap();
        assert_eq!(assistants.assistants.len(), 1);
        assert_eq!(assistants.assistants[0].system_prompt, "保留助手提示词");
    }

    #[test]
    fn set_backup_round_trip_preserves_archived_chats_files_and_assistant_without_overwrite() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let backup = source.path().join("backup.json");
        export_in(source.path(), "set_one", &backup).unwrap();
        let original = fs::read(source.path().join("conversations/conv_one.json")).unwrap();
        let imported = import_in(source.path(), &backup).unwrap();
        assert_eq!(imported.name, "写作 (2)");
        assert_eq!(imported.system_prompt, "保留提示词");
        assert_eq!(imported.color.as_deref(), Some("#abcdef"));
        assert_ne!(imported.id, "set_one");
        assert_eq!(
            fs::read(source.path().join("conversations/conv_one.json")).unwrap(),
            original
        );
        let index: ConversationIndex =
            read_json(&source.path().join("conversations/index.json")).unwrap();
        let chats: Vec<_> = index
            .conversations
            .iter()
            .filter(|c| c.set_id.as_deref() == Some(&imported.id))
            .collect();
        assert_eq!(chats.len(), 1);
        let conversation = read_conversation_file(
            &source
                .path()
                .join(format!("conversations/{}.json", chats[0].id)),
            &chats[0].id,
        )
        .unwrap();
        assert!(conversation.archived);
        assert_eq!(conversation.messages[0].content, "note.txt");
        assert_eq!(conversation.messages[0].attachments[0].name, "note.txt");
        assert_eq!(
            fs::read(
                resolve_file(
                    &source.path().join("conversations"),
                    &conversation.id,
                    &conversation.messages[0].attachments[0].path
                )
                .unwrap()
            )
            .unwrap(),
            b"attachment bytes"
        );
        assert_eq!(
            conversation.messages[0].attachments[1].content.as_deref(),
            Some("内存正文")
        );
        assert_eq!(
            fs::read(source.path().join(format!(
                "conversations/{}_attachments/msgimg-test.png",
                conversation.id
            )))
            .unwrap(),
            b"model image bytes"
        );
        assert!(
            matches!(&conversation.messages[0].model_messages[0].content[0], crate::chat::model::MessagePart::Image { path: Some(path), .. } if path == "msgimg-test.png")
        );
        assert_eq!(
            conversation.messages[0].api_messages[0]["content"][0]["image_url"]["url"],
            "kivio-attachment://image/png/msgimg-test.png"
        );
        let assistants: ChatAssistantIndex =
            read_json(&source.path().join("conversations/assistants.json")).unwrap();
        let assistant = assistants
            .assistants
            .iter()
            .find(|a| Some(&a.id) == imported.default_assistant_id.as_ref())
            .unwrap();
        assert_eq!(assistant.system_prompt, "保留助手提示词");
        assert_eq!(
            conversation.assistant_id.as_deref(),
            Some(assistant.id.as_str())
        );
        let pins: HashMap<String, Vec<ConversationPin>> =
            read_json(&source.path().join("conversations/conversation-pins.json")).unwrap();
        assert_eq!(pins[&imported.id][0].id, conversation.id);
        assert_eq!(pins[&imported.id][0].row, 3);
        // Also portable into an empty app data directory, independent of the originals.
        // The source may use a different operating system's absolute paths.
        let mut portable: Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
        let file = portable["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|file| file["path"] == "note.txt")
            .unwrap();
        file["path"] = json!(r"C:\old-machine\note.txt");
        file["absolute"] = json!(true);
        portable["conversations"][0]["messages"][0]["attachments"][0]["path"] =
            json!(r"C:\old-machine\note.txt");
        fs::write(&backup, portable.to_string()).unwrap();
        let target = tempfile::tempdir().unwrap();
        let imported = import_in(target.path(), &backup).unwrap();
        assert_eq!(imported.name, "写作");
        let index: ConversationIndex =
            read_json(&target.path().join("conversations/index.json")).unwrap();
        assert_eq!(index.conversations.len(), 1);
        let id = &index.conversations[0].id;
        let restored =
            read_conversation_file(&target.path().join(format!("conversations/{id}.json")), id)
                .unwrap();
        let path = Path::new(&restored.messages[0].attachments[0].path);
        assert!(path.starts_with(target.path()));
        assert_eq!(fs::read(path).unwrap(), b"attachment bytes");
    }

    #[test]
    fn empty_set_still_exports_its_prompt_and_default_assistant() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        fs::remove_file(source.path().join("conversations/conv_one.json")).unwrap();
        let backup = source.path().join("backup.json");
        export_in(source.path(), "set_one", &backup).unwrap();
        let target = tempfile::tempdir().unwrap();
        let set = import_in(target.path(), &backup).unwrap();
        assert_eq!(set.system_prompt, "保留提示词");
        assert!(set.default_assistant_id.is_some());
        let index: ConversationIndex =
            read_json(&target.path().join("conversations/index.json")).unwrap();
        assert!(index.conversations.is_empty());
    }

    #[test]
    fn invalid_backup_fails_before_changing_existing_data() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let backup = source.path().join("backup.json");
        export_in(source.path(), "set_one", &backup).unwrap();
        let raw: Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
        let before = fs::read(source.path().join("conversations/sets.json")).unwrap();
        for kind in [
            "version",
            "traversal",
            "missing_file",
            "duplicate",
            "bad_base64",
            "absolute_as_relative",
        ] {
            let mut value = raw.clone();
            match kind {
                "version" => value["version"] = json!(999),
                "traversal" => value["files"][0]["path"] = json!("../../outside.txt"),
                "missing_file" => value["files"] = json!([]),
                "absolute_as_relative" => {
                    value["files"][0]["path"] = json!(source.path().join("outside.txt"));
                    value["files"][0]["absolute"] = json!(false);
                }
                "duplicate" => {
                    let first = value["conversations"][0].clone();
                    value["conversations"].as_array_mut().unwrap().push(first);
                }
                _ => value["files"][0]["data"] = json!("!not-base64!"),
            }
            fs::write(&backup, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(import_in(source.path(), &backup).is_err(), "{kind}");
            assert_eq!(
                fs::read(source.path().join("conversations/sets.json")).unwrap(),
                before
            );
            assert!(!source.path().join("conversations/index.json").exists());
        }
    }

    #[test]
    fn failed_export_preserves_previous_backup_and_failed_import_removes_new_files() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let backup = source.path().join("backup.json");
        fs::write(&backup, b"previous backup").unwrap();
        fs::remove_file(
            source
                .path()
                .join("conversations/conv_one_attachments/note.txt"),
        )
        .unwrap();
        assert!(export_in(source.path(), "set_one", &backup).is_err());
        assert_eq!(fs::read(&backup).unwrap(), b"previous backup");
        fixture(source.path());
        export_in(source.path(), "set_one", &backup).unwrap();
        let target = tempfile::tempdir().unwrap();
        fail_atomic_replace_for_current_test(true);
        let result = import_in(target.path(), &backup);
        fail_atomic_replace_for_current_test(false);
        assert!(result.is_err());
        assert_eq!(
            fs::read_dir(target.path().join("conversations"))
                .unwrap()
                .count(),
            0
        );
        assert!(import_in(target.path(), &backup).is_ok());
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn external_backup_keeps_binding_and_refuses_duplicate_or_missing_original_directory() {
        let source = tempfile::tempdir().unwrap();
        let dir = source.path().join("conversations");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(source.path().join("external-agent-sessions")).unwrap();
        fs::write(
            dir.join("sets.json"),
            json!({"sets":[{"id":"set_cli","name":"CLI","created_at":1,"updated_at":1}]})
                .to_string(),
        )
        .unwrap();
        fs::write(dir.join("conv_cli.json"), json!({"id":"conv_cli","title":"CLI","provider_id":"p","model":"m","set_id":"set_cli","agent_runtime":{"kind":"external","externalAgentId":"codex"},"created_at":1,"updated_at":1,"messages":[]}).to_string()).unwrap();
        let binding = json!({"agent_id":"codex","protocol":"codex_app_server","native_id":"native-one","cwd":source.path()});
        fs::write(
            source
                .path()
                .join("external-agent-sessions/live-conv_cli.json"),
            binding.to_string(),
        )
        .unwrap();
        let backup = source.path().join("backup.json");
        export_in(source.path(), "set_cli", &backup).unwrap();
        assert!(import_in(source.path(), &backup)
            .unwrap_err()
            .contains("不能复制"));
        let target = tempfile::tempdir().unwrap();
        let set = import_in(target.path(), &backup).unwrap();
        let restored = read_conversation_file(
            &target.path().join("conversations/conv_cli.json"),
            "conv_cli",
        )
        .unwrap();
        assert_eq!(restored.set_id.as_deref(), Some(set.id.as_str()));
        assert_eq!(
            read_json::<Value>(
                &target
                    .path()
                    .join("external-agent-sessions/live-conv_cli.json")
            )
            .unwrap(),
            binding
        );
        let target2 = tempfile::tempdir().unwrap();
        let mut raw: Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
        raw["bindings"]["conv_cli"]["live-"]["cwd"] = json!(source.path().join("missing"));
        fs::write(&backup, raw.to_string()).unwrap();
        assert!(import_in(target2.path(), &backup)
            .unwrap_err()
            .contains("原工作目录不存在"));
        assert!(!target2.path().join("conversations/sets.json").exists());
    }
}
