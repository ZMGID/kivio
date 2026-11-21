//! Knowledge base (RAG) — storage layer + vector search.
//!
//! MVP design:
//! - Multiple libraries, each bound to one `(embedding_provider, model, dim)`.
//! - Vectors stored as plain `f32` in a per-library JSON file; search is an
//!   exact brute-force cosine scan in Rust.
//!   ponytail: brute-force cosine over a loaded JSON file. Fine for a desktop
//!   KB (thousands–tens-of-thousands of chunks). Swap to sqlite-vec / LanceDB
//!   behind a trait if a library ever grows past ~1e5 chunks.
//! - Parsing / chunking / embedding live in `ingest.rs` (PR2); this file owns
//!   the on-disk layout, CRUD, and retrieval math only.
//!
//! Layout: `{app_data}/knowledge_base/`
//! ```text
//! libraries.json            # Vec<KnowledgeLibrary>
//! <kb_id>/docs.json         # Vec<KnowledgeDocument>
//! <kb_id>/chunks.json       # Vec<KnowledgeChunk> (text + metadata + embedding)
//! <kb_id>/sources/<file>    # original file snapshots
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::storage::atomic_write;

pub mod chunking;
pub mod commands;
pub mod embeddings;
pub mod ingest;
pub mod parse;
pub mod process;
pub mod rerank;
pub mod retrieval;
pub mod store;

#[cfg(test)]
mod eval_tests;
#[cfg(test)]
mod live_e2e_tests;

// Catalog read-modify-write operations are serialized independently of per-library indexing.
// Listing reads an atomically published snapshot and never waits for this writer lock.
static LIBRARY_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A knowledge library. `embedding_dim` is 0 until the first chunk is indexed
/// (the dimension is learned from the first embedding response).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeLibrary {
    pub id: String,
    pub name: String,
    pub embedding_provider_id: String,
    pub embedding_model: String,
    #[serde(default)]
    pub embedding_dim: usize,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub doc_count: usize,
    #[serde(default)]
    pub chunk_count: usize,
    /// 每次 embedding 请求打包的片段数。0 = 用内置默认（`ingest::DEFAULT_EMBED_BATCH`）。
    /// 只影响后续索引的请求分批，不改变已存向量，改它无需重建。
    #[serde(default)]
    pub embed_batch_size: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index_generation: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocStatus {
    Indexing,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocument {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub chunk_count: usize,
    pub status: DocStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

/// One indexed chunk. `embedding` is the dense vector; everything else is
/// citation metadata so a retrieval hit can point back to the source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeChunk {
    pub id: String,
    pub doc_id: String,
    pub doc_name: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(default)]
    pub char_start: usize,
    #[serde(default)]
    pub char_end: usize,
    #[serde(default)]
    pub order_index: usize,
    pub embedding: Vec<f32>,
}

/// A retrieval hit: the chunk plus its cosine score and which library it came
/// from (chunks from multiple libraries can be merged in one search).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredChunk {
    pub kb_id: String,
    pub score: f32,
    #[serde(flatten)]
    pub chunk: KnowledgeChunk,
}

// ===== id generation (dependency-free) =====

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn gen_id(prefix: &str) -> String {
    let millis = chrono::Local::now().timestamp_millis();
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    // Separator between timestamp and counter so distinct (millis, n) pairs
    // can't collapse to the same hex string.
    format!("{prefix}_{millis:x}_{n:x}")
}

fn validate_kb_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("kb_")
        && id.len() > 3
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid knowledge base id: {id}"))
    }
}

// ===== paths =====

pub fn kb_root(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?;
    let dir = base.join("knowledge_base");
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create knowledge_base dir: {e}"))?;
    }
    Ok(dir)
}

pub fn sources_dir(app: &AppHandle, kb_id: &str) -> Result<PathBuf, String> {
    sources_dir_at(&kb_root(app)?, kb_id)
}

// ===== root-injectable cores (testable without an AppHandle) =====
//
// Every disk op funnels through a `*_at(root, …)` core so the integration
// tests can run the full create → index → search → delete cycle against a temp
// directory. The public `app`-taking functions just resolve `kb_root(app)` and
// delegate.

fn kb_dir_at(root: &Path, kb_id: &str) -> Result<PathBuf, String> {
    validate_kb_id(kb_id)?;
    Ok(root.join(kb_id))
}

fn sources_dir_at(root: &Path, kb_id: &str) -> Result<PathBuf, String> {
    let dir = kb_dir_at(root, kb_id)?.join("sources");
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create sources dir: {e}"))?;
    }
    Ok(dir)
}

fn load_libraries_at(root: &Path) -> Result<Vec<KnowledgeLibrary>, String> {
    let path = root.join("libraries.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read libraries.json: {e}"))?;
    match serde_json::from_str(&content) {
        Ok(libs) => Ok(libs),
        Err(e) => {
            // libraries.json is the root of every KB operation. If it's
            // corrupt (manual edit / FS damage), don't brick the whole panel:
            // back the bad file up and start empty so the user can rebuild.
            eprintln!(
                "libraries.json corrupt ({e}); backing up to libraries.json.bak and starting empty"
            );
            let _ = fs::rename(&path, root.join("libraries.json.bak"));
            Ok(Vec::new())
        }
    }
}

