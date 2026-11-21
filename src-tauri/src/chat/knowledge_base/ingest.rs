//! Document ingest: upload → parse → chunk → embed → store, with background
//! indexing and `kb-index` progress events. Re-index (e.g. after the embedding
//! model changes) re-runs the same pipeline from the stored source snapshots.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::api::effective_retry_attempts;
use crate::state::AppState;

use super::{
    chunking, embeddings, parse, refresh_library_counts, sources_dir, DocStatus, KnowledgeChunk,
    KnowledgeDocument,
};

/// Default embedding-request batch size when a library doesn't override it.
pub const DEFAULT_EMBED_BATCH: usize = 64;
static UPLOAD_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static INDEX_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

fn read_source(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size > parse::MAX_DOC_BYTES {
        return Err(format!(
            "file too large: {size} bytes (max {})",
            parse::MAX_DOC_BYTES
        ));
    }
    let mut bytes = Vec::new();
    file.take(parse::MAX_DOC_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > parse::MAX_DOC_BYTES {
        return Err("file grew beyond the document size limit".into());
    }
    Ok(bytes)
}

/// Lowercase hex SHA256 of `bytes` — content hash used for upload/import dedup.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

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

fn emit_index(
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    status: &str,
    indexed: usize,
    total: usize,
    error: Option<String>,
) {
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

pub(super) fn find_source(app: &AppHandle, kb_id: &str, doc_id: &str) -> Result<PathBuf, String> {
    let dir = sources_dir(app, kb_id)?;
    let prefix = format!("{doc_id}__");
    for entry in fs::read_dir(&dir).map_err(|e| format!("read sources dir: {e}"))? {
        let entry = entry.map_err(|e| format!("read sources entry: {e}"))?;
        if entry.file_name().to_string_lossy().starts_with(&prefix)
            && entry.file_type().map_err(|e| e.to_string())?.is_file()
        {
            return Ok(entry.path());
        }
    }
    Err(format!("source snapshot missing for {doc_id}"))
}

/// Prepare the replacement without touching the searchable index.
async fn build_doc_index(
    app: &AppHandle,
    lib: &super::KnowledgeLibrary,
    doc: &KnowledgeDocument,
) -> Result<Vec<KnowledgeChunk>, String> {
    let _slot = INDEX_SLOTS.acquire().await.map_err(|e| e.to_string())?;
    let kb_id = &lib.id;
    let doc_id = &doc.id;
    let path = find_source(app, kb_id, doc_id)?;

    let state = app.state::<AppState>();
    let state: &AppState = &state;

    // Resolve provider + retry budget + doc-processing config under the lock,
    // then drop the guard before any await (never hold a std lock across .await).
    let (provider, attempts, doc_cfg, chunk_target) = {
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
        (
            provider,
            effective_retry_attempts(&settings),
            settings.document_processing.clone(),
            (settings.knowledge_base.chunk_tokens as usize).clamp(256, 8192),
        )
    };

    // Built-in parse (txt/md/html/pdf-text/docx/xlsx), or image → OCR per the config.
    let parsed = super::process::process_document(state, &doc_cfg, &path).await?;
    let pieces = tauri::async_runtime::spawn_blocking(move || {
        let split = |text: &str| {
            chunking::chunk_with(
                text,
                parsed.markdown,
                chunk_target,
                chunking::OVERLAP_TOKENS,
                chunking::MIN_TOKENS,
            )
        };
        if parsed.page_ranges.is_empty() {
            return split(&parsed.text)
                .into_iter()
                .map(|piece| (piece, None))
                .collect::<Vec<_>>();
        }
        let mut pieces = Vec::new();
        let mut char_offset = 0;
        for (index, range) in parsed.page_ranges.into_iter().enumerate() {
            let page = &parsed.text[range];
            pieces.extend(split(page).into_iter().map(|mut piece| {
                piece.char_start += char_offset;
                piece.char_end += char_offset;
                (piece, Some(index + 1))
            }));
            char_offset += page.chars().count() + 1;
        }
        pieces
    })
    .await
    .map_err(|e| e.to_string())?;

    let total = pieces.len();
    emit_index(app, kb_id, doc_id, "indexing", 0, total, None);

    if pieces.is_empty() {
        return Err("No indexable text was extracted from this document.".into());
    }

    let texts: Vec<String> = pieces.iter().map(|(piece, _)| piece.text.clone()).collect();

    // 每次 embedding 请求打包多少片段。每库可调（lib.embed_batch_size），0 = 用默认。
    // 上限夹到 128：过大易撞 provider 单请求条数/token 上限。
    let batch = if lib.embed_batch_size == 0 {
        DEFAULT_EMBED_BATCH
    } else {
        lib.embed_batch_size.clamp(1, 128)
    };
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for batch in texts.chunks(batch) {
        let mut got = embeddings::embed_batch(
            state,
            &provider,
            &lib.embedding_model,
            batch,
            embeddings::EmbeddingRole::Document,
            attempts,
        )
        .await?;
        vectors.append(&mut got);
        emit_index(app, kb_id, doc_id, "indexing", vectors.len(), total, None);
    }

    let chunks: Vec<KnowledgeChunk> = pieces
        .into_iter()
        .zip(vectors)
        .enumerate()
        .map(|(order, ((piece, page), embedding))| KnowledgeChunk {
            id: super::gen_id("chunk"),
            doc_id: doc_id.clone(),
            doc_name: doc.name.clone(),
            text: piece.text,
            heading_path: piece.heading_path,
            page,
            char_start: piece.char_start,
            char_end: piece.char_end,
            order_index: order,
            embedding,
        })
        .collect();

    let dim = chunks[0].embedding.len();
    if dim == 0
        || chunks.iter().any(|chunk| {
            chunk.embedding.len() != dim || chunk.embedding.iter().any(|value| !value.is_finite())
        })
    {
        return Err("Embedding service returned invalid or inconsistent vectors.".into());
    }
    Ok(chunks)
}

/// Serialize operations that change a library's sources or published index.
fn kb_lock_registry() -> &'static std::sync::Mutex<
    std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
> {
    use std::sync::{Mutex, OnceLock};
    static LOCKS: OnceLock<
        Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    > = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub(super) fn kb_lock_for(kb_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let mut guard = kb_lock_registry().lock().unwrap_or_else(|e| e.into_inner());
    guard.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = guard.get(kb_id).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    guard.insert(kb_id.to_string(), std::sync::Arc::downgrade(&lock));
    lock
}

fn prune_kb_locks() {
    kb_lock_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, lock| lock.strong_count() > 0);
}

async fn index_document(
    app: &AppHandle,
    lib: &super::KnowledgeLibrary,
    doc: &KnowledgeDocument,
) -> Result<(), String> {
    let result = async {
        super::set_doc_status(
            app,
            &lib.id,
            &doc.id,
            DocStatus::Indexing,
            doc.chunk_count,
            None,
        )?;
        emit_index(app, &lib.id, &doc.id, "indexing", 0, 0, None);
        let chunks = build_doc_index(app, lib, doc).await?;
        let dim = chunks[0].embedding.len();
        let root = super::kb_root(app)?;
        super::store::commit_doc_index(
            &super::open_library_index(&root, lib)?,
            &doc.id,
            dim,
            &chunks,
        )?;
        Ok::<_, String>((dim, chunks.len()))
    }
    .await;
    match result {
        Ok((dim, count)) => {
            let metadata = super::set_library_dim(app, &lib.id, dim)
                .and_then(|()| refresh_library_counts(app, &lib.id));
            emit_index(app, &lib.id, &doc.id, "ready", count, count, None);
            metadata
        }
        Err(error) => {
            super::set_doc_status(
                app,
                &lib.id,
                &doc.id,
                DocStatus::Error,
                doc.chunk_count,
                Some(&error),
            )?;
            emit_index(app, &lib.id, &doc.id, "error", 0, 0, Some(error.clone()));
            Err(error)
        }
    }
}

/// Imported sources are registered before the background index starts.
async fn run_index(app: AppHandle, kb_id: String, doc_id: String) {
    let lock = kb_lock_for(&kb_id);
    {
        let _guard = lock.lock().await;
        let result = async {
            let lib = super::get_library(&app, &kb_id)?;
            let doc = super::load_docs(&app, &kb_id)?
                .into_iter()
                .find(|doc| doc.id == doc_id)
                .ok_or_else(|| format!("Document {doc_id} no longer exists"))?;
            if doc.status != DocStatus::Ready {
                index_document(&app, &lib, &doc).await?;
            }
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = result {
            eprintln!("Index {kb_id}/{doc_id}: {error}");
        }
    }
    drop(lock);
    prune_kb_locks();
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
    super::heal_stale_indexing_once(&app);
    let _slot = UPLOAD_SLOTS.acquire().await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _slot = _slot;
        let src = PathBuf::from(&file_path);
        if !parse::is_supported_ext(&src) {
            return Err(format!(
                "Unsupported file type: {}",
                src.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ));
        }
        let bytes = read_source(&src)?;
        let hash = sha256_hex(&bytes);

        let file_name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());

        let write = super::LIBRARY_WRITES
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        super::get_library(&app, &kb_id)?;
        // Dedup by content hash: re-uploading the same file is a no-op.
        if let Some(existing) = super::doc_by_hash(&app, &kb_id, &hash)? {
            if existing.status == DocStatus::Error {
                let app2 = app.clone();
                let doc_id = existing.id.clone();
                drop(write);
                tauri::async_runtime::spawn(run_index(app2, kb_id, doc_id));
            }
            return Ok(existing);
        }

        let doc_id = super::gen_id("doc");
        let dest =
            sources_dir(&app, &kb_id)?.join(format!("{doc_id}__{}", sanitize_filename(&file_name)));
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
            source_url: None,
        };
        // Roll back the just-written source snapshot if registration fails, so a
        // failed upload doesn't leave an unreachable orphan file under sources/.
        if let Err(e) = super::insert_doc(&app, &kb_id, &doc) {
            let _ = fs::remove_file(&dest);
            return Err(e);
        }
        drop(write);
        let _ = refresh_library_counts(&app, &kb_id);

        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            run_index(app2, kb_id, doc_id).await;
        });

        Ok(doc)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Import a new URL or replace its current revision after the new one is indexed.
