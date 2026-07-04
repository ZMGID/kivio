use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

use super::user_home_dir;
use crate::mcp::types::ChatToolArtifact;

/// Persistent per-conversation delivery directory. Files here are finished
/// deliverables for the user (produced by `write_file` writing into this dir, or
/// by `run_python` artifacts), shown as downloadable file cards. **Persistent:
/// never auto-pruned** — only removed when the conversation itself is deleted.
const OUTPUTS_ROOT: &str = "Kivio/outputs";
/// Legacy ephemeral exports tree from prior versions (`run_python` used to write
/// here under `<conversation>/<message>/`). Still GC'd at startup so old runs go
/// away; nothing writes here anymore.
const LEGACY_RUNS_ROOT: &str = "Kivio/runs";
const LEGACY_RUNS_RETENTION_DAYS: u64 = 7;
const MAX_EXPORT_FILE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_EXPORT_FILES_PER_RUN: usize = 16;
/// Per-conversation cap on regular files kept in the persistent delivery dir
/// `~/Kivio/outputs/<conversation>/`. After each write, the oldest files beyond
/// this many are evicted by mtime so deliverables can't grow unbounded.
/// Must be ≥ [`MAX_EXPORT_FILES_PER_RUN`], otherwise a single run emitting the
/// max number of artifacts would have its own oldest deliverables pruned away.
const DELIVERY_DIR_MAX_FILES: usize = MAX_EXPORT_FILES_PER_RUN;

