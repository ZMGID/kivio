//! Artifact IDs for model reads/edits, and the Works gallery index.
//! Files stay where they were created. Works indexes delivered chat files and completed media creations.
use super::{ChatMessage, Conversation};
use crate::mcp::types::{ChatToolArtifact, ChatToolDefinition, McpToolCallResult};
use base64::{engine::general_purpose::STANDARD, Engine};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub id: String,
    pub work_id: String,
    pub parent_id: Option<String>,
    pub conversation_id: String,
    pub message_id: String,
    pub title: String,
    pub created_at: i64,
    pub source_tool: String,
    pub delivered: bool,
    pub artifact: ChatToolArtifact,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    #[serde(flatten)]
    pub record: ArtifactRecord,
    pub available: bool,
    pub source_available: bool,
}

#[derive(Serialize)]
pub struct LibraryPage {
    pub items: Vec<LibraryItem>,
    pub warnings: usize,
}

fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("artifacts"))
}

// Media import/list/action never acquire the station lock. Deletion acquires the
// station lock first, then this lock, so a stale import cannot restore a record.
static MEDIA_INDEX: Mutex<()> = Mutex::new(());

pub(crate) fn delete_media_job(app: &AppHandle, media_root: &Path, id: &str) -> Result<(), String> {
    delete_media_job_in(&root(app)?, media_root, id)
}

