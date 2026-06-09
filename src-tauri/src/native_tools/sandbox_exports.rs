use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

use super::user_home_dir;
use crate::mcp::types::ChatToolArtifact;

const RUNS_ROOT: &str = "Kivio/runs";
const LEGACY_OUTPUTS_DIR: &str = "Kivio/outputs";
const DEFAULT_RETENTION_DAYS: u64 = 7;
const MAX_EXPORT_FILE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_EXPORT_FILES_PER_RUN: usize = 16;

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

pub fn sandbox_run_export_dir(ctx: &SandboxExportContext) -> Result<PathBuf, String> {
    let home = user_home_dir()?;
    Ok(home
        .join(RUNS_ROOT)
        .join(sanitize_path_segment(&ctx.conversation_id))
        .join(sanitize_path_segment(&ctx.message_id)))
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

/// Export Pyodide artifacts for one tool run into `~/Kivio/runs/{conversation}/{message}/`.
pub fn export_sandbox_artifacts(
    ctx: &SandboxExportContext,
    artifacts: &[ChatToolArtifact],
) -> Result<Vec<PathBuf>, String> {
    if artifacts.is_empty() {
        return Ok(Vec::new());
    }

    let export_dir = sandbox_run_export_dir(ctx)?;
    fs::create_dir_all(&export_dir)
        .map_err(|err| format!("Create sandbox export dir failed: {err}"))?;

    let mut exported = Vec::new();
    let mut meta_files = Vec::new();

    for artifact in artifacts.iter().take(MAX_EXPORT_FILES_PER_RUN) {
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
        exported.push(path);
    }

    if !meta_files.is_empty() {
        let meta = merged_export_meta(&export_dir, ctx, meta_files);
        write_export_meta(&export_dir, &meta)?;
    }

    Ok(exported)
}

pub fn format_exported_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "exported files (~/Kivio/runs/<conversation>/<message>/; retained ~7 days):".to_string(),
    ];
    for path in paths {
        lines.push(format!("- {}", path.display()));
    }
    lines.join("\n")
}

pub fn format_export_error(err: &str) -> String {
    format!("export warning: failed to save sandbox files to ~/Kivio/runs/: {err}")
}

/// Remove all sandbox exports for one conversation (e.g. when the chat is deleted).
pub fn remove_sandbox_exports_for_conversation(conversation_id: &str) {
    let home = match user_home_dir() {
        Ok(home) => home,
        Err(_) => return,
    };
    let path = home
        .join(RUNS_ROOT)
        .join(sanitize_path_segment(conversation_id));
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
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

/// Remove sandbox run folders older than the default retention window. Also prunes legacy flat `outputs/`.
pub fn cleanup_stale_sandbox_exports() {
    let retention = Duration::from_secs(DEFAULT_RETENTION_DAYS * 24 * 60 * 60);
    let home = match user_home_dir() {
        Ok(home) => home,
        Err(err) => {
            eprintln!("[sandbox-export-cleanup] home dir unavailable: {err}");
            return;
        }
    };

    let mut removed = 0u32;
    let mut bytes_freed = 0u64;

    let runs_root = home.join(RUNS_ROOT);
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

    let legacy_outputs = home.join(LEGACY_OUTPUTS_DIR);
    if legacy_outputs.is_dir() {
        let cutoff = SystemTime::now()
            .checked_sub(retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if let Ok(entries) = fs::read_dir(&legacy_outputs) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    remove_dir_if_stale(&path, cutoff, &mut removed, &mut bytes_freed);
                    continue;
                }
                let modified = match fs::metadata(&path).and_then(|meta| meta.modified()) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if modified >= cutoff {
                    continue;
                }
                let size = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                if fs::remove_file(&path).is_ok() {
                    removed += 1;
                    bytes_freed += size;
                }
            }
        }
    }

    if removed > 0 {
        eprintln!(
            "[sandbox-export-cleanup] removed {removed} stale export folder(s), freed {} KB",
            bytes_freed / 1024
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_sandbox_artifacts_writes_under_runs_tree() {
        let png = general_purpose::STANDARD.encode([137u8, 80, 78, 71, 13, 10, 26, 10]);
        let artifacts = vec![ChatToolArtifact {
            name: "chart.png".to_string(),
            mime_type: "image/png".to_string(),
            data_url: format!("data:image/png;base64,{png}"),
            size_bytes: Some(8),
        }];
        let ctx = SandboxExportContext {
            conversation_id: "conv_test".to_string(),
            message_id: "msg_test".to_string(),
            tool_call_id: Some("call_test".to_string()),
        };
        let paths = export_sandbox_artifacts(&ctx, &artifacts).expect("export");
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());
        assert!(paths[0]
            .to_string_lossy()
            .contains("Kivio/runs/conv_test/msg_test/chart.png"));
        let meta_path = paths[0].parent().expect("parent").join("meta.json");
        assert!(meta_path.exists());
        let _ = fs::remove_dir_all(
            sandbox_run_export_dir(&ctx)
                .expect("dir")
                .parent()
                .expect("conv"),
        );
    }

    #[test]
    fn export_sandbox_artifacts_supports_csv() {
        let csv = general_purpose::STANDARD.encode(b"a,b\n1,2\n");
        let artifacts = vec![ChatToolArtifact {
            name: "summary.csv".to_string(),
            mime_type: "text/csv".to_string(),
            data_url: format!("data:text/csv;base64,{csv}"),
            size_bytes: Some(10),
        }];
        let ctx = SandboxExportContext {
            conversation_id: "conv_csv".to_string(),
            message_id: "msg_csv".to_string(),
            tool_call_id: None,
        };
        let paths = export_sandbox_artifacts(&ctx, &artifacts).expect("export csv");
        assert_eq!(paths.len(), 1);
        assert!(paths[0].to_string_lossy().ends_with("summary.csv"));
        let _ = fs::remove_dir_all(sandbox_run_export_dir(&ctx).expect("dir"));
    }

    #[test]
    fn export_sandbox_artifacts_merges_meta_across_calls() {
        let first = general_purpose::STANDARD.encode(b"a,b\n1,2\n");
        let second = general_purpose::STANDARD.encode(b"# Report\n\nDone.\n");
        let ctx = SandboxExportContext {
            conversation_id: "conv_merge".to_string(),
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
            }],
        )
        .expect("second export");

        let export_dir = sandbox_run_export_dir(&ctx).expect("dir");
        let meta = read_export_meta(&export_dir).expect("meta should deserialize");
        let meta_paths = meta.files.iter().map(|file| &file.path).collect::<Vec<_>>();

        assert_eq!(first_paths.len(), 1);
        assert_eq!(second_paths.len(), 1);
        assert_eq!(meta.files.len(), 2);
        assert!(meta_paths.contains(&&first_paths[0].display().to_string()));
        assert!(meta_paths.contains(&&second_paths[0].display().to_string()));

        let _ = fs::remove_dir_all(export_dir.parent().expect("conv"));
    }
}
