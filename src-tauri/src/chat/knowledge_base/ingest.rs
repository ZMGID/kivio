//! Document ingest: upload → parse → chunk → embed → store, with background
//! indexing and `kb-index` progress events. Re-index (e.g. after the embedding
//! model changes) re-runs the same pipeline from the stored source snapshots.

use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::api::effective_retry_attempts;
use crate::state::AppState;

use super::{
    chunking, embeddings, parse, refresh_library_counts, sources_dir, DocStatus, KnowledgeChunk,
    KnowledgeDocument,
};

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct KbIndexEvent {
    kb_id: String,
    doc_id: String,
    status: String,
    indexed: usize,
    total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn emit_index(app: &AppHandle, kb_id: &str, doc_id: &str, status: &str, indexed: usize, total: usize, error: Option<String>) {
    let _ = app.emit(
        "kb-index",
        KbIndexEvent {
            kb_id: kb_id.to_string(),
            doc_id: doc_id.to_string(),
            status: status.to_string(),
            indexed,
            total,
            error,
        },
    );
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

fn find_source(app: &AppHandle, kb_id: &str, doc_id: &str) -> Result<PathBuf, String> {
    let dir = sources_dir(app, kb_id)?;
    let prefix = format!("{doc_id}__");
    for entry in fs::read_dir(&dir).map_err(|e| format!("read sources dir: {e}"))? {
        let entry = entry.map_err(|e| format!("read sources entry: {e}"))?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            return Ok(entry.path());
        }
    }
    Err(format!("source snapshot missing for {doc_id}"))
}

fn set_doc_status(
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    status: DocStatus,
    chunk_count: usize,
    error: Option<String>,
) -> Result<(), String> {
    let mut docs = super::load_docs(app, kb_id)?;
    if let Some(doc) = docs.iter_mut().find(|d| d.id == doc_id) {
        doc.status = status;
        doc.chunk_count = chunk_count;
        doc.error = error;
    }
    super::save_docs(app, kb_id, &docs)
}

/// The actual pipeline for one document. Returns the chunk count on success.
async fn index_one(app: &AppHandle, kb_id: &str, doc_id: &str) -> Result<usize, String> {
    let lib = super::get_library(app, kb_id)?;
    let doc_name = super::load_docs(app, kb_id)?
        .into_iter()
        .find(|d| d.id == doc_id)
        .map(|d| d.name)
        .unwrap_or_else(|| doc_id.to_string());

    let path = find_source(app, kb_id, doc_id)?;
    let parsed = parse::parse_file(&path)?;
    let pieces = chunking::chunk_document(&parsed.text, parsed.markdown);

    let state = app.state::<AppState>();
    let state: &AppState = &state;

    // Resolve provider + retry budget under the lock, then drop the guard
    // before any await (never hold a std lock across .await).
    let (provider, attempts) = {
        let settings = state.settings_read();
        let provider = settings
            .get_provider(&lib.embedding_provider_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Embedding provider '{}' not found",
                    lib.embedding_provider_id
                )
            })?;
        (provider, effective_retry_attempts(&settings))
    };

    let total = pieces.len();
    emit_index(app, kb_id, doc_id, "indexing", 0, total, None);

    if pieces.is_empty() {
        // Nothing to embed — drop any stale chunks for this doc and finish.
        let mut all = super::load_chunks(app, kb_id)?;
        all.retain(|c| c.doc_id != doc_id);
        super::save_chunks(app, kb_id, &all)?;
        return Ok(0);
    }

    let texts: Vec<String> = pieces.iter().map(|p| p.text.clone()).collect();

    const BATCH: usize = 64;
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for batch in texts.chunks(BATCH) {
        let mut got =
            embeddings::embed_batch(state, &provider, &lib.embedding_model, batch, attempts).await?;
        vectors.append(&mut got);
        emit_index(app, kb_id, doc_id, "indexing", vectors.len(), total, None);
    }

    let new_chunks: Vec<KnowledgeChunk> = pieces
        .into_iter()
        .zip(vectors)
        .enumerate()
        .map(|(order, (piece, embedding))| KnowledgeChunk {
            id: super::gen_id("chunk"),
            doc_id: doc_id.to_string(),
            doc_name: doc_name.clone(),
            text: piece.text,
            heading_path: piece.heading_path,
            page: None,
            char_start: piece.char_start,
            char_end: piece.char_end,
            order_index: order,
            embedding,
        })
        .collect();

    // Replace this doc's chunks atomically (remove old, append new).
    let mut all = super::load_chunks(app, kb_id)?;
    all.retain(|c| c.doc_id != doc_id);
    all.extend(new_chunks);
    let n = all.iter().filter(|c| c.doc_id == doc_id).count();
    super::save_chunks(app, kb_id, &all)?;
    Ok(n)
}