fn save_libraries_at(root: &Path, libs: &[KnowledgeLibrary]) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(libs).map_err(|e| format!("serialize libraries: {e}"))?;
    atomic_write(&root.join("libraries.json"), &content, "libraries")
}

fn get_library_at(root: &Path, kb_id: &str) -> Result<KnowledgeLibrary, String> {
    load_libraries_at(root)?
        .into_iter()
        .find(|l| l.id == kb_id)
        .ok_or_else(|| format!("Knowledge base not found: {kb_id}"))
}

fn create_library_at(
    root: &Path,
    name: &str,
    provider_id: &str,
    model: &str,
) -> Result<KnowledgeLibrary, String> {
    let _write = LIBRARY_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let name = name.trim();
    if name.is_empty() {
        return Err("Knowledge base name is empty".to_string());
    }
    if provider_id.trim().is_empty() || model.trim().is_empty() {
        return Err("Embedding provider and model are required".to_string());
    }
    let now = chrono::Local::now().timestamp();
    let lib = KnowledgeLibrary {
        id: gen_id("kb"),
        name: name.to_string(),
        embedding_provider_id: provider_id.to_string(),
        embedding_model: model.to_string(),
        embedding_dim: 0,
        created_at: now,
        updated_at: now,
        doc_count: 0,
        chunk_count: 0,
        embed_batch_size: 0,
        index_generation: None,
    };
    let mut libs = load_libraries_at(root)?;
    libs.push(lib.clone());
    save_libraries_at(root, &libs)?;
    Ok(lib)
}

fn rename_library_at(root: &Path, kb_id: &str, name: &str) -> Result<(), String> {
    let _write = LIBRARY_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let name = name.trim();
    if name.is_empty() {
        return Err("Knowledge base name is empty".to_string());
    }
    let mut libs = load_libraries_at(root)?;
    let lib = libs
        .iter_mut()
        .find(|l| l.id == kb_id)
        .ok_or_else(|| format!("Knowledge base not found: {kb_id}"))?;
    lib.name = name.to_string();
    lib.updated_at = chrono::Local::now().timestamp();
    save_libraries_at(root, &libs)
}

fn delete_library_at(root: &Path, kb_id: &str) -> Result<(), String> {
    let _write = LIBRARY_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let mut libs = load_libraries_at(root)?;
    let before = libs.len();
    libs.retain(|l| l.id != kb_id);
    if libs.len() == before {
        return Err(format!("Knowledge base not found: {kb_id}"));
    }
    save_libraries_at(root, &libs)?;
    if let Ok(dir) = kb_dir_at(root, kb_id) {
        let _ = fs::remove_dir_all(dir);
    }
    Ok(())
}

fn refresh_library_counts_at(root: &Path, kb_id: &str) -> Result<(), String> {
    let _write = LIBRARY_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let (ndocs, nchunks) = store::counts(&open_kb_at(root, kb_id)?)?;
    let mut libs = load_libraries_at(root)?;
    if let Some(lib) = libs.iter_mut().find(|l| l.id == kb_id) {
        lib.doc_count = ndocs;
        lib.chunk_count = nchunks;
        lib.updated_at = chrono::Local::now().timestamp();
        save_libraries_at(root, &libs)?;
    }
    Ok(())
}

/// Persist the learned embedding dimension on the library record (set on first
/// successful index; dimension is fixed per library).
fn set_library_dim_at(root: &Path, kb_id: &str, dim: usize) -> Result<(), String> {
    let _write = LIBRARY_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    if dim == 0 {
        return Ok(());
    }
    let mut libs = load_libraries_at(root)?;
    if let Some(lib) = libs.iter_mut().find(|l| l.id == kb_id) {
        if lib.embedding_dim != dim {
            lib.embedding_dim = dim;
            lib.updated_at = chrono::Local::now().timestamp();
            save_libraries_at(root, &libs)?;
        }
    }
    Ok(())
}

// ===== per-kb SQLite store access =====

fn kb_db_path(root: &Path, lib: &KnowledgeLibrary) -> Result<PathBuf, String> {
    let name = lib.index_generation.map_or_else(
        || "store.db".to_string(),
        |generation| format!("store-{generation}.db"),
    );
    Ok(kb_dir_at(root, &lib.id)?.join(name))
}

fn open_kb_at(root: &Path, kb_id: &str) -> Result<rusqlite::Connection, String> {
    let lib = get_library_at(root, kb_id)?;
    open_library_index(root, &lib)
}