/// Best-effort: after a file lands in the delivery dir, keep only the newest
/// [`DELIVERY_DIR_MAX_FILES`] **regular files** directly inside `dir` (subdirs
/// ignored), deleting the oldest by mtime. Any stat/remove error is swallowed so
/// pruning never breaks the write that triggered it.
fn prune_delivery_dir(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let meta = fs::metadata(&path).ok()?;
            if !meta.is_file() {
                return None;
            }
            // `meta.json` is bookkeeping, not a deliverable — never count/evict it.
            if path.file_name().and_then(|n| n.to_str()) == Some("meta.json") {
                return None;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, path))
        })
        .collect();
    if files.len() <= DELIVERY_DIR_MAX_FILES {
        return;
    }
    // Oldest first; delete everything past the newest DELIVERY_DIR_MAX_FILES.
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let excess = files.len() - DELIVERY_DIR_MAX_FILES;
    for (_, path) in files.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxExportContext {
    pub conversation_id: String,
    pub message_id: String,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SandboxExportMetaFile {
    name: String,
    path: String,
    mime_type: String,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SandboxExportMeta {
    conversation_id: String,
    message_id: String,
    tool_call_id: Option<String>,
    exported_at: i64,
    files: Vec<SandboxExportMetaFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxExportedArtifact {
    pub artifact_index: usize,
    pub path: PathBuf,
}

fn sanitize_path_segment(raw: &str) -> String {
    let sanitized = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn outputs_root() -> Result<PathBuf, String> {
    Ok(user_home_dir()?.join(OUTPUTS_ROOT))
}

/// Resolve the persistent delivery directory for a conversation:
/// `~/Kivio/outputs/<sanitized_conversation_id>/`. The conversation id segment
/// is sanitized so it can never escape the outputs root.
pub fn delivery_dir(conversation_id: &str) -> Result<PathBuf, String> {
    Ok(outputs_root()?.join(sanitize_path_segment(conversation_id)))
}

/// Resolve and create the delivery directory for a conversation.
pub fn ensure_delivery_dir(conversation_id: &str) -> Result<PathBuf, String> {
    let dir = delivery_dir(conversation_id)?;
    fs::create_dir_all(&dir).map_err(|err| format!("Create delivery dir failed: {err}"))?;
    Ok(dir)
}

/// True when `path` resolves to a location inside the conversation's delivery
/// directory. Used by `write_file` to decide whether a successful write should
/// be surfaced as a downloadable file card. Compares canonicalized paths so a
/// `..` or symlink cannot spoof membership.
pub fn path_under_delivery_dir(conversation_id: &str, path: &Path) -> bool {
    let Ok(dir) = delivery_dir(conversation_id) else {
        return false;
    };
    // The delivery dir may not exist yet; canonicalize the deepest existing
    // ancestor of each side so the comparison is stable on both platforms.
    let canon = |p: &Path| -> PathBuf { canonicalize_lenient(p) };
    let dir_canon = canon(&dir);
    let path_canon = canon(path);
    path_canon.starts_with(&dir_canon)
}

/// Canonicalize a path, falling back to canonicalizing the nearest existing
/// ancestor and re-appending the missing tail (so a not-yet-created file still
/// resolves symlinks/`..` in its parent chain).
fn canonicalize_lenient(path: &Path) -> PathBuf {
    if let Ok(canon) = fs::canonicalize(path) {
        return canon;
    }
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(canon) = fs::canonicalize(current) {
            let mut resolved = canon;
            for name in missing.iter().rev() {
                resolved.push(name);
            }
            return resolved;
        }
        match current.parent() {
            Some(parent) => {
                if let Some(name) = current.file_name() {
                    missing.push(name.to_os_string());
                }
                current = parent;
            }
            None => return path.to_path_buf(),
        }
    }
}

/// Confine an arbitrary host path to the Kivio outputs (delivery) tree. Used by
/// the open/reveal commands so the UI can only act on generated deliverables.
pub fn resolve_sandbox_export_file_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Generated file path is empty".to_string());
    }
    let full = Path::new(trimmed);
    if !full.is_absolute() {
        return Err("Generated file path must be absolute".to_string());
    }
    if !full.is_file() {
        return Err("Generated file does not exist".to_string());
    }
    let canonical_path = fs::canonicalize(full)
        .map_err(|err| format!("Resolve generated file path failed: {err}"))?;
    let canonical_root = fs::canonicalize(outputs_root()?)
        .map_err(|err| format!("Resolve generated file root failed: {err}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("Generated file is outside the Kivio outputs directory".to_string());
    }
    Ok(canonical_path)
}

fn decode_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    let payload = data_url
        .split_once(',')
        .map(|(_, data)| data)
        .ok_or_else(|| "Invalid artifact data URL".to_string())?;
    general_purpose::STANDARD
        .decode(payload)
        .map_err(|err| format!("Decode artifact failed: {err}"))
}

fn sanitize_export_filename(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let sanitized = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches(['.', '_']);
    if trimmed.is_empty() {
        "output.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn unique_export_path(dir: &Path, filename: &str) -> PathBuf {
    let mut candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let ext = Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    for index in 2..=99 {
        candidate = dir.join(format!("{stem}-{index}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}", uuid::Uuid::new_v4()))
}

fn write_export_meta(dir: &Path, meta: &SandboxExportMeta) -> Result<(), String> {
    let json = serde_json::to_string_pretty(meta)
        .map_err(|err| format!("Serialize sandbox export meta failed: {err}"))?;
    fs::write(dir.join("meta.json"), json)
        .map_err(|err| format!("Write sandbox export meta failed: {err}"))
}

fn read_export_meta(dir: &Path) -> Option<SandboxExportMeta> {
    let content = fs::read_to_string(dir.join("meta.json")).ok()?;
    serde_json::from_str(&content).ok()
}

fn merged_export_meta(
    dir: &Path,
    ctx: &SandboxExportContext,
    next_files: Vec<SandboxExportMetaFile>,
) -> SandboxExportMeta {
    let mut files = read_export_meta(dir)
        .map(|meta| meta.files)
        .unwrap_or_default()
        .into_iter()
        .filter(|file| Path::new(&file.path).exists())
        .collect::<Vec<_>>();

    for next in next_files {
        files.retain(|file| file.path != next.path);
        files.push(next);
    }

    SandboxExportMeta {
        conversation_id: ctx.conversation_id.clone(),
        message_id: ctx.message_id.clone(),
        tool_call_id: ctx.tool_call_id.clone(),
        exported_at: chrono::Local::now().timestamp(),
        files,
    }
}

/// Export Pyodide artifacts for one tool run into the conversation's persistent
/// delivery directory `~/Kivio/outputs/<conversation>/`. The written files are
/// the same downloadable deliverables surfaced by `write_file` writing into that
/// dir, so python output and plain writes share one persistent area and render
/// with the identical file card.
pub fn export_sandbox_artifacts(
    ctx: &SandboxExportContext,
    artifacts: &[ChatToolArtifact],
) -> Result<Vec<SandboxExportedArtifact>, String> {
    if artifacts.is_empty() {
        return Ok(Vec::new());
    }

    let export_dir = ensure_delivery_dir(&ctx.conversation_id)?;

    let mut exported = Vec::new();
    let mut meta_files = Vec::new();

    for (artifact_index, artifact) in artifacts.iter().enumerate().take(MAX_EXPORT_FILES_PER_RUN) {
        if artifact.data_url.trim().is_empty() {
            continue;
        }
        let bytes = decode_data_url(&artifact.data_url)?;
        if bytes.is_empty() {
            continue;
        }
        if bytes.len() as u64 > MAX_EXPORT_FILE_BYTES {
            continue;
        }
        let filename = sanitize_export_filename(&artifact.name);
        let path = unique_export_path(&export_dir, &filename);
        fs::write(&path, &bytes).map_err(|err| format!("Write sandbox export failed: {err}"))?;
        meta_files.push(SandboxExportMetaFile {
            name: filename,
            path: path.display().to_string(),
            mime_type: artifact.mime_type.clone(),
            size_bytes: bytes.len() as u64,
        });
        exported.push(SandboxExportedArtifact {
            artifact_index,
            path,
        });
    }

    if !meta_files.is_empty() {
        let meta = merged_export_meta(&export_dir, ctx, meta_files);
        write_export_meta(&export_dir, &meta)?;
    }

    // Cap the persistent delivery dir; just-written files are newest, never evicted.
    prune_delivery_dir(&export_dir);

    Ok(exported)
}

/// Guess a MIME type from a file extension. Used to label deliverables (e.g.
/// `write_file` writing into the delivery dir) when no explicit mime is known.
/// Falls back to `application/octet-stream`.
pub fn guess_mime_from_name(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/x-yaml",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" => "text/plain",
        "css" => "text/css",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    };
    mime.to_string()
}

/// Build a card-ready [`ChatToolArtifact`] from a file that already lives on
/// disk inside the delivery directory. Used by `write_file`: after a successful
/// write into `~/Kivio/outputs/<conversation>/`, the file is surfaced as a
/// downloadable card. The `data_url` (read back) is only populated for files at
/// or under the export size cap so previews/downloads of small files work; for
/// larger files only `path`/`size_bytes` are set (the UI can still open it).
pub fn build_delivery_artifact_for_path(path: &Path) -> Result<ChatToolArtifact, String> {
    let metadata =
        fs::metadata(path).map_err(|err| format!("Stat delivery file failed: {err}"))?;
    let size_bytes = metadata.len();    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("output")
        .to_string();
    let mime_type = guess_mime_from_name(&name);
    let data_url = if size_bytes <= MAX_EXPORT_FILE_BYTES {
        let bytes = fs::read(path).map_err(|err| format!("Read delivery file failed: {err}"))?;
        Some(format!(
            "data:{mime_type};base64,{}",
            general_purpose::STANDARD.encode(&bytes)
        ))
    } else {
        None
    };
    // `path` is already confirmed under the delivery dir by the caller; cap the
    // dir now that this newest file has landed (best-effort, never the evicted one).
    if let Some(parent) = path.parent() {
        prune_delivery_dir(parent);
    }
    Ok(ChatToolArtifact {
        name,
        mime_type,
        data_url: data_url.unwrap_or_default(),
        size_bytes: Some(size_bytes),
        path: Some(path.display().to_string()),
    })
}

pub fn format_exported_paths(exports: &[SandboxExportedArtifact]) -> String {
    if exports.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "delivered files (~/Kivio/outputs/<conversation>/; shown as downloadable cards):"
            .to_string(),
    ];
    for export in exports {
        lines.push(format!("- {}", export.path.display()));
    }
    lines.join("\n")
}

pub fn format_export_error(err: &str) -> String {
    format!("export warning: failed to save files to ~/Kivio/outputs/: {err}")
}

/// Remove the persistent delivery directory for one conversation (when the chat
/// is deleted).
pub fn remove_sandbox_exports_for_conversation(conversation_id: &str) {
    let home = match user_home_dir() {
        Ok(home) => home,
        Err(_) => return,
    };
    let path = home
        .join(OUTPUTS_ROOT)
        .join(sanitize_path_segment(conversation_id));
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    }
    // Also sweep any leftover legacy ephemeral exports for this conversation.
    let legacy = home
        .join(LEGACY_RUNS_ROOT)
        .join(sanitize_path_segment(conversation_id));
    if legacy.is_dir() {
        let _ = fs::remove_dir_all(legacy);
    }
}