/// Per-kb async lock registry. Indexing a library is a non-atomic
/// read-modify-write over its JSON files (docs/chunks/library counts), so all
/// index writes for one kb must be serialized or concurrent uploads silently
/// overwrite each other's chunks (lost update).
/// ponytail: process-global registry of per-kb locks. Kivio is a single-user
/// desktop app; a global map keyed by kb_id is enough — no AppState field, no
/// 3-constructor churn. Move into AppState if this ever needs per-window scope.
fn kb_lock_for(kb_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(kb_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Apply the terminal status + counts + event for one finished document.
fn finish_index(app: &AppHandle, kb_id: &str, doc_id: &str, result: Result<usize, String>) {
    match result {
        Ok(n) => {
            let _ = set_doc_status(app, kb_id, doc_id, DocStatus::Ready, n, None);
            let _ = refresh_library_counts(app, kb_id);
            emit_index(app, kb_id, doc_id, "ready", n, n, None);
        }
        Err(e) => {
            let _ = set_doc_status(app, kb_id, doc_id, DocStatus::Error, 0, Some(e.clone()));
            let _ = refresh_library_counts(app, kb_id);
            emit_index(app, kb_id, doc_id, "error", 0, 0, Some(e));
        }
    }
}

/// Upload path: index one document under the kb's serialization lock, then emit
/// its terminal state. The lock is held across the embedding awaits so two
/// uploads to the same library never interleave their chunks.json read-modify-write.
async fn run_index(app: AppHandle, kb_id: String, doc_id: String) {
    let lock = kb_lock_for(&kb_id);
    let _guard = lock.lock().await;
    let result = index_one(&app, &kb_id, &doc_id).await;
    finish_index(&app, &kb_id, &doc_id, result);
}

// ===== commands =====

/// Import a file into a library. Reads the dropped path, snapshots it under
/// `sources/`, registers the document as `indexing`, and kicks off background
/// indexing. Returns the freshly-created (or existing, on hash dedup) document.
#[tauri::command]
pub(crate) async fn kb_upload_document(
    app: AppHandle,
    kb_id: String,
    file_path: String,
) -> Result<KnowledgeDocument, String> {
    // Validate the library exists up front.
    let _lib = super::get_library(&app, &kb_id)?;

    let src = PathBuf::from(&file_path);
    if !parse::is_supported_ext(&src) {
        return Err(format!(
            "Unsupported file type: {}",
            src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ));
    }
    let bytes = fs::read(&src).map_err(|e| format!("read {}: {e}", src.display()))?;
    if bytes.len() as u64 > parse::MAX_DOC_BYTES {
        return Err(format!(
            "file too large: {} bytes (max {})",
            bytes.len(),
            parse::MAX_DOC_BYTES
        ));
    }
    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    };

    let file_name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled".to_string());

    let mut docs = super::load_docs(&app, &kb_id)?;
    // Dedup by content hash: re-uploading the same file is a no-op.
    if let Some(existing) = docs.iter().find(|d| d.hash == hash) {
        return Ok(existing.clone());
    }

    let doc_id = super::gen_id("doc");
    let dest = sources_dir(&app, &kb_id)?.join(format!(
        "{doc_id}__{}",
        sanitize_filename(&file_name)
    ));
    fs::write(&dest, &bytes).map_err(|e| format!("write source snapshot: {e}"))?;

    let doc = KnowledgeDocument {
        id: doc_id.clone(),
        name: file_name,
        size_bytes: bytes.len() as u64,
        hash,
        chunk_count: 0,
        status: DocStatus::Indexing,
        error: None,
        created_at: chrono::Local::now().timestamp(),
    };
    docs.push(doc.clone());
    // Roll back the just-written source snapshot if registration fails, so a
    // failed upload doesn't leave an unreachable orphan file under sources/.
    if let Err(e) = super::save_docs(&app, &kb_id, &docs) {
        let _ = fs::remove_file(&dest);
        return Err(e);
    }
    let _ = refresh_library_counts(&app, &kb_id);

    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        run_index(app2, kb_id, doc_id).await;
    });

    Ok(doc)
}