#[tauri::command]
pub(crate) async fn kb_import_url(
    app: AppHandle,
    kb_id: String,
    url: String,
    doc_id: Option<String>,
) -> Result<KnowledgeDocument, String> {
    super::heal_stale_indexing_once(&app);
    let _lib = super::get_library(&app, &kb_id)?;
    let mut url = reqwest::Url::parse(url.trim()).map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http(s) URLs are supported".to_string());
    }
    url.set_fragment(None);
    let url = url.to_string();
    let text = fetch_web_text(&app, &url).await?;
    let title = truncate_name(
        &text
            .lines()
            .find_map(|line| {
                line.strip_prefix("# ")
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
            })
            .unwrap_or(&url),
    );
    let lock = kb_lock_for(&kb_id);
    let _guard = lock.lock().await;
    let lib = super::get_library(&app, &kb_id)?;
    let docs = super::load_docs(&app, &kb_id)?;
    let prior = docs
        .iter()
        .find(|doc| doc.source_url.as_deref() == Some(&url));
    if doc_id
        .as_ref()
        .is_some_and(|id| prior.is_none_or(|doc| &doc.id != id))
    {
        return Err("The web source changed. Refresh the list and try again.".into());
    }

    let hash = sha256_hex(text.as_bytes());
    if let Some(doc) = prior.filter(|doc| doc.hash == hash) {
        if doc.status == DocStatus::Error {
            index_document(&app, &lib, doc).await?;
            return super::load_docs(&app, &kb_id)?
                .into_iter()
                .find(|indexed| indexed.id == doc.id)
                .ok_or_else(|| "Document not found".into());
        }
        return Ok(doc.clone());
    }

    let mut doc = KnowledgeDocument {
        id: super::gen_id("doc"),
        name: title,
        size_bytes: text.len() as u64,
        hash,
        chunk_count: 0,
        status: DocStatus::Indexing,
        error: None,
        created_at: chrono::Local::now().timestamp(),
        source_url: Some(url),
    };
    let dest =
        sources_dir(&app, &kb_id)?.join(format!("{}__{}.md", doc.id, sanitize_filename(&doc.name)));
    fs::write(&dest, text.as_bytes()).map_err(|e| format!("write source snapshot: {e}"))?;

    if let Some(prior) = prior {
        let result = async {
            let chunks = build_doc_index(&app, &lib, &doc).await?;
            doc.status = DocStatus::Ready;
            doc.chunk_count = chunks.len();
            let root = super::kb_root(&app)?;
            super::store::commit_web_revision(
                &super::open_library_index(&root, &lib)?,
                &prior.id,
                &doc,
                &chunks,
            )
        }
        .await;
        if let Err(error) = result {
            let _ = fs::remove_file(dest);
            emit_index(&app, &kb_id, &doc.id, "error", 0, 0, Some(error.clone()));
            return Err(error);
        }
        emit_index(
            &app,
            &kb_id,
            &doc.id,
            "ready",
            doc.chunk_count,
            doc.chunk_count,
            None,
        );
        refresh_library_counts(&app, &kb_id)?;
    } else {
        if let Err(error) = super::insert_doc(&app, &kb_id, &doc) {
            let _ = fs::remove_file(dest);
            return Err(error);
        }
        let _ = refresh_library_counts(&app, &kb_id);
        tauri::async_runtime::spawn(run_index(app, kb_id, doc.id.clone()));
    }
    Ok(doc)
}