fn dir_modified_at(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn remove_dir_if_stale(path: &Path, cutoff: SystemTime, removed: &mut u32, bytes_freed: &mut u64) {
    let modified = match dir_modified_at(path) {
        Some(value) => value,
        None => return,
    };
    if modified >= cutoff {
        return;
    }
    let size = dir_size(path);
    if fs::remove_dir_all(path).is_ok() {
        *removed += 1;
        *bytes_freed += size;
    }
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            total += dir_size(&entry_path);
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

/// Startup GC. The persistent delivery tree (`~/Kivio/outputs/`) is intentionally
/// NOT pruned — deliverables are long-lived and only removed when their
/// conversation is deleted. This only sweeps the LEGACY ephemeral exports tree
/// (`~/Kivio/runs/`) from prior versions, so old `run_python` runs eventually go
/// away. Nothing writes to `~/Kivio/runs/` anymore.
pub fn cleanup_stale_sandbox_exports() {
    let retention = Duration::from_secs(LEGACY_RUNS_RETENTION_DAYS * 24 * 60 * 60);
    let home = match user_home_dir() {
        Ok(home) => home,
        Err(err) => {
            eprintln!("[sandbox-export-cleanup] home dir unavailable: {err}");
            return;
        }
    };

    let mut removed = 0u32;
    let mut bytes_freed = 0u64;

    let runs_root = home.join(LEGACY_RUNS_ROOT);
    if runs_root.is_dir() {
        let cutoff = SystemTime::now()
            .checked_sub(retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if let Ok(conversations) = fs::read_dir(&runs_root) {
            for conv_entry in conversations.flatten() {
                let conv_path = conv_entry.path();
                if !conv_path.is_dir() {
                    continue;
                }
                let Ok(messages) = fs::read_dir(&conv_path) else {
                    continue;
                };
                for msg_entry in messages.flatten() {
                    let msg_path = msg_entry.path();
                    if msg_path.is_dir() {
                        remove_dir_if_stale(&msg_path, cutoff, &mut removed, &mut bytes_freed);
                    }
                }
                if fs::read_dir(&conv_path)
                    .map(|mut dir| dir.next().is_none())
                    .unwrap_or(false)
                {
                    let _ = fs::remove_dir(&conv_path);
                }
            }
        }
    }

    if removed > 0 {
        eprintln!(
            "[sandbox-export-cleanup] removed {removed} stale legacy run folder(s), freed {} KB",
            bytes_freed / 1024
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_sandbox_artifacts_writes_under_outputs_tree() {
        let png = general_purpose::STANDARD.encode([137u8, 80, 78, 71, 13, 10, 26, 10]);
        let artifacts = vec![ChatToolArtifact {
            name: "chart.png".to_string(),
            mime_type: "image/png".to_string(),
            data_url: format!("data:image/png;base64,{png}"),
            size_bytes: Some(8),
            path: None,
        }];
        let ctx = SandboxExportContext {
            conversation_id: "conv_test".to_string(),
            message_id: "msg_test".to_string(),
            tool_call_id: Some("call_test".to_string()),
        };
        let paths = export_sandbox_artifacts(&ctx, &artifacts).expect("export");
        assert_eq!(paths.len(), 1);
        assert!(paths[0].path.exists());
        assert_eq!(paths[0].artifact_index, 0);
        // Persistent delivery dir keyed by conversation only (no message subdir).
        assert!(paths[0]
            .path
            .to_string_lossy()
            .contains("Kivio/outputs/conv_test/chart.png"));
        let meta_path = paths[0].path.parent().expect("parent").join("meta.json");
        assert!(meta_path.exists());
        let _ = fs::remove_dir_all(delivery_dir(&ctx.conversation_id).expect("dir"));
    }

    #[test]
    fn export_sandbox_artifacts_supports_csv() {
        let csv = general_purpose::STANDARD.encode(b"a,b\n1,2\n");
        let artifacts = vec![ChatToolArtifact {
            name: "summary.csv".to_string(),
            mime_type: "text/csv".to_string(),
            data_url: format!("data:text/csv;base64,{csv}"),
            size_bytes: Some(10),
            path: None,
        }];
        let ctx = SandboxExportContext {
            conversation_id: "conv_csv".to_string(),
            message_id: "msg_csv".to_string(),
            tool_call_id: None,
        };
        let paths = export_sandbox_artifacts(&ctx, &artifacts).expect("export csv");
        assert_eq!(paths.len(), 1);
        assert!(paths[0].path.to_string_lossy().ends_with("summary.csv"));
        let _ = fs::remove_dir_all(delivery_dir(&ctx.conversation_id).expect("dir"));
    }

    #[test]
    fn resolve_sandbox_export_file_path_rejects_outside_paths() {
        let outside = std::env::temp_dir().join("kivio-outside-artifact.txt");
        fs::write(&outside, "outside").expect("write outside file");

        let err = resolve_sandbox_export_file_path(&outside.to_string_lossy())
            .expect_err("outside files must be rejected");
        assert!(err.contains("outside the Kivio outputs directory"));

        let _ = fs::remove_file(outside);
    }

    #[test]
    fn delivery_dir_sanitizes_conversation_id_and_confines() {
        // A traversal-style conv id is reduced to a single safe segment under
        // the outputs root — it cannot escape into a sibling/parent directory.
        let dir = delivery_dir("../../etc").expect("dir");
        let root = outputs_root().expect("root");
        assert!(dir.starts_with(&root), "must stay under outputs root: {dir:?}");
        assert!(!dir.to_string_lossy().contains(".."));
    }

    #[test]
    fn path_under_delivery_dir_matches_only_inside() {
        let conv = format!("conv_member_{}", uuid::Uuid::new_v4().simple());
        let dir = ensure_delivery_dir(&conv).expect("dir");
        let inside = dir.join("report.csv");
        fs::write(&inside, "a,b\n1,2\n").expect("write inside");
        assert!(path_under_delivery_dir(&conv, &inside));

        // A path in the project/temp area is NOT under the delivery dir.
        let outside = std::env::temp_dir().join(format!("kivio_outside_{}.txt", uuid::Uuid::new_v4()));
        fs::write(&outside, "x").expect("write outside");
        assert!(!path_under_delivery_dir(&conv, &outside));

        // A traversal attempt out of the delivery dir is rejected by canonicalization.
        let escape = dir.join("../escape.txt");
        assert!(!path_under_delivery_dir(&conv, &escape));

        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_delivery_artifact_for_small_file_has_data_url() {
        let conv = format!("conv_artifact_{}", uuid::Uuid::new_v4().simple());
        let dir = ensure_delivery_dir(&conv).expect("dir");
        let file = dir.join("notes.md");
        fs::write(&file, "# Hello\n\nWorld\n").expect("write");

        let artifact = build_delivery_artifact_for_path(&file).expect("artifact");
        assert_eq!(artifact.name, "notes.md");
        assert_eq!(artifact.mime_type, "text/markdown");
        assert_eq!(artifact.size_bytes, Some("# Hello\n\nWorld\n".len() as u64));
        assert_eq!(artifact.path.as_deref(), Some(file.to_string_lossy().as_ref()));
        assert!(artifact.data_url.starts_with("data:text/markdown;base64,"));
        let payload = artifact.data_url.split_once(',').expect("data url").1;
        let decoded = general_purpose::STANDARD.decode(payload).expect("decode");
        assert_eq!(decoded, b"# Hello\n\nWorld\n");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_delivery_artifact_for_oversize_file_omits_data_url() {
        let conv = format!("conv_oversize_{}", uuid::Uuid::new_v4().simple());
        let dir = ensure_delivery_dir(&conv).expect("dir");
        let file = dir.join("big.bin");
        let huge = vec![0u8; (MAX_EXPORT_FILE_BYTES + 1) as usize];
        fs::write(&file, &huge).expect("write big");

        let artifact = build_delivery_artifact_for_path(&file).expect("artifact");
        assert_eq!(artifact.size_bytes, Some(MAX_EXPORT_FILE_BYTES + 1));
        assert!(artifact.data_url.is_empty(), "oversize file omits data_url");
        assert_eq!(artifact.path.as_deref(), Some(file.to_string_lossy().as_ref()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_sandbox_artifacts_merges_meta_across_calls() {
        let first = general_purpose::STANDARD.encode(b"a,b\n1,2\n");
        let second = general_purpose::STANDARD.encode(b"# Report\n\nDone.\n");
        let ctx = SandboxExportContext {
            conversation_id: format!("conv_merge_{}", uuid::Uuid::new_v4().simple()),
            message_id: "msg_merge".to_string(),
            tool_call_id: Some("call_latest".to_string()),
        };

        let first_paths = export_sandbox_artifacts(
            &ctx,
            &[ChatToolArtifact {
                name: "summary.csv".to_string(),
                mime_type: "text/csv".to_string(),
                data_url: format!("data:text/csv;base64,{first}"),
                size_bytes: Some(8),
                path: None,
            }],
        )
        .expect("first export");
        let second_paths = export_sandbox_artifacts(
            &ctx,
            &[ChatToolArtifact {
                name: "report.md".to_string(),
                mime_type: "text/markdown".to_string(),
                data_url: format!("data:text/markdown;base64,{second}"),
                size_bytes: Some(16),
                path: None,
            }],
        )
        .expect("second export");

        let export_dir = delivery_dir(&ctx.conversation_id).expect("dir");
        let meta = read_export_meta(&export_dir).expect("meta should deserialize");
        let meta_paths = meta.files.iter().map(|file| &file.path).collect::<Vec<_>>();

        assert_eq!(first_paths.len(), 1);
        assert_eq!(second_paths.len(), 1);
        assert_eq!(meta.files.len(), 2);
        assert!(meta_paths.contains(&&first_paths[0].path.display().to_string()));
        assert!(meta_paths.contains(&&second_paths[0].path.display().to_string()));

        let _ = fs::remove_dir_all(&export_dir);
    }

    /// Write `count` files into `dir` named `f00.txt`, `f01.txt`, … with strictly
    /// increasing mtimes (oldest first). Returns the created paths in write order.
    fn write_files_with_increasing_mtime(dir: &Path, count: usize) -> Vec<PathBuf> {
        let mut paths = Vec::with_capacity(count);
        for index in 0..count {
            let path = dir.join(format!("f{index:02}.txt"));
            fs::write(&path, format!("file {index}")).expect("write");
            // Sub-second sleep so each subsequent file has a strictly newer mtime.
            std::thread::sleep(Duration::from_millis(20));
            paths.push(path);
        }
        paths
    }

    #[test]
    fn prune_delivery_dir_keeps_newest_and_drops_oldest() {
        let conv = format!("conv_prune_{}", uuid::Uuid::new_v4().simple());
        let dir = ensure_delivery_dir(&conv).expect("dir");
        // Write DELIVERY_DIR_MAX_FILES + 1 files; the oldest (index 0) must be evicted.
        let paths = write_files_with_increasing_mtime(&dir, DELIVERY_DIR_MAX_FILES + 1);

        prune_delivery_dir(&dir);

        let remaining = fs::read_dir(&dir)
            .expect("read dir")
            .flatten()
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(remaining, DELIVERY_DIR_MAX_FILES, "cap-many files remain");
        assert!(!paths[0].exists(), "oldest file was deleted");
        assert!(
            paths[DELIVERY_DIR_MAX_FILES].exists(),
            "newest file is kept"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_delivery_dir_noop_when_under_cap() {
        let conv = format!("conv_undercap_{}", uuid::Uuid::new_v4().simple());
        let dir = ensure_delivery_dir(&conv).expect("dir");
        let paths = write_files_with_increasing_mtime(&dir, DELIVERY_DIR_MAX_FILES);

        prune_delivery_dir(&dir);

        for path in &paths {
            assert!(path.exists(), "files at/under cap are untouched");
        }
        assert_eq!(paths.len(), DELIVERY_DIR_MAX_FILES);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_delivery_dir_ignores_subdirectories() {
        let conv = format!("conv_subdir_{}", uuid::Uuid::new_v4().simple());
        let dir = ensure_delivery_dir(&conv).expect("dir");
        // A subdir plus 16 regular files: subdir is neither counted nor removed.
        let subdir = dir.join("nested");
        fs::create_dir_all(&subdir).expect("create subdir");
        let paths = write_files_with_increasing_mtime(&dir, DELIVERY_DIR_MAX_FILES + 1);

        prune_delivery_dir(&dir);

        assert!(subdir.is_dir(), "subdirectory is preserved");
        let remaining_files = fs::read_dir(&dir)
            .expect("read dir")
            .flatten()
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(remaining_files, DELIVERY_DIR_MAX_FILES);
        assert!(!paths[0].exists(), "oldest regular file evicted");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_sandbox_artifacts_prunes_to_cap() {
        let conv = format!("conv_export_prune_{}", uuid::Uuid::new_v4().simple());
        let dir = ensure_delivery_dir(&conv).expect("dir");
        // Seed 15 older files so the next export pushes the dir over the cap.
        let seeded = write_files_with_increasing_mtime(&dir, DELIVERY_DIR_MAX_FILES);
        std::thread::sleep(Duration::from_millis(20));

        let png = general_purpose::STANDARD.encode([137u8, 80, 78, 71, 13, 10, 26, 10]);
        let ctx = SandboxExportContext {
            conversation_id: conv.clone(),
            message_id: "msg".to_string(),
            tool_call_id: None,
        };
        let exported = export_sandbox_artifacts(
            &ctx,
            &[ChatToolArtifact {
                name: "chart.png".to_string(),
                mime_type: "image/png".to_string(),
                data_url: format!("data:image/png;base64,{png}"),
                size_bytes: Some(8),
                path: None,
            }],
        )
        .expect("export");

        // The freshly exported file survives; the oldest seeded file is evicted.
        assert_eq!(exported.len(), 1);
        assert!(exported[0].path.exists(), "newly exported file kept");
        assert!(!seeded[0].exists(), "oldest pre-existing file evicted");
        let remaining_files = fs::read_dir(&dir)
            .expect("read dir")
            .flatten()
            .filter(|e| {
                let p = e.path();
                p.is_file() && p.file_name().and_then(|n| n.to_str()) != Some("meta.json")
            })
            .count();
        assert_eq!(remaining_files, DELIVERY_DIR_MAX_FILES);

        let _ = fs::remove_dir_all(&dir);
    }
}