/// Re-index every document in a library from its stored source snapshot. Used
/// after the embedding model changes (vectors of the old dimension are dropped
/// first). Documents are flipped to `indexing` immediately; work runs in the
/// background, one document at a time.
#[tauri::command]
pub(crate) async fn kb_reindex_library(app: AppHandle, kb_id: String) -> Result<(), String> {
    let _lib = super::get_library(&app, &kb_id)?;
    let mut docs = super::load_docs(&app, &kb_id)?;
    for doc in docs.iter_mut() {
        doc.status = DocStatus::Indexing;
        doc.error = None;
    }
    super::save_docs(&app, &kb_id, &docs)?;

    let ids: Vec<String> = docs.into_iter().map(|d| d.id).collect();
    let app2 = app.clone();
    // Whole reindex runs as ONE locked unit: clear + per-doc indexing happen
    // under the kb lock so a concurrent upload can't interleave with the clear
    // or with a doc's chunk write. We call `index_one` directly (not `run_index`)
    // because `run_index` re-acquires the same lock (tokio Mutex is not reentrant).
    tauri::async_runtime::spawn(async move {
        let lock = kb_lock_for(&kb_id);
        let _guard = lock.lock().await;
        // Drop all chunks now — they may be the wrong dimension after a model swap.
        let _ = super::save_chunks(&app2, &kb_id, &[]);
        for doc_id in &ids {
            let result = index_one(&app2, &kb_id, doc_id).await;
            finish_index(&app2, &kb_id, doc_id, result);
        }
        // Empty library emits no per-doc event; refresh counts so the UI's
        // library card converges (chunk_count → 0) without a manual reload.
        if ids.is_empty() {
            let _ = refresh_library_counts(&app2, &kb_id);
        }
    });
    Ok(())
}

/// Change a library's embedding provider/model, then re-index from sources.
/// The dimension is reset to 0 and re-learned from the first new embedding.
#[tauri::command]
pub(crate) async fn kb_update_embedding(
    app: AppHandle,
    kb_id: String,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    if provider_id.trim().is_empty() || model.trim().is_empty() {
        return Err("Embedding provider and model are required".to_string());
    }
    let mut libs = super::load_libraries(&app)?;
    let lib = libs
        .iter_mut()
        .find(|l| l.id == kb_id)
        .ok_or_else(|| format!("Knowledge base not found: {kb_id}"))?;
    lib.embedding_provider_id = provider_id;
    lib.embedding_model = model;
    lib.embedding_dim = 0;
    lib.updated_at = chrono::Local::now().timestamp();
    super::save_libraries(&app, &libs)?;

    kb_reindex_library(app, kb_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_sanitization() {
        assert_eq!(sanitize_filename("a b/c.txt"), "a_b_c.txt");
        assert_eq!(sanitize_filename("报告.pdf"), "报告.pdf"); // CJK is alphanumeric
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename("../etc/passwd"), ".._etc_passwd");
    }
}