async fn fetch_web_text(app: &AppHandle, url: &str) -> Result<String, String> {
    // Fetch + read body (drop the state guard before parsing).
    let (body, is_html) = {
        let state = app.state::<AppState>();
        let resp = crate::api::with_standard_request_timeout(
            state
                .http
                .get(url)
                .header("User-Agent", "Mozilla/5.0 (compatible; KivioBot/1.0)"),
        )
        .send()
        .await
        .map_err(|e| format!("fetch {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("fetch {url}: {e}"))?;
        let is_html = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_ascii_lowercase().contains("html"))
            .unwrap_or(false);
        if resp
            .content_length()
            .is_some_and(|size| size > parse::MAX_DOC_BYTES)
        {
            return Err("Web source exceeds the 20 MiB limit.".into());
        }
        let body = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if body.len() as u64 > parse::MAX_DOC_BYTES {
            return Err("Web source exceeds the 20 MiB limit.".into());
        }
        (body, is_html)
    };

    let text = if is_html || body.trim_start().starts_with('<') {
        crate::native_tools::html_to_text(&body)
    } else {
        body
    };
    if text.trim().is_empty() {
        return Err("No extractable text at that URL".to_string());
    }

    Ok(text)
}

fn truncate_name(s: &str) -> String {
    s.trim().chars().take(120).collect()
}

/// Rebuild each document independently. Its last successful index remains
/// searchable until its replacement commits.
#[tauri::command]
pub(crate) async fn kb_reindex_library(
    app: AppHandle,
    kb_id: String,
    doc_id: Option<String>,
) -> Result<(), String> {
    super::heal_stale_indexing_once(&app);
    let lock = kb_lock_for(&kb_id);
    let _guard = lock.lock().await;
    let lib = super::get_library(&app, &kb_id)?;
    let docs = super::load_docs(&app, &kb_id)?;
    let mut failures = Vec::new();
    let mut matched = false;
    for doc in docs
        .into_iter()
        .filter(|doc| doc_id.as_ref().is_none_or(|id| id == &doc.id))
    {
        matched = true;
        if let Err(error) = index_document(&app, &lib, &doc).await {
            failures.push(format!("{}: {error}", doc.name));
        }
    }
    if !matched && doc_id.is_some() {
        return Err("Document not found".to_string());
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Build a complete index alongside the published one, then publish its model
/// and database together by switching the library's atomically written pointer.
#[tauri::command]
pub(crate) async fn kb_update_embedding(
    app: AppHandle,
    kb_id: String,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let provider_id = provider_id.trim().to_string();
    let model = model.trim().to_string();
    if provider_id.is_empty() || model.is_empty() {
        return Err("Embedding provider and model are required".to_string());
    }
    if app
        .state::<AppState>()
        .settings_read()
        .get_provider(&provider_id)
        .is_none()
    {
        return Err("Embedding provider is unavailable".to_string());
    }
    super::heal_stale_indexing_once(&app);
    let lock = kb_lock_for(&kb_id);
    let _guard = lock.lock().await;
    let root = super::kb_root(&app)?;
    let mut next = super::get_library_at(&root, &kb_id)?;
    if next.embedding_provider_id == provider_id && next.embedding_model == model {
        return Ok(());
    }
    let (docs, web_versions) = {
        let conn = super::open_library_index(&root, &next)?;
        let docs = super::store::load_docs(&conn)?;
        let mut versions = Vec::new();
        for doc in &docs {
            if let Some(url) = &doc.source_url {
                for id in super::store::web_snapshots(&conn, url)? {
                    if id != doc.id {
                        versions.push((id, url.clone()));
                    }
                }
            }
        }
        (docs, versions)
    };
    next.embedding_provider_id = provider_id;
    next.embedding_model = model;
    next.embedding_dim = 0;
    next.index_generation = Some(uuid::Uuid::new_v4());
    let path = super::kb_db_path(&root, &next)?;
    let result = async {
        // Reopen the staged database for short writes so no SQLite handle or
        // transaction spans a provider request.
        {
            let conn = super::store::open_db(&path)?;
            for (id, url) in web_versions {
                conn.execute(
                    "INSERT INTO web_sources VALUES (?1, ?2)",
                    rusqlite::params![id, url],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        for doc in &docs {
            let chunks = build_doc_index(&app, &next, doc)
                .await
                .map_err(|error| format!("{}: {error}", doc.name))?;
            let dim = chunks[0].embedding.len();
            let conn = super::store::open_db(&path)?;
            super::store::insert_doc(&conn, doc)?;
            super::store::commit_doc_index(&conn, &doc.id, dim, &chunks)?;
            next.embedding_dim = dim;
        }
        super::publish_library_index(&root, &next)
    }
    .await;
    if result.is_err() {
        remove_index_files(&path);
    }
    let error = result.as_ref().err().cloned();
    emit_index(
        &app,
        &kb_id,
        "",
        if error.is_some() { "error" } else { "ready" },
        0,
        0,
        error,
    );
    result
}

pub(super) fn remove_index_files(path: &std::path::Path) {
    for file in [
        path.to_owned(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ] {
        if let Err(error) = fs::remove_file(&file) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Remove index file {}: {error}", file.display());
            }
        }
    }
}

/// Set a library's embedding-request batch size (0 = use the built-in default).
/// Only affects future indexing requests — no reindex, existing vectors untouched.
#[tauri::command]
pub(crate) async fn kb_set_embed_batch_size(
    app: AppHandle,
    kb_id: String,
    size: usize,
) -> Result<(), String> {
    {
        let _write = super::LIBRARY_WRITES
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut libs = super::load_libraries(&app)?;
        let lib = libs
            .iter_mut()
            .find(|l| l.id == kb_id)
            .ok_or_else(|| format!("Knowledge base not found: {kb_id}"))?;
        // 0 = default; otherwise clamp to the same 1..=128 the ingest path enforces.
        lib.embed_batch_size = if size == 0 { 0 } else { size.clamp(1, 128) };
        lib.updated_at = chrono::Local::now().timestamp();
        super::save_libraries(&app, &libs)?;
    }
    Ok(())
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

    #[test]
    fn kb_lock_registry_reuses_live_lock_and_reclaims_dead_lock() {
        let kb_id = format!("kb-lock-test-{}", uuid::Uuid::new_v4());
        let first = kb_lock_for(&kb_id);
        let first_weak = std::sync::Arc::downgrade(&first);
        let second = kb_lock_for(&kb_id);
        assert!(std::sync::Arc::ptr_eq(&first, &second));

        drop(first);
        drop(second);
        assert!(first_weak.upgrade().is_none());
        prune_kb_locks();
        assert!(!kb_lock_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&kb_id));
    }

    #[test]
    fn concurrent_kb_lock_lookup_returns_one_live_lock() {
        let kb_id = format!("kb-lock-concurrent-{}", uuid::Uuid::new_v4());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let kb_id = kb_id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    kb_lock_for(&kb_id)
                })
            })
            .collect();
        let locks: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert!(locks
            .iter()
            .skip(1)
            .all(|lock| std::sync::Arc::ptr_eq(&locks[0], lock)));
        drop(locks);
        prune_kb_locks();
    }

    #[tokio::test]
    async fn different_kb_locks_do_not_block_each_other() {
        let first = kb_lock_for(&format!("kb-lock-a-{}", uuid::Uuid::new_v4()));
        let second = kb_lock_for(&format!("kb-lock-b-{}", uuid::Uuid::new_v4()));
        let _first_guard = first.lock().await;

        let second_guard =
            tokio::time::timeout(std::time::Duration::from_millis(100), second.lock())
                .await
                .expect("different knowledge bases should lock independently");
        drop(second_guard);
    }
}
