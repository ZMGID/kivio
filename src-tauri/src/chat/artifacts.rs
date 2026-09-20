//! Artifact IDs for model reads/edits, and the Works gallery index.
//! Files stay where they were created. Works is a view of delivered conversation files.
use super::{ChatMessage, Conversation};
use crate::mcp::types::{ChatToolArtifact, ChatToolDefinition, McpToolCallResult};
use base64::{engine::general_purpose::STANDARD, Engine};
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

fn record_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !id.starts_with("art_")
        || id.len() > 160
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
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

fn persist_filename(name: &str) -> String {
    sanitized_name(name, name).unwrap_or_else(|_| "artifact.bin".into())
}

fn visible_in_works(delivered: bool, conversation_exists: bool) -> bool {
    delivered && conversation_exists
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
        } else if record.artifact.data_url.len() > 64 * 1024 {
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
    let prepared = trusted_native
        && tool.name == "present_artifacts"
        && arguments["mode"].as_str() != Some("preview");
    let read_ids: Vec<String> = if trusted_native && tool.name == "read" {
        arguments["artifact_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    } else {
        Vec::new()
    };
    let parent = arguments["artifact_ids"]
        .as_array()
        .filter(|ids| ids.len() == 1)
        .and_then(|ids| ids[0].as_str())
        .map(str::to_string);
    let present_ids: Vec<String> = if prepared {
        arguments["artifact_ids"]
            .as_array()
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
        tauri::async_runtime::spawn_blocking(move || {
            let root = root(&app)?;
            for id in &present_ids {
                resolve(&app, &conversation_id, id)?;
                let mut record = load(&root, id)?;
                record.delivered = true;
                save(&root, &record)?;
            }
            if !read_ids.is_empty() && read_ids.len() == output.artifacts.len() {
                for (artifact, id) in output.artifacts.iter_mut().zip(&read_ids) {
                    *artifact = resolve(&app, &conversation_id, id)?;
                }
                return Ok(output);
            }
            let parent_record = if generated && output.artifacts.len() == 1 {
                parent
                    .as_deref()
                    .and_then(|id| load(&root, id).ok())
                    .filter(|r| r.conversation_id == conversation_id)
            } else {
                None
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

fn referenced_ids(text: &str) -> HashSet<String> {
    let fence = regex::Regex::new(r"(?s)```.*?```|`[^`]*`").unwrap();
    let text = fence.replace_all(text, "");
    regex::Regex::new(r"artifact:(?://)?(art_[A-Za-z0-9_-]+)")
        .unwrap()
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

#[tauri::command]
pub async fn chat_artifacts_list(app: AppHandle) -> Result<LibraryPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root(&app)?;
        let removed = load_removed(&root);
        let index = super::storage::load_index(&app)?;
        let mut warnings = 0;
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
                let source = index
                    .conversations
                    .iter()
                    .find(|c| c.id == record.conversation_id);
                if !visible_in_works(record.delivered, source.is_some()) {
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
                    source_available: true,
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
            "preview" => super::attachments::read_attachment_as_data_url(Path::new(path)).map(Some),
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
            "export" => {
                let destination = destination.ok_or("Choose a destination")?;
                if !Path::new(&destination).is_absolute() {
                    return Err("Destination must be absolute".into());
                }
                let target = Path::new(&destination);
                let parent = target
                    .parent()
                    .ok_or("Invalid destination")?
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                if parent.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
                    return Err("Choose a location outside the managed Works folder".into());
                }
                if target.canonicalize().ok() != Path::new(path).canonicalize().ok() {
                    fs::copy(path, destination).map_err(|e| e.to_string())?;
                }
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
    #[test]
    fn register_keeps_existing_source_path_and_does_not_copy() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let file = project.join("report.xlsx");
        fs::write(&file, b"workbook").unwrap();
        let original = record("art_one", b"ignored");
        for _ in 0..3 {
            let saved = register(dir.path(), original.clone(), Some(&file), None).unwrap();
            assert_eq!(PathBuf::from(saved.artifact.path.as_ref().unwrap()), file);
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
    fn works_gallery_requires_a_living_conversation() {
        assert!(visible_in_works(true, true));
        assert!(!visible_in_works(true, false));
        assert!(!visible_in_works(false, true));
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
}
