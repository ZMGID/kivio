//! Knowledge base (RAG) — storage layer + vector search.
//!
//! MVP design (see `.trellis/tasks/06-25-knowledge-base-rag/prd.md`):
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
    format!("{prefix}_{millis:x}{n:x}")
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
    serde_json::from_str(&content).map_err(|e| format!("libraries.json corrupt: {e}"))
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
    };
    let mut libs = load_libraries_at(root)?;
    libs.push(lib.clone());
    save_libraries_at(root, &libs)?;
    Ok(lib)
}

fn rename_library_at(root: &Path, kb_id: &str, name: &str) -> Result<(), String> {
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
    let docs = load_docs_at(root, kb_id)?;
    let chunks = load_chunks_at(root, kb_id)?;
    let dim = chunks.first().map(|c| c.embedding.len()).unwrap_or(0);
    let mut libs = load_libraries_at(root)?;
    if let Some(lib) = libs.iter_mut().find(|l| l.id == kb_id) {
        lib.doc_count = docs.len();
        lib.chunk_count = chunks.len();
        if dim > 0 {
            lib.embedding_dim = dim;
        }
        lib.updated_at = chrono::Local::now().timestamp();
        save_libraries_at(root, &libs)?;
    }
    Ok(())
}

fn load_docs_at(root: &Path, kb_id: &str) -> Result<Vec<KnowledgeDocument>, String> {
    let path = kb_dir_at(root, kb_id)?.join("docs.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read docs.json: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("docs.json corrupt: {e}"))
}

fn save_docs_at(root: &Path, kb_id: &str, docs: &[KnowledgeDocument]) -> Result<(), String> {
    let path = kb_dir_at(root, kb_id)?.join("docs.json");
    let content = serde_json::to_string_pretty(docs).map_err(|e| format!("serialize docs: {e}"))?;
    atomic_write(&path, &content, "docs")
}

fn load_chunks_at(root: &Path, kb_id: &str) -> Result<Vec<KnowledgeChunk>, String> {
    let path = kb_dir_at(root, kb_id)?.join("chunks.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read chunks.json: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("chunks.json corrupt: {e}"))
}

fn save_chunks_at(root: &Path, kb_id: &str, chunks: &[KnowledgeChunk]) -> Result<(), String> {
    let path = kb_dir_at(root, kb_id)?.join("chunks.json");
    let content = serde_json::to_string(chunks).map_err(|e| format!("serialize chunks: {e}"))?;
    atomic_write(&path, &content, "chunks")
}

fn delete_document_at(root: &Path, kb_id: &str, doc_id: &str) -> Result<(), String> {
    let mut docs = load_docs_at(root, kb_id)?;
    let before = docs.len();
    docs.retain(|d| d.id != doc_id);
    if docs.len() == before {
        return Err(format!("Document not found: {doc_id}"));
    }
    save_docs_at(root, kb_id, &docs)?;

    let mut chunks = load_chunks_at(root, kb_id)?;
    remove_doc_chunks(&mut chunks, doc_id);
    save_chunks_at(root, kb_id, &chunks)?;

    if let Ok(dir) = sources_dir_at(root, kb_id) {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{doc_id}__"))
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
    refresh_library_counts_at(root, kb_id)
}

fn search_at(
    root: &Path,
    kb_ids: &[String],
    query: &[f32],
    top_k: usize,
) -> Result<Vec<ScoredChunk>, String> {
    let mut candidates: Vec<(String, KnowledgeChunk)> = Vec::new();
    for kb_id in kb_ids {
        for chunk in load_chunks_at(root, kb_id)? {
            candidates.push((kb_id.clone(), chunk));
        }
    }
    Ok(top_k_by_cosine(candidates, query, top_k))
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

pub fn load_docs(app: &AppHandle, kb_id: &str) -> Result<Vec<KnowledgeDocument>, String> {
    load_docs_at(&kb_root(app)?, kb_id)
}

pub fn save_docs(app: &AppHandle, kb_id: &str, docs: &[KnowledgeDocument]) -> Result<(), String> {
    save_docs_at(&kb_root(app)?, kb_id, docs)
}

pub fn load_chunks(app: &AppHandle, kb_id: &str) -> Result<Vec<KnowledgeChunk>, String> {
    load_chunks_at(&kb_root(app)?, kb_id)
}

pub fn save_chunks(app: &AppHandle, kb_id: &str, chunks: &[KnowledgeChunk]) -> Result<(), String> {
    save_chunks_at(&kb_root(app)?, kb_id, chunks)
}

/// Remove a document and all its chunks + source snapshot, then refresh counts.
pub fn delete_document(app: &AppHandle, kb_id: &str, doc_id: &str) -> Result<(), String> {
    delete_document_at(&kb_root(app)?, kb_id, doc_id)
}


// ===== pure retrieval math (unit-tested without an AppHandle) =====

/// Cosine similarity. Returns 0.0 for mismatched-length or zero vectors so a
/// stray bad row can't poison a ranking with NaN.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Brute-force top-k by cosine over `(kb_id, chunk)` pairs. Sorted by score
/// descending; ties broken by insertion order (stable).
pub fn top_k_by_cosine(
    candidates: Vec<(String, KnowledgeChunk)>,
    query: &[f32],
    top_k: usize,
) -> Vec<ScoredChunk> {
    let mut scored: Vec<ScoredChunk> = candidates
        .into_iter()
        .map(|(kb_id, chunk)| {
            let score = cosine_similarity(query, &chunk.embedding);
            ScoredChunk {
                kb_id,
                score,
                chunk,
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_k);
    scored
}

fn remove_doc_chunks(chunks: &mut Vec<KnowledgeChunk>, doc_id: &str) {
    chunks.retain(|c| c.doc_id != doc_id);
}

/// Load chunks across the given libraries and return the top-k by cosine.
pub fn search(
    app: &AppHandle,
    kb_ids: &[String],
    query: &[f32],
    top_k: usize,
) -> Result<Vec<ScoredChunk>, String> {
    search_at(&kb_root(app)?, kb_ids, query, top_k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &str, doc_id: &str, emb: Vec<f32>) -> KnowledgeChunk {
        KnowledgeChunk {
            id: id.to_string(),
            doc_id: doc_id.to_string(),
            doc_name: "d.txt".to_string(),
            text: id.to_string(),
            heading_path: None,
            page: None,
            char_start: 0,
            char_end: 0,
            order_index: 0,
            embedding: emb,
        }
    }

    #[test]
    fn cosine_basics() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        // opposite vectors → -1
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        // mismatched length / zero vector → 0, never NaN
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn top_k_ranks_nearest_first_and_truncates() {
        let cands = vec![
            ("kb_a".to_string(), chunk("near", "d1", vec![0.9, 0.1])),
            ("kb_a".to_string(), chunk("far", "d1", vec![0.0, 1.0])),
            ("kb_b".to_string(), chunk("mid", "d2", vec![0.6, 0.6])),
        ];
        let out = top_k_by_cosine(cands, &[1.0, 0.0], 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].chunk.id, "near");
        assert_eq!(out[0].kb_id, "kb_a");
        assert_eq!(out[1].chunk.id, "mid");
        assert!(out[0].score >= out[1].score);
    }

    #[test]
    fn delete_removes_only_target_doc_chunks() {
        let mut chunks = vec![
            chunk("c1", "doc_keep", vec![1.0]),
            chunk("c2", "doc_drop", vec![1.0]),
            chunk("c3", "doc_keep", vec![1.0]),
            chunk("c4", "doc_drop", vec![1.0]),
        ];
        remove_doc_chunks(&mut chunks, "doc_drop");
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|c| c.doc_id == "doc_keep"));
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
        }
    }

    fn chunk_emb(id: &str, doc_id: &str, name: &str, heading: Option<&str>, emb: Vec<f32>) -> KnowledgeChunk {
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
        save_docs_at(&root, &kb, &[doc("doc_a", "a.md", 2), doc("doc_b", "b.md", 1)]).unwrap();
        save_chunks_at(
            &root,
            &kb,
            &[
                chunk_emb("c1", "doc_a", "a.md", Some("Intro > Setup"), vec![1.0, 0.0, 0.0]),
                chunk_emb("c2", "doc_a", "a.md", None, vec![0.0, 1.0, 0.0]),
                chunk_emb("c3", "doc_b", "b.md", None, vec![0.9, 0.1, 0.0]),
            ],
        )
        .unwrap();

        // 3) counts + dimension learned from chunks
        refresh_library_counts_at(&root, &kb).unwrap();
        let lib = get_library_at(&root, &kb).unwrap();
        assert_eq!(lib.doc_count, 2);
        assert_eq!(lib.chunk_count, 3);
        assert_eq!(lib.embedding_dim, 3);

        // 4) search: query near [1,0,0] → c1 (exact) then c3 (close); citation metadata intact
        let hits = search_at(&root, &[kb.clone()], &[1.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk.id, "c1");
        assert_eq!(hits[0].chunk.doc_name, "a.md");
        assert_eq!(hits[0].chunk.heading_path.as_deref(), Some("Intro > Setup"));
        assert_eq!(hits[1].chunk.id, "c3");
        assert!(hits[0].score >= hits[1].score);

        // 5) delete doc_a → its chunks gone, doc_b intact, counts refreshed
        delete_document_at(&root, &kb, "doc_a").unwrap();
        let remaining = load_chunks_at(&root, &kb).unwrap();
        assert!(remaining.iter().all(|c| c.doc_id == "doc_b"));
        let lib = get_library_at(&root, &kb).unwrap();
        assert_eq!(lib.doc_count, 1);
        assert_eq!(lib.chunk_count, 1);
        let hits = search_at(&root, &[kb.clone()], &[1.0, 0.0, 0.0], 5).unwrap();
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
        save_chunks_at(&root, &a, &[chunk_emb("a1", "d", "a.md", None, vec![1.0, 0.0])]).unwrap();
        save_chunks_at(&root, &b, &[chunk_emb("b1", "d", "b.md", None, vec![0.2, 1.0])]).unwrap();

        // Query closest to a1; both libraries searched, hit tagged with its kb id.
        let hits = search_at(&root, &[a.clone(), b.clone()], &[1.0, 0.0], 5).unwrap();
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
        save_chunks_at(&root, &kb, &[chunk_emb("old1", "doc_x", "x.md", None, vec![1.0])]).unwrap();

        let mut all = load_chunks_at(&root, &kb).unwrap();
        all.retain(|c| c.doc_id != "doc_x");
        all.push(chunk_emb("new1", "doc_x", "x.md", None, vec![1.0]));
        all.push(chunk_emb("new2", "doc_x", "x.md", None, vec![1.0]));
        save_chunks_at(&root, &kb, &all).unwrap();

        let chunks = load_chunks_at(&root, &kb).unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|c| c.id.starts_with("new")));

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
}