fn open_library_index(root: &Path, lib: &KnowledgeLibrary) -> Result<rusqlite::Connection, String> {
    let path = kb_db_path(root, lib)?;
    if lib.index_generation.is_some() && !path.is_file() {
        return Err(format!(
            "Published index is missing for knowledge base {}",
            lib.id
        ));
    }
    let conn = store::open_db(&path)?;
    if lib.index_generation.is_none() {
        migrate_json_if_needed(root, &lib.id, &conn)?;
    }
    Ok(conn)
}

/// Publish the model and the matching index by committing the library record.
fn publish_library_index(root: &Path, next: &KnowledgeLibrary) -> Result<(), String> {
    let staged = open_library_index(root, next)?;
    let _write = LIBRARY_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let mut libs = load_libraries_at(root)?;
    let published = libs
        .iter_mut()
        .find(|lib| lib.id == next.id)
        .ok_or_else(|| format!("Knowledge base not found: {}", next.id))?;
    // Uploads can register new source snapshots during the rebuild. Carry
    // their pending records over; the queued workers read the published model.
    let staged_ids: std::collections::HashSet<_> = store::load_docs(&staged)?
        .into_iter()
        .map(|doc| doc.id)
        .collect();
    let tx = staged.unchecked_transaction().map_err(|e| e.to_string())?;
    for doc in store::load_docs(&open_library_index(root, published)?)? {
        if !staged_ids.contains(&doc.id) {
            if doc.status != DocStatus::Indexing || doc.chunk_count != 0 {
                return Err(format!("Staged index is missing {}", doc.name));
            }
            store::insert_doc(&tx, &doc)?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    let (doc_count, chunk_count) = store::counts(&staged)?;
    drop(staged);
    let previous_path = kb_db_path(root, published)?;
    published
        .embedding_provider_id
        .clone_from(&next.embedding_provider_id);
    published.embedding_model.clone_from(&next.embedding_model);
    published.embedding_dim = next.embedding_dim;
    published.index_generation = next.index_generation;
    published.doc_count = doc_count;
    published.chunk_count = chunk_count;
    published.updated_at = chrono::Local::now().timestamp();
    save_libraries_at(root, &libs)?;
    ingest::remove_index_files(&previous_path);
    Ok(())
}

/// Import V1 as one transaction. A durable marker also covers interruption
/// between the database commit and archiving the original JSON files.
fn migrate_json_if_needed(
    root: &Path,
    kb_id: &str,
    conn: &rusqlite::Connection,
) -> Result<(), String> {
    let dir = kb_dir_at(root, kb_id)?;
    let files = [dir.join("docs.json"), dir.join("chunks.json")];
    if !files.iter().any(|path| path.exists()) {
        return Ok(());
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS kb_migrations (name TEXT PRIMARY KEY)")
        .map_err(|e| e.to_string())?;
    let completed: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM kb_migrations WHERE name='legacy_json')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !completed {
        fn read_legacy<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
            let archived = path.with_extension("json.migrated");
            let path = if path.exists() { path } else { &archived };
            match fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|e| format!("Migrate {}: {e}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(e) => Err(format!("Read {} for migration: {e}", path.display())),
            }
        }
        let docs: Vec<KnowledgeDocument> = read_legacy(&files[0])?;
        let chunks: Vec<KnowledgeChunk> = read_legacy(&files[1])?;
        let dim = chunks.first().map_or(0, |c| c.embedding.len());
        if chunks.iter().any(|chunk| chunk.embedding.len() != dim) {
            return Err(format!(
                "Legacy knowledge base {kb_id} has inconsistent embedding dimensions"
            ));
        }
        let existing: std::collections::HashSet<_> = store::load_docs(&tx)?
            .into_iter()
            .map(|doc| doc.id)
            .collect();
        for doc in &docs {
            if !existing.contains(&doc.id) {
                store::insert_doc(&tx, doc)?;
            }
        }
        let mut by_doc = std::collections::BTreeMap::<String, Vec<KnowledgeChunk>>::new();
        for chunk in chunks {
            by_doc.entry(chunk.doc_id.clone()).or_default().push(chunk);
        }
        for (doc_id, chunks) in by_doc {
            let already_indexed: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM chunks WHERE doc_id=?1)",
                    [&doc_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !already_indexed {
                store::replace_doc_chunks_in_transaction(&tx, &doc_id, dim, &chunks)?;
            }
        }
        for doc in docs.iter().filter(|doc| doc.chunk_count > 0) {
            let found: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM chunks WHERE doc_id=?1)",
                    [&doc.id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !found {
                return Err(format!(
                    "Legacy document {} is missing its indexed chunks",
                    doc.name
                ));
            }
        }
        tx.execute("INSERT INTO kb_migrations VALUES ('legacy_json')", [])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    for path in files {
        if let Err(error) = fs::rename(&path, path.with_extension("json.migrated")) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Archive migrated {}: {error}", path.display());
            }
        }
    }
    Ok(())
}

fn load_docs_at(root: &Path, kb_id: &str) -> Result<Vec<KnowledgeDocument>, String> {
    store::load_docs(&open_kb_at(root, kb_id)?)
}