pub(crate) fn delete_media_job_in(root: &Path, media_root: &Path, id: &str) -> Result<(), String> {
    let _guard = MEDIA_INDEX.lock();
    if uuid::Uuid::parse_str(id).is_err() {
        return Err("Invalid media job ID".into());
    }
    let directory = media_root.join(id);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("Invalid media job directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let prefix = format!("art_media_{id}_");
    match fs::read_dir(root.join("records")) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name
                    .strip_prefix(&prefix)
                    .and_then(|s| s.strip_suffix(".json"))
                    .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                {
                    // Only the index belongs to this job, never a referenced/exported file.
                    fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let mut removed = load_removed(root);
    let previous_len = removed.len();
    removed.retain(|artifact| !artifact.starts_with(&prefix));
    if removed.len() != previous_len {
        super::storage::atomic_write(
            &removed_path(root),
            &serde_json::to_string(&removed).map_err(|e| e.to_string())?,
            "artifact removed",
        )?;
    }
    delete_media_directory(&directory, |path| {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(path).map_err(|e| e.to_string())
        } else {
            fs::remove_file(path).map_err(|e| e.to_string())
        }
    })
}

fn delete_media_directory(
    directory: &Path,
    mut remove_output: impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    // Metadata is the restart-visible record. Keep it until all output cleanup
    // succeeds, even when an earlier output was already removed.
    let metadata = directory.join("job.json");
    let saved = match fs::read_to_string(&metadata) {
        Ok(saved) => Some(saved),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() != "job.json" {
            remove_output(&entry.path())
                .map_err(|error| format!("部分文件清理失败，任务记录已保留：{error}"))?;
        }
    }
    if saved.is_some() {
        fs::remove_file(&metadata).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::remove_dir(directory) {
        if let Some(saved) = saved {
            super::storage::atomic_write(&metadata, &saved, "media job")
                .map_err(|restore| format!("删除目录失败：{error}；恢复任务记录失败：{restore}"))?;
        }
        return Err(format!(
            "部分文件已清理，删除目录失败，任务记录已保留：{error}"
        ));
    }
    Ok(())
}

pub(crate) fn is_valid_artifact_id(id: &str) -> bool {
    id.starts_with("art_")
        && id.len() <= 160
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Read selection and result registration must use the same ordered IDs.
/// Empty optional fields from model calls are not artifact references.
pub(crate) fn input_artifact_ids(arguments: &Value) -> Result<Vec<String>, String> {
    let Some(value) = arguments.get("artifact_ids") else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or("artifact_ids must be an array")?;
    let mut ids = Vec::new();
    for value in values {
        let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) else {
            continue;
        };
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn record_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_valid_artifact_id(id) {
        return Err("Invalid artifact ID".into());
    }
    Ok(root.join("records").join(format!("{id}.json")))
}

fn load(root: &Path, id: &str) -> Result<ArtifactRecord, String> {
    let bytes = fs::read(record_path(root, id)?).map_err(|_| format!("Unknown artifact: {id}"))?;
    serde_json::from_slice(&bytes).map_err(|_| format!("Invalid artifact record: {id}"))
}

fn save(root: &Path, record: &ArtifactRecord) -> Result<(), String> {
    super::storage::atomic_write(
        &record_path(root, &record.id)?,
        &serde_json::to_string(record).map_err(|e| e.to_string())?,
        "artifact",
    )
}

fn removed_path(root: &Path) -> PathBuf {
    root.join("removed.json")
}

fn load_removed(root: &Path) -> HashSet<String> {
    fs::read(removed_path(root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn mark_removed(root: &Path, id: &str) -> Result<(), String> {
    let mut removed = load_removed(root);
    removed.insert(id.to_string());
    super::storage::atomic_write(
        &removed_path(root),
        &serde_json::to_string(&removed).map_err(|e| e.to_string())?,
        "artifact removed",
    )
}

fn sanitized_name(current: &str, proposed: &str) -> Result<String, String> {
    let name = proposed.trim();
    if name.is_empty() || name.len() > 180 {
        return Err("Invalid file name".into());
    }
    if name == "."
        || name == ".."
        || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || name.bytes().any(|b| b < 32)
    {
        return Err("Invalid file name".into());
    }
    if Path::new(name).extension().is_none() {
        if let Some(ext) = Path::new(current).extension().and_then(|e| e.to_str()) {
            return Ok(format!("{name}.{ext}"));
        }
    }
    Ok(name.to_string())
}

fn blob_unused(root: &Path, blob: &str) -> bool {
    let Ok(entries) = fs::read_dir(root.join("records")) else {
        return true;
    };
    !entries.flatten().any(|entry| {
        entry.path().extension().is_some_and(|e| e == "json")
            && fs::read(entry.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ArtifactRecord>(&bytes).ok())
                .is_some_and(|record| record.artifact.path.as_deref() == Some(blob))
    })
}

fn is_managed_blob(root: &Path, path: &str) -> bool {
    let path = Path::new(path);
    ["files", "fileless"]
        .into_iter()
        .any(|name| path.starts_with(root.join(name)))
}

fn unique_persist_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    let ext = Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    for index in 2..=99 {
        let next = dir.join(format!("{stem}-{index}{ext}"));
        if !next.exists() {
            return next;
        }
    }
    dir.join(format!("{stem}-{}{ext}", uuid::Uuid::new_v4().simple()))
}

fn export_artifact(
    root: &Path,
    source: &Path,
    destination: &Path,
    unique: bool,
) -> Result<(), String> {
    if !destination.is_absolute() {
        return Err("Destination must be absolute".into());
    }
    let parent = destination
        .parent()
        .ok_or("Invalid destination")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("Choose a location outside the managed Works folder".into());
    }
    if unique {
        let filename = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Invalid destination")?;
        let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
        for _ in 0..100 {
            let target = unique_persist_path(&parent, filename);
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut output) => {
                    if let Err(error) = std::io::copy(&mut input, &mut output) {
                        drop(output);
                        let _ = fs::remove_file(&target);
                        return Err(error.to_string());
                    }
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        return Err("Could not choose a unique destination".into());
    }
    if destination.canonicalize().ok() != source.canonicalize().ok() {
        fs::copy(source, destination).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn persist_filename(name: &str) -> String {
    sanitized_name(name, name).unwrap_or_else(|_| "artifact.bin".into())
}

fn visible_in_works(record: &ArtifactRecord, conversation_exists: bool) -> bool {
    record.delivered && (conversation_exists || record.source_tool == "media_station")
}

fn conversation_artifact_ids(conversation: &Conversation) -> HashSet<String> {
    conversation
        .messages
        .iter()
        .flat_map(|message| {
            message
                .artifacts
                .iter()
                .chain(
                    message
                        .tool_calls
                        .iter()
                        .flat_map(|tool| tool.artifacts.iter()),
                )
                .filter_map(|artifact| artifact.id.clone())
        })
        .collect()
}

fn drop_record(root: &Path, id: &str, tombstone: bool) -> Result<(), String> {
    let path = record_path(root, id)?;
    if let Ok(record) = load(root, id) {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
        if let Some(blob) = record.artifact.path.as_deref() {
            if is_managed_blob(root, blob) && blob_unused(root, blob) {
                let _ = fs::remove_file(blob);
            }
        }
    } else if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    if tombstone {
        mark_removed(root, id)?;
    }
    Ok(())
}

/// Hide a Works gallery entry. The original file and chat stay put; a tombstone
/// stops the next import from bringing the same item back.
fn remove_from_library(root: &Path, id: &str) -> Result<(), String> {
    drop_record(root, id, true)
}

fn each_record(root: &Path, mut visit: impl FnMut(ArtifactRecord)) {
    let Ok(entries) = fs::read_dir(root.join("records")) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(record) = fs::read(entry.path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ArtifactRecord>(&bytes).ok())
        else {
            continue;
        };
        visit(record);
    }
}

fn forget_conversation_in(root: &Path, conversation_id: &str) {
    let mut ids = Vec::new();
    each_record(root, |record| {
        if record.conversation_id == conversation_id {
            ids.push(record.id);
        }
    });
    for id in ids {
        let _ = drop_record(root, &id, false);
    }
    let cache = root.join("imported-revisions.json");
    if let Ok(bytes) = fs::read(&cache) {
        if let Ok(mut imported) = serde_json::from_slice::<HashMap<String, u64>>(&bytes) {
            if imported.remove(conversation_id).is_some() {
                if let Ok(json) = serde_json::to_string(&imported) {
                    let _ = super::storage::atomic_write(&cache, &json, "artifact import cache");
                }
            }
        }
    }
}

/// Drop gallery records when the source conversation is deleted.
pub(crate) fn forget_conversation(app: &AppHandle, conversation_id: &str) {
    let Ok(root) = root(app) else {
        return;
    };
    forget_conversation_in(&root, conversation_id);
}

fn prune_missing_records(root: &Path, conversation_id: &str, live_ids: &HashSet<String>) {
    let mut ids = Vec::new();
    each_record(root, |record| {
        if record.conversation_id == conversation_id && !live_ids.contains(&record.id) {
            ids.push(record.id);
        }
    });
    for id in ids {
        let _ = drop_record(root, &id, false);
    }
}

/// The only path interpretation for stored tool artifacts, including old records.
pub fn file_path(
    app: &AppHandle,
    conversation: &str,
    artifact: &ChatToolArtifact,
) -> Result<PathBuf, String> {
    let value = artifact
        .path
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or("Artifact has no local file")?;
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if value.contains('/') || value.contains('\\') || value == "." || value == ".." {
        return Err("Invalid relative artifact path".into());
    }
    Ok(super::storage::conversation_attachments_dir(app, conversation)?.join(path))
}

pub fn find_in_messages<'a>(
    messages: impl IntoIterator<Item = &'a ChatMessage>,
    id: &str,
) -> Option<&'a ChatToolArtifact> {
    messages.into_iter().find_map(|m| {
        m.artifacts
            .iter()
            .chain(m.tool_calls.iter().flat_map(|t| &t.artifacts))
            .find(|a| a.id.as_deref() == Some(id))
    })
}

fn bytes_for(artifact: &ChatToolArtifact, path: Option<&Path>) -> Result<Vec<u8>, String> {
    if let Some(path) = path {
        let meta = fs::metadata(path).map_err(|_| "Artifact file is missing".to_string())?;
        if !meta.is_file() || meta.len() > 512 * 1024 * 1024 {
            return Err("Artifact exceeds the 512 MB limit".into());
        }
        return fs::read(path).map_err(|e| e.to_string());
    }
    let (header, payload) = artifact
        .data_url
        .split_once(',')
        .ok_or("Artifact has no readable content")?;
    if !header.starts_with("data:") || !header.ends_with(";base64") {
        return Err("Unsupported artifact content".into());
    }
    if payload.len() > 700 * 1024 * 1024 {
        return Err("Artifact exceeds the 512 MB limit".into());
    }
    STANDARD
        .decode(payload)
        .map_err(|_| "Invalid artifact content".into())
}

fn register(
    root: &Path,
    mut record: ArtifactRecord,
    source_path: Option<&Path>,
    persist_dir: Option<&Path>,
) -> Result<ArtifactRecord, String> {
    if let Ok(existing) = load(root, &record.id) {
        if existing.conversation_id != record.conversation_id {
            return Err("Artifact belongs to another conversation".into());
        }
        // Registration is idempotent. Never turn a thumbnail into a new original.
        return Ok(existing);
    }
    if let Some(source) = source_path {
        let meta = fs::metadata(source).map_err(|_| "Artifact file is missing".to_string())?;
        if !meta.is_file() || meta.len() > 512 * 1024 * 1024 {
            return Err("Artifact exceeds the 512 MB limit".into());
        }
        record.artifact.path = Some(source.to_string_lossy().into_owned());
        record.artifact.size_bytes = Some(meta.len());
        if record.artifact.mime_type.starts_with("image/") {
            if let Ok(bytes) = fs::read(source) {
                record.artifact.data_url =
                    super::attachments::make_thumbnail_data_url(&bytes).unwrap_or_default();
            }
        } else {
            record.artifact.data_url.clear();
        }
        save(root, &record)?;
        return Ok(record);
    }
    let bytes = bytes_for(&record.artifact, None)?;
    let dir = persist_dir.ok_or("Artifact has no local file")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let target = unique_persist_path(dir, &persist_filename(&record.artifact.name));
    fs::write(&target, &bytes).map_err(|e| format!("Artifact could not be saved: {e}"))?;
    record.artifact.path = Some(target.to_string_lossy().into_owned());
    record.artifact.size_bytes = Some(bytes.len() as u64);
    record.artifact.data_url = if record.artifact.mime_type.starts_with("image/") {
        super::attachments::make_thumbnail_data_url(&bytes).unwrap_or_default()
    } else {
        String::new()
    };
    save(root, &record)?;
    Ok(record)
}

fn edit_parent(
    root: &Path,
    conversation: &str,
    arguments: &Value,
) -> Result<Option<ArtifactRecord>, String> {
    let ids = input_artifact_ids(arguments)?;
    let [id] = ids.as_slice() else {
        return Ok(None);
    };
    Ok(load(root, id)
        .ok()
        .filter(|record| record.conversation_id == conversation))
}

pub fn prepare_output<'a>(
    app: &AppHandle,
    conversation_id: &str,
    message_id: &str,
    tool: &ChatToolDefinition,
    arguments: &Value,
    mut output: McpToolCallResult,
) -> super::agent::ToolExecutorFuture<'a> {
    let app = app.clone();
    let conversation_id = conversation_id.to_string();
    let message_id = message_id.to_string();
    let source_tool = tool.name.clone();
    let trusted_native = tool.source == "native";
    let generated = tool.source == "mixer" && tool.name == "mixer_generate_image";
    let presentation = trusted_native && tool.name == "present_artifacts";
    let prepared = presentation && arguments["mode"].as_str() != Some("preview");
    let read_ids = if trusted_native && tool.name == "read" {
        input_artifact_ids(arguments)
    } else {
        Ok(Vec::new())
    };
    let edit_arguments = generated.then(|| arguments.clone());
    let present_ids: Vec<String> = if presentation {
        // Native presentation already filtered malformed IDs. Re-reading the
        // raw arguments here would reintroduce them and discard valid files.
        output
            .structured_content
            .as_ref()
            .and_then(|value| value.get("artifactIds"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    } else {
        Vec::new()
    };
    Box::pin(async move {
        if output.is_error || (output.artifacts.is_empty() && present_ids.is_empty()) {
            return Ok(output);
        }
        let read_ids = read_ids?;
        tauri::async_runtime::spawn_blocking(move || {
            let root = root(&app)?;
            for id in &present_ids {
                resolve(&app, &conversation_id, id)?;
            }
            // Validate every selected ID before marking any file delivered.
            // Preview shares validation but does not publish to Works.
            if prepared {
                for id in &present_ids {
                    let mut record = load(&root, id)?;
                    record.delivered = true;
                    save(&root, &record)?;
                }
            }
            if !read_ids.is_empty() && read_ids.len() == output.artifacts.len() {
                for (artifact, id) in output.artifacts.iter_mut().zip(&read_ids) {
                    *artifact = resolve(&app, &conversation_id, id)?;
                }
                return Ok(output);
            }
            let parent_record = match edit_arguments.as_ref() {
                Some(arguments) if output.artifacts.len() == 1 => {
                    edit_parent(&root, &conversation_id, arguments)?
                }
                _ => None,
            };
            for artifact in &mut output.artifacts {
                let id = format!("art_{}", uuid::Uuid::new_v4().simple());
                artifact.id = Some(id.clone());
                let path = artifact
                    .path
                    .as_ref()
                    .map(|_| file_path(&app, &conversation_id, artifact))
                    .transpose()?;
                let persist = if path.is_none() {
                    super::storage::conversation_attachments_dir(&app, &conversation_id).ok()
                } else {
                    None
                };
                let record = register(
                    &root,
                    ArtifactRecord {
                        work_id: parent_record
                            .as_ref()
                            .map(|r| r.work_id.clone())
                            .unwrap_or_else(|| id.clone()),
                        parent_id: parent_record.as_ref().map(|r| r.id.clone()),
                        id,
                        conversation_id: conversation_id.clone(),
                        message_id: message_id.clone(),
                        title: artifact.name.clone(),
                        created_at: chrono::Utc::now().timestamp(),
                        source_tool: source_tool.clone(),
                        delivered: generated || prepared,
                        artifact: artifact.clone(),
                    },
                    path.as_deref(),
                    persist.as_deref(),
                )?;
                *artifact = record.artifact;
            }
            Ok(output)
        })
        .await
        .map_err(|e| e.to_string())?
    })
}

/// Resolve IDs within the current conversation; never broaden filesystem access.
pub fn resolve(app: &AppHandle, conversation: &str, id: &str) -> Result<ChatToolArtifact, String> {
    let root = root(app)?;
    let existing = load(&root, id).ok();
    if let Some(record) = &existing {
        if record.conversation_id == conversation {
            return Ok(record.artifact.clone());
        }
    }
    record_path(&root, id)?;
    let saved = super::storage::load_conversation(app, conversation)?;
    let drafts = super::draft_journal::latest_drafts_for(app, conversation);
    let artifact = find_in_messages(drafts.iter().chain(saved.messages.iter()), id)
        .cloned()
        .ok_or_else(|| format!("Unknown artifact: {id}"))?;
    // A fork may carry the same immutable artifact in its copied messages.
    // Possession of an arbitrary ID alone does not authorize another chat.
    if let Some(record) = existing {
        return Ok(record.artifact);
    }
    let path = artifact
        .path
        .as_ref()
        .map(|_| file_path(app, conversation, &artifact))
        .transpose()?;
    let persist = if path.is_none() {
        super::storage::conversation_attachments_dir(app, conversation).ok()
    } else {
        None
    };
    register(
        &root,
        ArtifactRecord {
            id: id.into(),
            work_id: id.into(),
            parent_id: None,
            conversation_id: conversation.into(),
            message_id: String::new(),
            title: saved.title,
            created_at: chrono::Utc::now().timestamp(),
            source_tool: "legacy".into(),
            delivered: false,
            artifact,
        },
        path.as_deref(),
        persist.as_deref(),
    )
    .map(|r| r.artifact)
}

/// Index delivered conversation files for the Works gallery. Does not rewrite
/// the conversation or copy original files. Repeated imports are cheap.
fn import_conversation(
    app: &AppHandle,
    root: &Path,
    conversation: &Conversation,
    removed: &HashSet<String>,
) -> usize {
    let mut warnings = 0;
    let mut live_ids = conversation_artifact_ids(conversation);
    for message in &conversation.messages {
        if message.role != "assistant" {
            continue;
        }
        let referenced = referenced_ids(&message.content);
        for (tool, arguments, artifact) in
            message.artifacts.iter().map(|a| ("direct", None, a)).chain(
                message.tool_calls.iter().flat_map(|t| {
                    t.artifacts
                        .iter()
                        .map(move |a| (t.name.as_str(), Some(t.arguments.as_str()), a))
                }),
            )
        {
            let id = artifact.id.clone().unwrap_or_else(|| {
                format!(
                    "art_legacy_{:x}",
                    Sha256::digest(
                        format!(
                            "{}:{}:{}:{}",
                            conversation.id,
                            message.id,
                            artifact.name,
                            artifact.path.as_deref().unwrap_or("")
                        )
                        .as_bytes()
                    )
                )
            });
            let args: Value = arguments
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null);
            let delivered = tool == "direct"
                || tool == "mixer_generate_image"
                || (tool == "present_artifacts" && args["mode"].as_str() != Some("preview"))
                || referenced.contains(&id);
            live_ids.insert(id.clone());
            if !delivered || removed.contains(&id) {
                continue;
            }
            if let Ok(mut existing) = load(root, &id) {
                if !existing.delivered {
                    existing.delivered = true;
                    if save(root, &existing).is_err() {
                        warnings += 1;
                    }
                }
                continue;
            }
            let path = artifact
                .path
                .as_ref()
                .map(|_| file_path(app, &conversation.id, artifact));
            let path = match path.transpose() {
                Ok(p) => p,
                Err(_) => {
                    warnings += 1;
                    continue;
                }
            };
            let mut artifact = artifact.clone();
            artifact.id = Some(id.clone());
            let record = ArtifactRecord {
                work_id: id.clone(),
                parent_id: None,
                id,
                conversation_id: conversation.id.clone(),
                message_id: message.id.clone(),
                title: conversation.title.clone(),
                created_at: message.timestamp,
                source_tool: tool.into(),
                delivered,
                artifact,
            };
            let persist = if path.is_none() {
                super::storage::conversation_attachments_dir(app, &conversation.id).ok()
            } else {
                None
            };
            if register(root, record, path.as_deref(), persist.as_deref()).is_err() {
                warnings += 1;
            }
        }
    }
    prune_missing_records(root, &conversation.id, &live_ids);
    warnings
}

/// Window metadata only: resolve dependencies without changing message ownership,
/// delivery state, or the stored conversation. No original image bytes are read.
pub(crate) fn history_reference_artifacts(
    messages: &[ChatMessage],
    range: std::ops::Range<usize>,
) -> Vec<ChatToolArtifact> {
    let mut needed = HashSet::new();
    let mut present = HashSet::new();
    for message in &messages[range] {
        let mut text = message.content.clone();
        for segment in &message.segments {
            if let Some(segment_text) = &segment.text {
                text.push_str("\n\n");
                text.push_str(segment_text);
            }
        }
        needed.extend(referenced_ids(&text));
        for tool in &message.tool_calls {
            if tool.source == "native" && tool.name == "present_artifacts" {
                if let Some(value) = tool
                    .structured_content
                    .as_ref()
                    .filter(|value| value["type"] == "artifact_presentation")
                {
                    for id in value
                        .get("artifactIds")
                        .or_else(|| value.get("artifact_ids"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        needed.insert(id.trim().to_string());
                    }
                }
            }
        }
        present.extend(
            message
                .artifacts
                .iter()
                .chain(message.tool_calls.iter().flat_map(|tool| &tool.artifacts))
                .filter_map(|artifact| artifact.id.clone()),
        );
    }
    needed.retain(|id| !present.contains(id));
    if needed.is_empty() {
        return Vec::new();
    }
    let mut selected = std::collections::BTreeMap::new();
    for artifact in messages.iter().flat_map(|message| {
        message
            .artifacts
            .iter()
            .chain(message.tool_calls.iter().flat_map(|tool| &tool.artifacts))
    }) {
        if let Some(id) = artifact.id.as_ref().filter(|id| needed.contains(*id)) {
            selected.insert(id, artifact);
        }
    }
    selected.into_values().cloned().collect()
}

fn referenced_ids(text: &str) -> HashSet<String> {
    if !text.contains("artifact:") {
        return HashSet::new();
    }
    static PATTERNS: std::sync::OnceLock<(regex::Regex, regex::Regex)> = std::sync::OnceLock::new();
    let (fence, reference) = PATTERNS.get_or_init(|| {
        (
            regex::Regex::new(r"(?s)```.*?```|`[^`]*`").unwrap(),
            regex::Regex::new(r"artifact:(?://)?(art_[A-Za-z0-9_-]+)").unwrap(),
        )
    });
    let text = fence.replace_all(text, "");
    reference
        .captures_iter(&text)
        .map(|c| c[1].into())
        .collect()
}

/// Older turns assigned another ID when reading back a generated file. Keep
/// both IDs resolvable, but show its delivered original only once in Works.
fn omit_repeated_image_reads(items: &mut Vec<LibraryItem>) {
    let generated: HashSet<_> = items
        .iter()
        .filter(|item| {
            matches!(
                item.record.source_tool.as_str(),
                "mixer_generate_image" | "direct"
            )
        })
        .filter_map(|item| {
            item.record.artifact.path.as_ref().map(|path| {
                (
                    item.record.conversation_id.clone(),
                    item.record.message_id.clone(),
                    path.clone(),
                )
            })
        })
        .collect();
    items.retain(|item| {
        item.record.source_tool != "read"
            || !item.record.artifact.path.as_ref().is_some_and(|path| {
                generated.contains(&(
                    item.record.conversation_id.clone(),
                    item.record.message_id.clone(),
                    path.clone(),
                ))
            })
    });
}

/// Media jobs remain the source of truth; import only completed outputs into the
/// same index used by chat works, keeping original files and gallery edits intact.
fn import_media_jobs(root: &Path, media_root: &Path) -> usize {
    use super::media_station::{MediaJob, MediaStatus};
    let entries = match fs::read_dir(media_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(_) => return 1,
    };
    let removed = load_removed(root);
    let mut warnings = 0;
    for entry in entries {
        let directory = match entry {
            Ok(entry) if entry.path().is_dir() => entry.path(),
            Ok(_) => continue,
            Err(_) => {
                warnings += 1;
                continue;
            }
        };
        let job_path = directory.join("job.json");
        if !job_path.exists() {
            continue;
        }
        let job = fs::read(&job_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<MediaJob>(&bytes).ok());
        let Some(job) = job else {
            warnings += 1;
            continue;
        };
        if uuid::Uuid::parse_str(&job.id).is_err()
            || directory.file_name().and_then(|name| name.to_str()) != Some(job.id.as_str())
        {
            warnings += 1;
            continue;
        }
        if job.status != MediaStatus::Completed {
            continue;
        }
        for (index, output) in job.outputs.iter().enumerate() {
            let id = format!("art_media_{}_{index}", job.id);
            if removed.contains(&id) || record_path(root, &id).is_ok_and(|path| path.exists()) {
                continue;
            }
            if sanitized_name(&output.name, &output.name).as_deref() != Ok(output.name.as_str()) {
                warnings += 1;
                continue;
            }
            let path = directory.join(&output.name);
            let record = ArtifactRecord {
                work_id: id.clone(),
                parent_id: None,
                conversation_id: String::new(),
                message_id: job.id.clone(),
                title: job.request.prompt.clone(),
                created_at: job.created_at / 1000,
                source_tool: "media_station".into(),
                delivered: true,
                artifact: ChatToolArtifact {
                    id: Some(id.clone()),
                    name: output.name.clone(),
                    mime_type: output.mime_type.clone(),
                    data_url: String::new(),
                    path: None,
                    size_bytes: None,
                },
                id,
            };
            if register(root, record, Some(&path), None).is_err() {
                warnings += 1;
            }
        }
    }
    warnings
}

#[tauri::command]
pub async fn chat_artifacts_list(
    app: AppHandle,
    import_history: Option<bool>,
) -> Result<LibraryPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root(&app)?;
        let _media_guard = MEDIA_INDEX.lock();
        let index = super::storage::load_index(&app)?;
        let mut warnings = 0;
        if import_history.unwrap_or(true) {
            warnings += import_media_jobs(&root, &root.with_file_name("media-station"));
            let removed = load_removed(&root);
            // The conversation revision owns invalidation. Failed imports remain
            // retryable; unchanged conversations need no repeated JSON/image reads.
            let import_cache = root.join("imported-revisions.json");
            let mut imported: HashMap<String, u64> = fs::read(&import_cache)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            let mut cache_changed = false;
            for item in &index.conversations {
                if item
                    .revision
                    .is_some_and(|revision| imported.get(&item.id) == Some(&revision))
                {
                    continue;
                }
                match super::storage::load_conversation(&app, &item.id) {
                    Ok(conversation) => {
                        let count = import_conversation(&app, &root, &conversation, &removed);
                        warnings += count;
                        if count == 0 {
                            if let Some(revision) = item.revision {
                                imported.insert(item.id.clone(), revision);
                                cache_changed = true;
                            }
                        }
                    }
                    Err(_) => warnings += 1,
                }
            }
            if cache_changed {
                if super::storage::atomic_write(
                    &import_cache,
                    &serde_json::to_string(&imported).map_err(|e| e.to_string())?,
                    "artifact import cache",
                )
                .is_err()
                {
                    warnings += 1;
                }
            }
        }
        let sources: HashMap<_, _> = index
            .conversations
            .iter()
            .map(|conversation| (conversation.id.as_str(), conversation))
            .collect();
        let mut items = Vec::new();
        let records = root.join("records");
        if records.exists() {
            for entry in fs::read_dir(records).map_err(|e| e.to_string())?.flatten() {
                if entry.path().extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let Ok(bytes) = fs::read(entry.path()) else {
                    warnings += 1;
                    continue;
                };
                let Ok(mut record) = serde_json::from_slice::<ArtifactRecord>(&bytes) else {
                    warnings += 1;
                    continue;
                };
                let source = sources.get(record.conversation_id.as_str()).copied();
                if !visible_in_works(&record, source.is_some()) {
                    continue;
                }
                if let Some(source) = source {
                    record.title = source.title.clone();
                }
                let available = record
                    .artifact
                    .path
                    .as_ref()
                    .is_some_and(|p| Path::new(p).is_file());
                items.push(LibraryItem {
                    record,
                    available,
                    source_available: source.is_some(),
                });
            }
        }
        omit_repeated_image_reads(&mut items);
        items.sort_by(|a, b| {
            b.record
                .created_at
                .cmp(&a.record.created_at)
                .then_with(|| a.record.id.cmp(&b.record.id))
        });
        Ok(LibraryPage { items, warnings })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn chat_artifact_action(
    app: AppHandle,
    id: String,
    action: String,
    destination: Option<String>,
    name: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root(&app)?;
        let _media_guard = MEDIA_INDEX.lock();
        match action.as_str() {
            "delete" => {
                remove_from_library(&root, &id)?;
                return Ok(None);
            }
            "rename" => {
                let proposed = name.or(destination).ok_or("Choose a name")?;
                let mut record = load(&root, &id)?;
                record.artifact.name = sanitized_name(&record.artifact.name, &proposed)?;
                save(&root, &record)?;
                return Ok(None);
            }
            _ => {}
        }
        let record = load(&root, &id)?;
        let path = record
            .artifact
            .path
            .as_deref()
            .ok_or("Artifact is unavailable")?;
        if !Path::new(path).is_file() {
            return Err("Artifact file is missing".into());
        }
        match action.as_str() {
            "reveal" => {
                crate::dock::fs::reveal_file_in_manager(Path::new(path))?;
                Ok(None)
            }
            "open" => {
                use tauri_plugin_shell::ShellExt;
                #[allow(deprecated)]
                app.shell()
                    .open(path.to_string(), None)
                    .map_err(|e| e.to_string())?;
                Ok(None)
            }
            "export" | "export_unique" => {
                let destination = destination.ok_or("Choose a destination")?;
                let target = if action == "export_unique" {
                    Path::new(&destination).join(persist_filename(&record.artifact.name))
                } else {
                    PathBuf::from(destination)
                };
                export_artifact(&root, Path::new(path), &target, action == "export_unique")?;
                Ok(None)
            }
            _ => Err("Unsupported artifact action".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_artifact_ids_ignore_empty_placeholders_and_deduplicate() {
        assert!(
            input_artifact_ids(&serde_json::json!({"artifact_ids": ["", " "]}))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            input_artifact_ids(&serde_json::json!({
                "artifact_ids": [" art_first ", "", "art_second", "art_first"]
            }))
            .unwrap(),
            vec!["art_first", "art_second"]
        );
        assert!(input_artifact_ids(&serde_json::json!({}))
            .unwrap()
            .is_empty());
        assert!(input_artifact_ids(&serde_json::json!({"artifact_ids": "art_first"})).is_err());
    }

    fn record(id: &str, content: &[u8]) -> ArtifactRecord {
        ArtifactRecord {
            id: id.into(),
            work_id: id.into(),
            parent_id: None,
            conversation_id: "conv_test".into(),
            message_id: "msg".into(),
            title: "Work".into(),
            created_at: 1,
            source_tool: "mixer_generate_image".into(),
            delivered: true,
            artifact: ChatToolArtifact {
                id: Some(id.into()),
                name: "drawing.png".into(),
                mime_type: "image/png".into(),
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(content)),
                path: None,
                size_bytes: None,
            },
        }
    }

    fn media_job(
        media_root: &Path,
        kind: super::super::media_station::MediaKind,
        name: &str,
        bytes: &[u8],
    ) -> super::super::media_station::MediaJob {
        use super::super::media_station::{
            MediaJob, MediaKind, MediaOutput, MediaRequest, MediaStatus,
        };
        let mime = if kind == MediaKind::Image {
            "image/png"
        } else {
            "video/mp4"
        };
        let job = MediaJob {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: 1_700_000_000_123,
            request: MediaRequest {
                kind,
                provider_id: "provider".into(),
                model: "model".into(),
                prompt: "Ocean at sunset".into(),
                aspect_ratio: "16:9".into(),
                duration: 5,
                reference_paths: Vec::new(),
            },
            status: MediaStatus::Completed,
            error: None,
            outputs: vec![MediaOutput {
                name: name.into(),
                mime_type: mime.into(),
                preview: String::new(),
            }],
            provider_task_id: None,
        };
        let directory = media_root.join(&job.id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(name), bytes).unwrap();
        fs::write(
            directory.join("job.json"),
            serde_json::to_vec(&job).unwrap(),
        )
        .unwrap();
        job
    }

    #[test]
    fn media_works_import_existing_images_and_completed_videos_without_a_chat() {
        use super::super::media_station::{MediaKind, MediaStatus};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let media = dir.path().join("media-station");
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8, 8)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let image = media_job(
            &media,
            MediaKind::Image,
            "generated-image-1.png",
            png.get_ref(),
        );
        let mut video = media_job(&media, MediaKind::Video, "video.mp4", b"video bytes");
        video.status = MediaStatus::Running;
        fs::write(
            media.join(&video.id).join("job.json"),
            serde_json::to_vec(&video).unwrap(),
        )
        .unwrap();
        assert_eq!(import_media_jobs(&root, &media), 0);
        let image_id = format!("art_media_{}_0", image.id);
        let video_id = format!("art_media_{}_0", video.id);
        let saved_image = load(&root, &image_id).unwrap();
        assert!(load(&root, &video_id).is_err());
        assert!(visible_in_works(&saved_image, false));
        assert_eq!(saved_image.created_at, 1_700_000_000);
        assert_eq!(saved_image.title, image.request.prompt);
        assert!(saved_image.artifact.data_url.starts_with("data:image/"));
        assert_eq!(
            Path::new(saved_image.artifact.path.as_ref().unwrap()),
            media.join(&image.id).join("generated-image-1.png")
        );
        let chat = record("art_chat", b"chat");
        assert!(!visible_in_works(&chat, false));
        assert!(visible_in_works(&chat, true));

        video.status = MediaStatus::Completed;
        fs::write(
            media.join(&video.id).join("job.json"),
            serde_json::to_vec(&video).unwrap(),
        )
        .unwrap();
        assert_eq!(import_media_jobs(&root, &media), 0);
        let saved_video = load(&root, &video_id).unwrap();
        assert!(visible_in_works(&saved_video, false));
        assert_eq!(saved_video.artifact.mime_type, "video/mp4");
        let exports = dir.path().join("exports");
        fs::create_dir_all(&exports).unwrap();
        for saved in [&saved_image, &saved_video] {
            let source = Path::new(saved.artifact.path.as_ref().unwrap());
            let target = exports.join(&saved.artifact.name);
            export_artifact(&root, source, &target, false).unwrap();
            assert_eq!(fs::read(target).unwrap(), fs::read(source).unwrap());
        }
        let mut renamed = saved_image;
        renamed.artifact.name = "cover.png".into();
        save(&root, &renamed).unwrap();
        remove_from_library(&root, &video_id).unwrap();
        assert_eq!(import_media_jobs(&root, &media), 0);
        assert_eq!(load(&root, &image_id).unwrap().artifact.name, "cover.png");
        assert!(load(&root, &video_id).is_err());
        assert_eq!(
            fs::read(media.join(&video.id).join("video.mp4")).unwrap(),
            b"video bytes"
        );
        assert!(!root.join("files").exists());
    }

    #[test]
    fn media_works_report_bad_history_and_retry_missing_files_without_blocking_other_results() {
        use super::super::media_station::MediaKind;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let media = dir.path().join("media-station");
        let good = media_job(&media, MediaKind::Video, "video.mp4", b"good video");
        let missing = media_job(&media, MediaKind::Video, "video.mp4", b"recoverable video");
        fs::remove_file(media.join(&missing.id).join("video.mp4")).unwrap();
        let mut unsafe_job = media_job(&media, MediaKind::Video, "video.mp4", b"unsafe");
        unsafe_job.outputs[0].name = "../outside.mp4".into();
        fs::write(
            media.join(&unsafe_job.id).join("job.json"),
            serde_json::to_vec(&unsafe_job).unwrap(),
        )
        .unwrap();
        let broken = media.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("job.json"), b"not json").unwrap();
        assert_eq!(import_media_jobs(&root, &media), 3);
        assert!(load(&root, &format!("art_media_{}_0", good.id)).is_ok());
        assert!(load(&root, &format!("art_media_{}_0", unsafe_job.id)).is_err());
        fs::write(
            media.join(&missing.id).join("video.mp4"),
            b"recovered video",
        )
        .unwrap();
        assert_eq!(import_media_jobs(&root, &media), 2);
        let restored = load(&root, &format!("art_media_{}_0", missing.id)).unwrap();
        assert_eq!(
            fs::read(restored.artifact.path.unwrap()).unwrap(),
            b"recovered video"
        );
    }

    #[test]
    fn tool_contract_edit_parent_uses_normalized_single_reference() {
        let root = tempfile::tempdir().unwrap();
        let original = record("art_original", b"image");
        save(root.path(), &original).unwrap();
        for ids in [
            serde_json::json!([" art_original "]),
            serde_json::json!(["art_original", "", "art_original"]),
        ] {
            let parent = edit_parent(
                root.path(),
                "conv_test",
                &serde_json::json!({"artifact_ids": ids}),
            )
            .unwrap()
            .expect("one normalized reference must retain its work");
            assert_eq!(parent.id, original.id);
            assert_eq!(parent.work_id, original.work_id);
        }
        assert!(edit_parent(
            root.path(),
            "other_conversation",
            &serde_json::json!({"artifact_ids": ["art_original"]})
        )
        .unwrap()
        .is_none());
        assert!(edit_parent(
            root.path(),
            "conv_test",
            &serde_json::json!({"artifact_ids": ["art_original", "art_other"]})
        )
        .unwrap()
        .is_none());
    }
    #[test]
    fn register_keeps_existing_source_path_and_does_not_copy() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let file = project.join("report.xlsx");
        fs::write(&file, b"workbook").unwrap();
        let mut original = record("art_one", b"workbook");
        original.artifact.name = "report.xlsx".into();
        original.artifact.mime_type =
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into();
        original.artifact.data_url = format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(b"workbook")
        );
        for _ in 0..3 {
            let saved = register(dir.path(), original.clone(), Some(&file), None).unwrap();
            assert_eq!(PathBuf::from(saved.artifact.path.as_ref().unwrap()), file);
            assert!(saved.artifact.data_url.is_empty());
            assert!(load(dir.path(), "art_one")
                .unwrap()
                .artifact
                .data_url
                .is_empty());
        }
        assert!(!dir.path().join("files").exists());
        assert_eq!(fs::read_dir(dir.path().join("records")).unwrap().count(), 1);
        assert_eq!(fs::read(&file).unwrap(), b"workbook");
    }

    #[test]
    fn register_without_source_writes_to_conversation_dir() {
        let dir = tempfile::tempdir().unwrap();
        let attachments = dir.path().join("attachments");
        let saved = register(
            dir.path(),
            record("art_img", b"png-bytes"),
            None,
            Some(&attachments),
        )
        .unwrap();
        let path = PathBuf::from(saved.artifact.path.unwrap());
        assert!(path.starts_with(&attachments));
        assert!(!path.starts_with(dir.path().join("files")));
        assert_eq!(fs::read(&path).unwrap(), b"png-bytes");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("drawing.png")
        );
    }

    #[test]
    fn missing_source_is_not_copied_into_a_works_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(record_path(dir.path(), "art_../../secret").is_err());
        assert!(register(
            dir.path(),
            record("art_no", b"x"),
            Some(&dir.path().join("missing")),
            None
        )
        .is_err());
        assert!(!dir.path().join("records/art_no.json").exists());
        assert!(!dir.path().join("files").exists());
    }
    #[test]
    fn examples_do_not_promote_internal_artifacts() {
        assert_eq!(
            referenced_ids("`[example](artifact:art_hidden)`\n![result](artifact:art_real)"),
            HashSet::from(["art_real".into()])
        );
    }

    #[test]
    fn legacy_self_checks_do_not_duplicate_a_delivered_work() {
        let mut original = record("art_original", b"x");
        original.artifact.path = Some("same.png".into());
        let mut read = original.clone();
        read.id = "art_read".into();
        read.source_tool = "read".into();
        let mut unrelated = read.clone();
        unrelated.id = "art_unrelated".into();
        unrelated.artifact.path = Some("another.png".into());
        let mut items: Vec<_> = [read, original, unrelated]
            .into_iter()
            .map(|record| LibraryItem {
                record,
                available: true,
                source_available: true,
            })
            .collect();
        omit_repeated_image_reads(&mut items);
        assert_eq!(
            items
                .iter()
                .map(|i| i.record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["art_original", "art_unrelated"]
        );
    }

    #[test]
    fn rename_keeps_extension_and_rejects_path_characters() {
        assert_eq!(sanitized_name("drawing.png", "封面").unwrap(), "封面.png");
        assert_eq!(
            sanitized_name("drawing.png", "cover.jpg").unwrap(),
            "cover.jpg"
        );
        assert!(sanitized_name("drawing.png", "../secret").is_err());
        assert!(sanitized_name("drawing.png", "").is_err());
    }

    #[test]
    fn bulk_exports_keep_existing_and_same_named_files() {
        let managed = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let first = managed.path().join("first.txt");
        let second = managed.path().join("second.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();
        let target = destination.path().join("report.txt");
        fs::write(&target, "existing").unwrap();
        export_artifact(managed.path(), &first, &target, true).unwrap();
        export_artifact(managed.path(), &second, &target, true).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "existing");
        assert_eq!(
            fs::read_to_string(destination.path().join("report-2.txt")).unwrap(),
            "first"
        );
        assert_eq!(
            fs::read_to_string(destination.path().join("report-3.txt")).unwrap(),
            "second"
        );
        assert_eq!(persist_filename("../escape.txt"), "artifact.bin");
    }

    #[test]
    fn delete_hides_the_record_without_touching_the_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("keep.xlsx");
        fs::write(&file, b"keep").unwrap();
        register(dir.path(), record("art_first", b"x"), Some(&file), None).unwrap();
        remove_from_library(dir.path(), "art_first").unwrap();
        assert!(load(dir.path(), "art_first").is_err());
        assert!(load_removed(dir.path()).contains("art_first"));
        assert_eq!(fs::read(&file).unwrap(), b"keep");
    }

    #[test]
    fn forget_conversation_drops_its_records_and_leaves_project_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("project-report.xlsx");
        fs::write(&file, b"report").unwrap();
        for index in 0..24 {
            register(
                dir.path(),
                record(&format!("art_keep_{index}"), b"x"),
                Some(&file),
                None,
            )
            .unwrap();
        }
        let mut other = record("art_other", b"y");
        other.conversation_id = "conv_other".into();
        register(dir.path(), other, Some(&file), None).unwrap();
        forget_conversation_in(dir.path(), "conv_test");
        for index in 0..24 {
            assert!(load(dir.path(), &format!("art_keep_{index}")).is_err());
        }
        assert_eq!(load(dir.path(), "art_other").unwrap().id, "art_other");
        assert_eq!(fs::read(&file).unwrap(), b"report");
    }

    #[test]
    fn prune_drops_records_removed_from_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gone.xlsx");
        fs::write(&file, b"x").unwrap();
        register(dir.path(), record("art_stale", b"x"), Some(&file), None).unwrap();
        register(dir.path(), record("art_live", b"x"), Some(&file), None).unwrap();
        prune_missing_records(dir.path(), "conv_test", &HashSet::from(["art_live".into()]));
        assert!(load(dir.path(), "art_stale").is_err());
        assert_eq!(load(dir.path(), "art_live").unwrap().id, "art_live");
        assert!(file.is_file());
    }

    #[test]
    fn managed_blob_paths_are_only_the_legacy_dump_folders() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_managed_blob(
            dir.path(),
            dir.path().join("files/abc.png").to_str().unwrap()
        ));
        assert!(is_managed_blob(
            dir.path(),
            dir.path().join("fileless/abc.png").to_str().unwrap()
        ));
        assert!(!is_managed_blob(
            dir.path(),
            dir.path().join("project/report.xlsx").to_str().unwrap()
        ));
    }

    fn multi_output_media_job(
        media_root: &Path,
        kind: super::super::media_station::MediaKind,
    ) -> super::super::media_station::MediaJob {
        use super::super::media_station::MediaOutput;
        let mut job = media_job(media_root, kind, "first.png", b"source");
        job.outputs.push(MediaOutput {
            name: "video.mp4".into(),
            mime_type: "video/mp4".into(),
            preview: String::new(),
        });
        save_media_fixture(media_root, &job);
        job
    }

    fn save_media_fixture(media_root: &Path, job: &super::super::media_station::MediaJob) {
        let directory = media_root.join(&job.id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("job.json"), serde_json::to_vec(job).unwrap()).unwrap();
        for output in &job.outputs {
            fs::write(directory.join(&output.name), b"source").unwrap();
        }
    }

    #[test]
    fn deleting_media_removes_all_works_without_exports_or_other_references() {
        use super::super::media_station::MediaKind;
        let root = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        let exported = tempfile::tempdir().unwrap();
        for kind in [MediaKind::Image, MediaKind::Video] {
            let job = multi_output_media_job(media.path(), kind);
            assert_eq!(import_media_jobs(root.path(), media.path()), 0);
            let id = format!("art_media_{}_0", job.id);
            let other = format!("art_media_{}_1", job.id);
            let stale = format!("art_media_{}_99", job.id);
            let mut stale_record = record(&stale, b"old");
            stale_record.artifact.path =
                Some(exported.path().join("export.png").to_string_lossy().into());
            save(root.path(), &stale_record).unwrap();
            fs::write(exported.path().join("export.png"), b"exported").unwrap();
            let mut reference = record("art_independent_reference", b"reference");
            reference.artifact.path = Some(
                media
                    .path()
                    .join(&job.id)
                    .join("first.png")
                    .to_string_lossy()
                    .into(),
            );
            save(root.path(), &reference).unwrap();
            remove_from_library(root.path(), &other).unwrap();
            mark_removed(root.path(), "art_unrelated_hidden").unwrap();
            delete_media_job_in(root.path(), media.path(), &job.id).unwrap();
            assert!(!media.path().join(&job.id).exists());
            for id in [&id, &other, &stale] {
                assert!(load(root.path(), id).is_err());
                assert!(!load_removed(root.path()).contains(id));
            }
            assert_eq!(
                load(root.path(), "art_independent_reference").unwrap().id,
                "art_independent_reference"
            );
            assert!(load_removed(root.path()).contains("art_unrelated_hidden"));
            assert_eq!(
                fs::read(exported.path().join("export.png")).unwrap(),
                b"exported"
            );
            assert_eq!(import_media_jobs(root.path(), media.path()), 0);
            assert!(load(root.path(), &id).is_err());
        }
    }

    #[test]
    fn source_delete_failure_is_not_reported_as_success() {
        let root = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        fs::write(media.path().join(&id), b"not a directory").unwrap();
        assert!(delete_media_job_in(root.path(), media.path(), &id).is_err());
        assert_eq!(
            fs::read(media.path().join(&id)).unwrap(),
            b"not a directory"
        );
        assert!(delete_media_job_in(root.path(), media.path(), "../escape").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn source_symlink_never_deletes_an_external_directory() {
        let root = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("keep"), b"keep").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        std::os::unix::fs::symlink(external.path(), media.path().join(&id)).unwrap();
        assert!(delete_media_job_in(root.path(), media.path(), &id).is_err());
        assert_eq!(fs::read(external.path().join("keep")).unwrap(), b"keep");
    }

    #[test]
    fn media_import_finishes_before_delete_and_cannot_restore_deleted_output() {
        let root = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        let job =
            multi_output_media_job(media.path(), super::super::media_station::MediaKind::Video);
        let guard = MEDIA_INDEX.lock();
        let root_path = root.path().to_path_buf();
        let media_path = media.path().to_path_buf();
        let id = job.id.clone();
        let (started, waiting) = std::sync::mpsc::channel();
        let deleting = std::thread::spawn(move || {
            started.send(()).unwrap();
            delete_media_job_in(&root_path, &media_path, &id)
        });
        waiting.recv().unwrap();
        // Same lock/snapshot boundary as the real gallery list.
        assert_eq!(import_media_jobs(root.path(), media.path()), 0);
        assert!(load(root.path(), &format!("art_media_{}_0", job.id)).is_ok());
        drop(guard);
        deleting.join().unwrap().unwrap();
        let _guard = MEDIA_INDEX.lock();
        assert_eq!(import_media_jobs(root.path(), media.path()), 0);
        assert!(load(root.path(), &format!("art_media_{}_0", job.id)).is_err());
    }

    #[test]
    fn partial_output_cleanup_keeps_metadata_for_restart() {
        let media = tempfile::tempdir().unwrap();
        let job =
            multi_output_media_job(media.path(), super::super::media_station::MediaKind::Video);
        let directory = media.path().join(&job.id);
        let mut calls = 0;
        let outcome = delete_media_directory(&directory, |path| {
            calls += 1;
            if calls == 2 {
                return Err("injected filesystem failure".into());
            }
            fs::remove_file(path).map_err(|e| e.to_string())
        });
        assert!(outcome.unwrap_err().contains("部分文件清理失败"));
        assert_eq!(calls, 2);
        let saved: super::super::media_station::MediaJob =
            serde_json::from_slice(&fs::read(directory.join("job.json")).unwrap()).unwrap();
        assert_eq!(saved.id, job.id);
        assert_eq!(
            job.outputs
                .iter()
                .filter(|output| directory.join(&output.name).exists())
                .count(),
            1
        );
    }
}