fn insert_doc_at(root: &Path, kb_id: &str, doc: &KnowledgeDocument) -> Result<(), String> {
    let conn = open_kb_at(root, kb_id)?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    store::insert_doc(&tx, doc)?;
    tx.commit().map_err(|e| e.to_string())
}

fn doc_by_hash_at(
    root: &Path,
    kb_id: &str,
    hash: &str,
) -> Result<Option<KnowledgeDocument>, String> {
    store::doc_by_hash(&open_kb_at(root, kb_id)?, hash)
}

fn set_doc_status_at(
    root: &Path,
    kb_id: &str,
    doc_id: &str,
    status: DocStatus,
    count: usize,
    error: Option<&str>,
) -> Result<(), String> {
    store::set_doc_status(&open_kb_at(root, kb_id)?, doc_id, status, count, error)
}

#[cfg(test)]
fn replace_doc_chunks_at(
    root: &Path,
    kb_id: &str,
    doc_id: &str,
    dim: usize,
    chunks: &[KnowledgeChunk],
) -> Result<(), String> {
    store::replace_doc_chunks(&open_kb_at(root, kb_id)?, doc_id, dim, chunks)
}

fn delete_document_at(root: &Path, kb_id: &str, doc_id: &str) -> Result<(), String> {
    let snapshots = store::delete_doc(&open_kb_at(root, kb_id)?, doc_id)?;
    // remove source snapshot(s) for this doc
    if let Ok(dir) = sources_dir_at(root, kb_id) {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name
                    .split_once("__")
                    .is_some_and(|(id, _)| snapshots.iter().any(|snapshot| snapshot == id))
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
    refresh_library_counts_at(root, kb_id)
}

/// Cross-library hybrid (vector + keyword RRF) search, top-k best-first.
/// Test-only: production retrieval goes through `retrieval::retrieve`; this
/// remains as the unit under test for the multi-library fusion/sort/truncate path.
#[cfg(test)]
fn search_at(
    root: &Path,
    kb_ids: &[String],
    query: &[f32],
    query_text: &str,
    top_k: usize,
    weight_vector: f32,
    weight_keyword: f32,
) -> Result<Vec<ScoredChunk>, String> {
    let mut all: Vec<ScoredChunk> = Vec::new();
    for kb_id in kb_ids {
        // Tolerate a single broken/corrupt library: skip (logged) so it can't
        // starve the rest of a cross-library search.
        match open_kb_at(root, kb_id).and_then(|c| {
            store::hybrid_search(&c, query, query_text, top_k, weight_vector, weight_keyword)
        }) {
            Ok(hits) => {
                for (chunk, score) in hits {
                    all.push(ScoredChunk {
                        kb_id: kb_id.clone(),
                        score,
                        chunk,
                    });
                }
            }
            Err(e) => eprintln!("kb search: skipping library {kb_id}: {e}"),
        }
    }
    all.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all.truncate(top_k);
    Ok(all)
}

// ===== public app wrappers =====

pub fn load_libraries(app: &AppHandle) -> Result<Vec<KnowledgeLibrary>, String> {
    load_libraries_at(&kb_root(app)?)
}

fn save_libraries(app: &AppHandle, libs: &[KnowledgeLibrary]) -> Result<(), String> {
    save_libraries_at(&kb_root(app)?, libs)
}

pub fn get_library(app: &AppHandle, kb_id: &str) -> Result<KnowledgeLibrary, String> {
    validate_kb_id(kb_id)?;
    get_library_at(&kb_root(app)?, kb_id)
}

pub fn create_library(
    app: &AppHandle,
    name: &str,
    provider_id: &str,
    model: &str,
) -> Result<KnowledgeLibrary, String> {
    create_library_at(&kb_root(app)?, name, provider_id, model)
}

pub fn rename_library(app: &AppHandle, kb_id: &str, name: &str) -> Result<(), String> {
    validate_kb_id(kb_id)?;
    rename_library_at(&kb_root(app)?, kb_id, name)
}

pub fn delete_library(app: &AppHandle, kb_id: &str) -> Result<(), String> {
    validate_kb_id(kb_id)?;
    delete_library_at(&kb_root(app)?, kb_id)
}

/// Recompute and persist `doc_count` / `chunk_count` / `embedding_dim` on the
/// library record from its docs + chunks. Called after any ingest/delete.
pub fn refresh_library_counts(app: &AppHandle, kb_id: &str) -> Result<(), String> {
    refresh_library_counts_at(&kb_root(app)?, kb_id)
}

/// Persist the learned embedding dimension (set on first successful index).
pub fn set_library_dim(app: &AppHandle, kb_id: &str, dim: usize) -> Result<(), String> {
    set_library_dim_at(&kb_root(app)?, kb_id, dim)
}

pub fn load_docs(app: &AppHandle, kb_id: &str) -> Result<Vec<KnowledgeDocument>, String> {
    load_docs_at(&kb_root(app)?, kb_id)
}

pub fn insert_doc(app: &AppHandle, kb_id: &str, doc: &KnowledgeDocument) -> Result<(), String> {
    insert_doc_at(&kb_root(app)?, kb_id, doc)
}

pub fn doc_by_hash(
    app: &AppHandle,
    kb_id: &str,
    hash: &str,
) -> Result<Option<KnowledgeDocument>, String> {
    doc_by_hash_at(&kb_root(app)?, kb_id, hash)
}

pub fn set_doc_status(
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    status: DocStatus,
    count: usize,
    error: Option<&str>,
) -> Result<(), String> {
    set_doc_status_at(&kb_root(app)?, kb_id, doc_id, status, count, error)
}

/// Remove a document and all its chunks + source snapshot, then refresh counts.
pub fn delete_document(app: &AppHandle, kb_id: &str, doc_id: &str) -> Result<(), String> {
    delete_document_at(&kb_root(app)?, kb_id, doc_id)
}

fn is_index_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "store.db"
        || name
            .strip_prefix("store-")
            .and_then(|name| name.strip_suffix(".db"))
            .is_some_and(|generation| uuid::Uuid::parse_str(generation).is_ok())
}

/// Startup self-heal: any document left in `indexing` is stale (its indexing
/// task died with the app / crashed / was killed mid-embed). Flip it to `error`
/// so the UI stops spinning forever and the user can retry. Returns how many
/// were healed.
fn heal_stale_indexing_at(root: &Path) -> usize {
    let mut healed = 0usize;
    for lib in load_libraries_at(root).unwrap_or_default() {
        let Ok(conn) = open_kb_at(root, &lib.id) else {
            continue;
        };
        if let Ok(active_path) = kb_db_path(root, &lib) {
            if let Some(index_dir) = active_path.parent() {
                if let Ok(entries) = fs::read_dir(index_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path != active_path && is_index_file(&path) {
                            ingest::remove_index_files(&path);
                        }
                    }
                }
            }
        }
        let Ok(docs) = store::load_docs(&conn) else {
            continue;
        };
        let mut changed = false;
        for doc in &docs {
            if doc.status == DocStatus::Indexing {
                let _ = store::set_doc_status(
                    &conn,
                    &doc.id,
                    DocStatus::Error,
                    doc.chunk_count,
                    Some("索引被中断，请重试 / Indexing was interrupted; please retry"),
                );
                changed = true;
                healed += 1;
            }
        }
        drop(conn);
        if changed {
            let _ = refresh_library_counts_at(root, &lib.id);
        }
    }
    healed
}

/// Call once at app startup (no indexing in flight) to clear stale `indexing`
/// statuses left by a previous run that didn't finish.
pub fn heal_stale_indexing(app: &AppHandle) {
    if let Ok(root) = kb_root(app) {
        let n = heal_stale_indexing_at(&root);
        if n > 0 {
            eprintln!("knowledge_base: healed {n} stale 'indexing' document(s) → error");
        }
    }
}

/// Recover once before the first listing or index mutation in this process.
pub fn heal_stale_indexing_once(app: &AppHandle) {
    static HEAL_ONCE: std::sync::Once = std::sync::Once::new();
    HEAL_ONCE.call_once(|| heal_stale_indexing(app));
}

/// System-prompt segment injected when the conversation has knowledge bases
/// attached. Tells the model the user's docs are already indexed (don't ask for
/// re-upload), to prefer `knowledge_search`, and to cite passages inline as
/// the exact `[kb:…]` marker. When `force` is set, the "prefer" nudge becomes a hard "must search
/// first" directive. Returns None when nothing resolvable is attached.
pub fn mount_system_prompt(app: &AppHandle, kb_ids: &[String], force: bool) -> Option<String> {
    if kb_ids.is_empty() {
        return None;
    }
    let libs = load_libraries(app).unwrap_or_default();
    let names: Vec<String> = kb_ids
        .iter()
        .filter_map(|id| libs.iter().find(|l| &l.id == id).map(|l| l.name.clone()))
        .collect();
    if names.is_empty() {
        return None;
    }
    let names_str = names.join(", ");
    Some(if force {
        format!(
            "This conversation has knowledge bases attached: {names_str}. **Forced retrieval is ON: you MUST call knowledge_search before every answer — do not skip it**, even if you think you already know the answer; search first to confirm. The documents are already indexed, so do not ask the user to re-upload files. When you use a retrieved passage, copy its exact [kb:…] citation marker inline; only if nothing relevant is found, say the knowledge base doesn't cover it."
        )
    } else {
        format!(
            "This conversation has knowledge bases attached: {names_str}. When the user's question may relate to these documents, prefer calling knowledge_search first — the documents are already indexed, so do not ask the user to re-upload files. When you use a retrieved passage, copy its exact [kb:…] citation marker inline so the user can trace it; only if nothing relevant is found, say the knowledge base doesn't cover it."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_library_creation_preserves_every_committed_library() {
        let dir = tempfile::tempdir().unwrap();
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for i in 0..8 {
                let root = dir.path();
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    for j in 0..4 {
                        create_library_at(root, &format!("{i}-{j}"), "provider", "model").unwrap();
                    }
                });
            }
        });
        assert_eq!(load_libraries_at(dir.path()).unwrap().len(), 32);
    }

    #[test]
    fn kb_id_validation() {
        assert!(validate_kb_id("kb_abc123").is_ok());
        assert!(validate_kb_id("kb_a-b_c").is_ok());
        assert!(validate_kb_id("conv_x").is_err());
        assert!(validate_kb_id("kb_../etc").is_err());
        assert!(validate_kb_id("kb_").is_err());
    }

    // ===== full storage + retrieval e2e (temp dir, no AppHandle / network) =====

    fn temp_root() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kivio-kb-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn doc(id: &str, name: &str, chunks: usize) -> KnowledgeDocument {
        KnowledgeDocument {
            id: id.to_string(),
            name: name.to_string(),
            size_bytes: 10,
            hash: format!("h-{id}"),
            chunk_count: chunks,
            status: DocStatus::Ready,
            error: None,
            created_at: 0,
            source_url: None,
        }
    }

    fn chunk_emb(
        id: &str,
        doc_id: &str,
        name: &str,
        heading: Option<&str>,
        emb: Vec<f32>,
    ) -> KnowledgeChunk {
        KnowledgeChunk {
            id: id.to_string(),
            doc_id: doc_id.to_string(),
            doc_name: name.to_string(),
            text: format!("text of {id}"),
            heading_path: heading.map(|s| s.to_string()),
            page: None,
            char_start: 0,
            char_end: 0,
            order_index: 0,
            embedding: emb,
        }
    }

    #[test]
    fn e2e_create_index_search_delete_cycle() {
        let root = temp_root();

        // 1) create a library
        let lib = create_library_at(&root, "Docs", "openai", "text-embedding-3-small").unwrap();
        let kb = lib.id.clone();
        assert_eq!(load_libraries_at(&root).unwrap().len(), 1);

        // 2) ingest output: two docs, three chunks with known vectors
        insert_doc_at(&root, &kb, &doc("doc_a", "a.md", 2)).unwrap();
        insert_doc_at(&root, &kb, &doc("doc_b", "b.md", 1)).unwrap();
        replace_doc_chunks_at(
            &root,
            &kb,
            "doc_a",
            3,
            &[
                chunk_emb(
                    "c1",
                    "doc_a",
                    "a.md",
                    Some("Intro > Setup"),
                    vec![1.0, 0.0, 0.0],
                ),
                chunk_emb("c2", "doc_a", "a.md", None, vec![0.0, 1.0, 0.0]),
            ],
        )
        .unwrap();
        replace_doc_chunks_at(
            &root,
            &kb,
            "doc_b",
            3,
            &[chunk_emb("c3", "doc_b", "b.md", None, vec![0.9, 0.1, 0.0])],
        )
        .unwrap();
        set_library_dim_at(&root, &kb, 3).unwrap();

        // 3) counts + dimension
        refresh_library_counts_at(&root, &kb).unwrap();
        let lib = get_library_at(&root, &kb).unwrap();
        assert_eq!(lib.doc_count, 2);
        assert_eq!(lib.chunk_count, 3);
        assert_eq!(lib.embedding_dim, 3);

        // 4) search (vec0 cosine): query near [1,0,0] → c1 (exact) then c3 (close)
        let hits = search_at(&root, &[kb.clone()], &[1.0, 0.0, 0.0], "", 2, 1.0, 0.0).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk.id, "c1");
        assert_eq!(hits[0].chunk.doc_name, "a.md");
        assert_eq!(hits[0].chunk.heading_path.as_deref(), Some("Intro > Setup"));
        assert_eq!(hits[1].chunk.id, "c3");
        assert!(hits[0].score >= hits[1].score);

        // 5) delete doc_a → its chunks gone, doc_b intact, counts refreshed
        delete_document_at(&root, &kb, "doc_a").unwrap();
        let remaining = load_docs_at(&root, &kb).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "doc_b");
        let lib = get_library_at(&root, &kb).unwrap();
        assert_eq!(lib.doc_count, 1);
        assert_eq!(lib.chunk_count, 1);
        let hits = search_at(&root, &[kb.clone()], &[1.0, 0.0, 0.0], "", 5, 1.0, 0.0).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.doc_id, "doc_b");

        // 6) delete library removes its directory
        delete_library_at(&root, &kb).unwrap();
        assert!(load_libraries_at(&root).unwrap().is_empty());
        assert!(!root.join(&kb).exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn e2e_multi_library_search_merges_and_ranks() {
        let root = temp_root();
        let a = create_library_at(&root, "A", "openai", "m").unwrap().id;
        let b = create_library_at(&root, "B", "openai", "m").unwrap().id;
        insert_doc_at(&root, &a, &doc("d", "a.md", 0)).unwrap();
        insert_doc_at(&root, &b, &doc("d", "b.md", 0)).unwrap();
        replace_doc_chunks_at(
            &root,
            &a,
            "d",
            2,
            &[chunk_emb("a1", "d", "a.md", None, vec![1.0, 0.0])],
        )
        .unwrap();
        replace_doc_chunks_at(
            &root,
            &b,
            "d",
            2,
            &[chunk_emb("b1", "d", "b.md", None, vec![0.2, 1.0])],
        )
        .unwrap();

        // Query closest to a1; both libraries searched, hit tagged with its kb id.
        let hits = search_at(&root, &[a.clone(), b.clone()], &[1.0, 0.0], "", 5, 1.0, 0.0).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk.id, "a1");
        assert_eq!(hits[0].kb_id, a);
        assert_eq!(hits[1].kb_id, b);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn e2e_reindex_replaces_doc_chunks_without_dup() {
        // Models the index_one replace step: drop a doc's old chunks, add new.
        let root = temp_root();
        let kb = create_library_at(&root, "L", "openai", "m").unwrap().id;
        insert_doc_at(&root, &kb, &doc("doc_x", "x.md", 0)).unwrap();
        replace_doc_chunks_at(
            &root,
            &kb,
            "doc_x",
            1,
            &[chunk_emb("old1", "doc_x", "x.md", None, vec![1.0])],
        )
        .unwrap();
        replace_doc_chunks_at(
            &root,
            &kb,
            "doc_x",
            1,
            &[
                chunk_emb("new1", "doc_x", "x.md", None, vec![1.0]),
                chunk_emb("new2", "doc_x", "x.md", None, vec![1.0]),
            ],
        )
        .unwrap();

        let hits = search_at(&root, &[kb.clone()], &[1.0], "", 5, 1.0, 0.0).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|c| c.chunk.id.starts_with("new")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rename_and_missing_library_errors() {
        let root = temp_root();
        let kb = create_library_at(&root, "Old", "openai", "m").unwrap().id;
        rename_library_at(&root, &kb, "New").unwrap();
        assert_eq!(get_library_at(&root, &kb).unwrap().name, "New");
        assert!(rename_library_at(&root, "kb_missing", "X").is_err());
        assert!(delete_document_at(&root, &kb, "doc_missing").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gen_id_has_separator_between_time_and_counter() {
        // prefix + millis + counter → at least two '_' so segments can't merge.
        let id = gen_id("kb");
        assert!(id.starts_with("kb_"));
        assert_eq!(id.matches('_').count(), 2, "id was {id}");
    }

    #[test]
    fn heal_flips_stale_indexing_to_error() {
        let root = temp_root();
        let kb = create_library_at(&root, "L", "openai", "m").unwrap().id;
        insert_doc_at(&root, &kb, &{
            let mut d = doc("doc_stuck", "a.md", 0);
            d.status = DocStatus::Indexing;
            d
        })
        .unwrap();
        insert_doc_at(&root, &kb, &doc("doc_ok", "b.md", 3)).unwrap();

        let healed = heal_stale_indexing_at(&root);
        assert_eq!(healed, 1);
        let docs = load_docs_at(&root, &kb).unwrap();
        let stuck = docs.iter().find(|d| d.id == "doc_stuck").unwrap();
        assert_eq!(stuck.status, DocStatus::Error);
        assert!(stuck.error.is_some());
        // healthy doc untouched
        assert_eq!(
            docs.iter().find(|d| d.id == "doc_ok").unwrap().status,
            DocStatus::Ready
        );
        // idempotent: nothing left to heal
        assert_eq!(heal_stale_indexing_at(&root), 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn search_skips_corrupt_library_instead_of_failing() {
        let root = temp_root();
        let good = create_library_at(&root, "good", "openai", "m").unwrap().id;
        let bad = create_library_at(&root, "bad", "openai", "m").unwrap().id;
        insert_doc_at(&root, &good, &doc("d", "g.md", 0)).unwrap();
        replace_doc_chunks_at(
            &root,
            &good,
            "d",
            2,
            &[chunk_emb("g1", "d", "g.md", None, vec![1.0, 0.0])],
        )
        .unwrap();
        // Corrupt the bad library's store.db with non-SQLite garbage.
        let bad_db = kb_db_path(&root, &get_library_at(&root, &bad).unwrap()).unwrap();
        std::fs::create_dir_all(bad_db.parent().unwrap()).unwrap();
        std::fs::write(&bad_db, "not a sqlite database").unwrap();

        let hits = search_at(
            &root,
            &[good.clone(), bad.clone()],
            &[1.0, 0.0],
            "",
            5,
            1.0,
            0.0,
        )
        .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "healthy library's hit must survive a corrupt sibling"
        );
        assert_eq!(hits[0].chunk.id, "g1");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn publishing_new_model_switches_to_the_matching_index() {
        let root = tempfile::tempdir().unwrap();
        let mut next = create_library_at(root.path(), "Docs", "provider", "old").unwrap();
        let kb = next.id.clone();
        let source = doc("doc_a", "a.md", 1);
        insert_doc_at(root.path(), &kb, &source).unwrap();
        replace_doc_chunks_at(
            root.path(),
            &kb,
            &source.id,
            2,
            &[chunk_emb(
                "old",
                &source.id,
                &source.name,
                None,
                vec![1.0, 0.0],
            )],
        )
        .unwrap();

        next.embedding_model = "new".into();
        next.embedding_dim = 3;
        next.index_generation = Some(uuid::Uuid::new_v4());
        assert!(publish_library_index(root.path(), &next).is_err());
        assert_eq!(
            get_library_at(root.path(), &kb).unwrap().embedding_model,
            "old"
        );
        assert_eq!(
            search_at(root.path(), &[kb.clone()], &[1.0, 0.0], "", 1, 1.0, 0.0).unwrap()[0]
                .chunk
                .id,
            "old"
        );

        let staged = store::open_db(&kb_db_path(root.path(), &next).unwrap()).unwrap();
        store::insert_doc(&staged, &source).unwrap();
        store::commit_doc_index(
            &staged,
            &source.id,
            3,
            &[chunk_emb(
                "new",
                &source.id,
                &source.name,
                None,
                vec![0.0, 1.0, 0.0],
            )],
        )
        .unwrap();
        drop(staged);
        let pending = KnowledgeDocument {
            status: DocStatus::Indexing,
            ..doc("pending", "new.md", 0)
        };
        insert_doc_at(root.path(), &kb, &pending).unwrap();
        publish_library_index(root.path(), &next).unwrap();
        assert!(load_docs_at(root.path(), &kb)
            .unwrap()
            .iter()
            .any(|doc| { doc.id == pending.id && doc.status == DocStatus::Indexing }));
        let published = get_library_at(root.path(), &kb).unwrap();
        assert_eq!(
            (published.embedding_model.as_str(), published.embedding_dim),
            ("new", 3)
        );
        assert_eq!(
            search_at(root.path(), &[kb], &[0.0, 1.0, 0.0], "", 1, 1.0, 0.0).unwrap()[0]
                .chunk
                .id,
            "new"
        );
    }

    #[test]
    fn interrupted_legacy_migration_keeps_json_and_resumes_with_existing_rows() {
        let root = tempfile::tempdir().unwrap();
        let kb = create_library_at(root.path(), "Legacy", "provider", "model")
            .unwrap()
            .id;
        let directory = kb_dir_at(root.path(), &kb).unwrap();
        let first = doc("doc_a", "a.md", 1);
        let second = doc("doc_b", "b.md", 1);
        insert_doc_at(root.path(), &kb, &first).unwrap();
        let docs_file = directory.join("docs.json");
        let chunks_file = directory.join("chunks.json");
        let write = |path: &Path, value: &serde_json::Value| {
            fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
        };
        write(&docs_file, &serde_json::json!([first, second]));
        let first_chunk = chunk_emb("a", "doc_a", "a.md", None, vec![1.0, 0.0]);
        let second_chunk = chunk_emb("b", "doc_z_missing", "b.md", None, vec![0.0, 1.0]);
        write(
            &chunks_file,
            &serde_json::json!([first_chunk, second_chunk]),
        );
        assert!(load_docs_at(root.path(), &kb)
            .unwrap_err()
            .contains("doc_z_missing"));
        assert!(docs_file.is_file() && chunks_file.is_file());
        let db = store::open_db(&directory.join("store.db")).unwrap();
        assert_eq!(store::counts(&db).unwrap(), (1, 0));
        drop(db);

        let second_chunk = KnowledgeChunk {
            doc_id: "doc_b".into(),
            ..second_chunk
        };
        write(
            &chunks_file,
            &serde_json::json!([first_chunk, second_chunk]),
        );
        assert_eq!(load_docs_at(root.path(), &kb).unwrap().len(), 2);
        let hits = search_at(root.path(), &[kb], &[0.0, 1.0], "", 2, 1.0, 0.0).unwrap();
        assert_eq!(hits[0].chunk.doc_id, "doc_b");
        assert!(docs_file.with_extension("json.migrated").is_file());
        assert!(chunks_file.with_extension("json.migrated").is_file());
    }

    #[test]
    fn corrupt_libraries_json_degrades_to_empty_with_backup() {
        let root = temp_root();
        std::fs::write(root.join("libraries.json"), "{ broken").unwrap();
        // Must not error — returns empty so the panel still opens.
        assert!(load_libraries_at(&root).unwrap().is_empty());
        // Bad file is backed up, not silently destroyed.
        assert!(root.join("libraries.json.bak").exists());
        std::fs::remove_dir_all(&root).ok();
    }
}
