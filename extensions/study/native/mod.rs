//! Persistent local reading library. Chat histories remain owned by chat storage.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

mod annotations;
mod bibliography;
pub mod chat;
pub mod operations;
pub mod setup;
pub mod writing;

static WRITES: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub page: usize,
    pub text: String,
    pub rect: Option<Rect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rects: Vec<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<PdfLocation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfLocation {
    pub kind: operations::PdfKind,
    pub run_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadingNote {
    pub id: String,
    pub content: String,
    pub anchor: Option<Anchor>,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingDocument {
    pub id: String,
    pub name: String,
    pub conversation_ids: Vec<String>,
    pub active_conversation_id: Option<String>,
    pub page: usize,
    pub offset: f64,
    pub notes: Vec<ReadingNote>,
    pub note_draft: String,
    #[serde(default)]
    pub bibliography: Bibliography,
    #[serde(default)]
    pub collection_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "unread")]
    pub status: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub trashed_at: Option<String>,
    #[serde(default)]
    pub added_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub last_opened_at: String,
    #[serde(default = "yes")]
    pub has_pdf: bool,
    #[serde(default)]
    pub fingerprint: String,
}
fn unread() -> String {
    "unread".into()
}
fn yes() -> bool {
    true
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bibliography {
    pub authors: Vec<String>,
    pub year: String,
    pub publication: String,
    pub doi: String,
    pub url: String,
    pub abstract_text: String,
    pub item_type: String,
    pub volume: String,
    pub issue: String,
    pub pages: String,
    pub publisher: String,
    pub isbn: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}
fn blank(name: String) -> ReadingDocument {
    ReadingDocument {
        id: Uuid::new_v4().to_string(),
        name,
        conversation_ids: vec![],
        active_conversation_id: None,
        page: 1,
        offset: 0.0,
        notes: vec![],
        note_draft: String::new(),
        bibliography: Bibliography::default(),
        collection_ids: vec![],
        tags: vec![],
        status: unread(),
        favorite: false,
        trashed_at: None,
        added_at: now(),
        updated_at: now(),
        last_opened_at: String::new(),
        has_pdf: false,
        fingerprint: String::new(),
    }
}
#[derive(Serialize)]
pub struct LibraryListing {
    documents: Vec<ReadingDocument>,
    issues: Vec<String>,
}
fn scan(root: &Path) -> Result<LibraryListing, String> {
    let mut result = LibraryListing {
        documents: vec![],
        issues: vec![],
    };
    if !root.exists() {
        return Ok(result);
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let path = entry.path().join("document.json");
        if !path.exists() {
            continue;
        }
        let record = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                serde_json::from_str::<ReadingDocument>(&text).map_err(|e| e.to_string())
            })
            .and_then(|doc| {
                if entry.file_name().to_str() != Some(doc.id.as_str()) {
                    return Err("文献 ID 与目录不匹配".into());
                }
                Ok(doc)
            });
        match record {
            Ok(doc) => result.documents.push(doc),
            Err(error) => result
                .issues
                .push(format!("{}：{error}（原文件已保留）", path.display())),
        }
    }
    result.documents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}
fn list(root: &Path) -> Result<Vec<ReadingDocument>, String> {
    Ok(scan(root)?.documents)
}
fn directory(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("reading");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}
fn document_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    Uuid::parse_str(id).map_err(|_| "无效的文档 ID".to_string())?;
    Ok(root.join(id))
}
fn save(root: &Path, document: &ReadingDocument) -> Result<(), String> {
    let dir = document_dir(root, &document.id)?;
    if document.name.trim().is_empty() {
        return Err("标题不能为空".into());
    }
    if !["unread", "reading", "read"].contains(&document.status.as_str()) {
        return Err("无效的阅读状态".into());
    }
    if document.has_pdf && !dir.join("source.pdf").is_file() {
        return Err("原文文件不存在".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let content = serde_json::to_string_pretty(document).map_err(|e| e.to_string())?;
    crate::extension_sdk::atomic_write(&dir.join("document.json"), &content, "reading document")
}
fn import(root: &Path, source: &Path) -> Result<ReadingDocument, String> {
    import_with_index(root, source, &mut list(root)?)
}
fn import_with_index(
    root: &Path,
    source: &Path,
    existing_docs: &mut Vec<ReadingDocument>,
) -> Result<ReadingDocument, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(source).map_err(|e| format!("无法打开 PDF：{e}"))?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length > 100 * 1024 * 1024 {
        return Err("请选择小于 100 MB 的 PDF".into());
    }
    let mut header = [0; 5];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header != b"%PDF-" {
        return Err("文件不是有效的 PDF".into());
    }
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    let fingerprint = format!("{:x}", Sha256::digest(&bytes));
    for existing in existing_docs.iter_mut() {
        // Legacy records acquire a fingerprint lazily without changing their annotations.
        if existing.fingerprint.is_empty() && existing.has_pdf {
            let original = std::fs::read(document_dir(root, &existing.id)?.join("source.pdf"))
                .map_err(|e| e.to_string())?;
            existing.fingerprint = format!("{:x}", Sha256::digest(original));
            save(root, existing)?;
        }
        if existing.fingerprint == fingerprint {
            return Ok(existing.clone());
        }
    }
    let mut doc = blank(
        source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    );
    doc.has_pdf = true;
    doc.fingerprint = fingerprint;
    let dir = document_dir(root, &doc.id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        std::fs::copy(source, dir.join("source.pdf")).map_err(|e| e.to_string())?;
        save(root, &doc)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(error);
    }
    existing_docs.push(doc.clone());
    Ok(doc)
}
#[tauri::command]
pub async fn reading_list(app: AppHandle) -> Result<LibraryListing, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || scan(&directory(&app)?))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn reading_import(app: AppHandle, path: String) -> Result<ReadingDocument, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        import(&directory(&app)?, Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[derive(Serialize)]
pub struct ImportBatch {
    documents: Vec<ReadingDocument>,
    failures: Vec<String>,
}
#[tauri::command]
pub async fn reading_import_many(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<ImportBatch, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let root = directory(&app)?;
        let mut index = list(&root)?;
        let mut result = ImportBatch {
            documents: vec![],
            failures: vec![],
        };
        for path in paths {
            match import_with_index(&root, Path::new(&path), &mut index) {
                Ok(doc) => result.documents.push(doc),
                Err(error) => result.failures.push(format!("{path}：{error}")),
            }
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn reading_save(app: AppHandle, document: ReadingDocument) -> Result<(), String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        save(&directory(&app)?, &document)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn reading_pdf(
    app: AppHandle,
    id: String,
    kind: Option<operations::PdfKind>,
    run_id: Option<String>,
) -> Result<tauri::ipc::Response, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(operations::pdf_path(
            &document_dir(&directory(&app)?, &id)?,
            kind.unwrap_or_default(),
            run_id.as_deref(),
        )?)
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn reading_create(
    app: AppHandle,
    name: String,
    bibliography: Bibliography,
) -> Result<ReadingDocument, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let root = directory(&app)?;
        create(&root, name, bibliography)
    })
    .await
    .map_err(|e| e.to_string())?
}
fn create(
    root: &Path,
    name: String,
    bibliography: Bibliography,
) -> Result<ReadingDocument, String> {
    let doi = bibliography.doi.trim().to_lowercase();
    if !doi.is_empty() {
        if let Some(existing) = list(root)?
            .into_iter()
            .find(|d| d.bibliography.doi.trim().eq_ignore_ascii_case(&doi))
        {
            return Ok(existing);
        }
    }
    let mut doc = blank(name);
    doc.bibliography = bibliography;
    save(root, &doc)?;
    Ok(doc)
}
#[tauri::command]
pub async fn reading_attach(
    app: AppHandle,
    id: String,
    path: String,
) -> Result<ReadingDocument, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let root = directory(&app)?;
        let dir = document_dir(&root, &id)?;
        let mut doc: ReadingDocument = serde_json::from_str(
            &std::fs::read_to_string(dir.join("document.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if doc.has_pdf {
            return Err("此文献已有 PDF，请另建条目保存不同版本，避免覆盖阅读标注".into());
        }
        if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 100 * 1024 * 1024 {
            return Err("请选择小于 100 MB 的 PDF".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if !bytes.starts_with(b"%PDF-") || bytes.len() > 100 * 1024 * 1024 {
            return Err("请选择有效且小于 100 MB 的 PDF".into());
        }
        use sha2::{Digest, Sha256};
        doc.fingerprint = format!("{:x}", Sha256::digest(&bytes));
        std::fs::write(dir.join("source.pdf"), bytes).map_err(|e| e.to_string())?;
        doc.has_pdf = true;
        doc.updated_at = now();
        save(&root, &doc)?;
        Ok(doc)
    })
    .await
    .map_err(|e| e.to_string())?
}
fn collections(root: &Path) -> Result<Vec<Collection>, String> {
    let path = root.join("collections.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn save_collections(root: &Path, items: &[Collection]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for item in items {
        Uuid::parse_str(&item.id).map_err(|_| "无效的分类 ID")?;
        if item.name.trim().is_empty() || !ids.insert(&item.id) {
            return Err("分类名称不能为空，ID 不可重复".into());
        }
        let mut seen = std::collections::HashSet::from([item.id.as_str()]);
        let mut parent = item.parent_id.as_deref();
        while let Some(id) = parent {
            if !seen.insert(id) {
                return Err("分类不能包含自身".into());
            }
            let p = items.iter().find(|c| c.id == id).ok_or("上级分类不存在")?;
            parent = p.parent_id.as_deref();
        }
    }
    crate::extension_sdk::atomic_write(
        &root.join("collections.json"),
        &serde_json::to_string_pretty(items).map_err(|e| e.to_string())?,
        "reading collections",
    )
}
#[tauri::command]
pub async fn reading_collections(app: AppHandle) -> Result<Vec<Collection>, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || collections(&directory(&app)?))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn reading_save_collections(
    app: AppHandle,
    items: Vec<Collection>,
) -> Result<(), String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        save_collections(&directory(&app)?, &items)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn reading_lookup(doi: String) -> Result<bibliography::Lookup, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    bibliography::lookup(&doi).await
}
#[tauri::command]
pub async fn reading_import_ris(
    app: AppHandle,
    path: String,
) -> Result<Vec<ReadingDocument>, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let root = directory(&app)?;
        if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 10 * 1024 * 1024 {
            return Err("RIS 文件请控制在 10 MB 内".into());
        }
        let file = std::fs::read(path).map_err(|e| e.to_string())?;
        if file.len() > 10 * 1024 * 1024 {
            return Err("RIS 文件请控制在 10 MB 内".into());
        }
        let text = String::from_utf8(file).map_err(|_| "请将 RIS 导出为 UTF-8 编码")?;
        let records = bibliography::parse_ris(&text)?;
        records
            .into_iter()
            .map(|record| {
                create(&root, record.name, record.bibliography).and_then(|mut doc| {
                    for tag in record.tags {
                        if !doc.tags.contains(&tag) {
                            doc.tags.push(tag);
                        }
                    }
                    save(&root, &doc)?;
                    Ok(doc)
                })
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn reading_export_ris(
    app: AppHandle,
    ids: Vec<String>,
    path: String,
) -> Result<(), String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let docs: Vec<_> = list(&directory(&app)?)?
            .into_iter()
            .filter(|d| ids.contains(&d.id))
            .collect();
        std::fs::write(path, bibliography::export_ris(&docs)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_pdf_and_reading_work_survive_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("paper.pdf");
        std::fs::write(&source, b"%PDF-1.7\ntest bytes").unwrap();
        let root = temp.path().join("library");
        let mut doc = import(&root, &source).unwrap();
        std::fs::remove_file(source).unwrap();
        doc.page = 3;
        doc.last_opened_at = "2026-10-18T03:00:00Z".into();
        doc.active_conversation_id = Some("chat-1".into());
        doc.conversation_ids.push("chat-1".into());
        doc.note_draft = "unsaved thought".into();
        doc.notes.push(ReadingNote {
            id: "note-1".into(),
            content: "important finding".into(),
            anchor: Some(Anchor {
                page: 3,
                text: "evidence".into(),
                rect: None,
                rects: vec![
                    Rect {
                        x: 0.1,
                        y: 0.2,
                        w: 0.3,
                        h: 0.02,
                    },
                    Rect {
                        x: 0.1,
                        y: 0.23,
                        w: 0.2,
                        h: 0.02,
                    },
                ],
                mark: Some("underline".into()),
                color: Some("#4b95c9".into()),
                view: None,
            }),
            kind: "highlight".into(),
        });
        save(&root, &doc).unwrap();
        let folder = document_dir(&root, &doc.id).unwrap();
        let loaded: ReadingDocument =
            serde_json::from_slice(&std::fs::read(folder.join("document.json")).unwrap()).unwrap();
        assert_eq!(loaded.page, 3);
        assert_eq!(loaded.last_opened_at, "2026-10-18T03:00:00Z");
        assert_eq!(loaded.active_conversation_id.as_deref(), Some("chat-1"));
        assert_eq!(loaded.notes[0].anchor.as_ref().unwrap().page, 3);
        let anchor = loaded.notes[0].anchor.as_ref().unwrap();
        assert_eq!(anchor.mark.as_deref(), Some("underline"));
        assert_eq!(anchor.color.as_deref(), Some("#4b95c9"));
        assert_eq!(anchor.rects.len(), 2);
        assert_eq!(anchor.rects[1].y, 0.23);
        let legacy: Anchor =
            serde_json::from_str(r#"{"page":1,"text":"legacy","rect":null}"#).unwrap();
        assert!(legacy.rects.is_empty());
        assert!(legacy.mark.is_none());
        assert_eq!(loaded.note_draft, "unsaved thought");
        assert_eq!(
            std::fs::read(folder.join("source.pdf")).unwrap(),
            b"%PDF-1.7\ntest bytes"
        );
    }
    #[test]
    fn damaged_record_does_not_hide_healthy_documents_or_get_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let healthy = create(temp.path(), "Healthy".into(), Bibliography::default()).unwrap();
        let broken = temp.path().join(Uuid::new_v4().to_string());
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("document.json"), "{broken").unwrap();
        let report = scan(temp.path()).unwrap();
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].contains(broken.to_str().unwrap()));
        let documents = report.documents;
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].id, healthy.id);
        assert_eq!(
            std::fs::read_to_string(broken.join("document.json")).unwrap(),
            "{broken"
        );
    }
    #[test]
    fn invalid_import_and_path_escape_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("not-a-pdf.pdf");
        std::fs::write(&source, b"plain text").unwrap();
        assert!(import(temp.path(), &source).is_err());
        assert!(document_dir(temp.path(), "../escape").is_err());
    }
    #[test]
    fn metadata_only_items_collections_and_trash_survive_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut doc = create(
            root,
            "A research paper".into(),
            Bibliography {
                authors: vec!["Ada Lovelace".into()],
                year: "1843".into(),
                doi: "10.1000/example".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!doc.has_pdf);
        let collection = Collection {
            id: Uuid::new_v4().to_string(),
            name: "Research".into(),
            parent_id: None,
            deleted: false,
        };
        save_collections(root, &[collection.clone()]).unwrap();
        doc.collection_ids.push(collection.id.clone());
        doc.tags.push("computing".into());
        doc.trashed_at = Some(now());
        doc.note_draft = "Keep this note".into();
        save(root, &doc).unwrap();
        let mut reopened = list(root).unwrap().remove(0);
        assert_eq!(reopened.bibliography.authors, vec!["Ada Lovelace"]);
        assert_eq!(reopened.collection_ids, vec![collection.id.clone()]);
        assert!(reopened.trashed_at.is_some());
        assert_eq!(collections(root).unwrap()[0].name, "Research");
        save_collections(
            root,
            &[Collection {
                deleted: true,
                ..collection
            }],
        )
        .unwrap();
        reopened.trashed_at = None;
        save(root, &reopened).unwrap();
        let restored = list(root).unwrap().remove(0);
        assert_eq!(restored.note_draft, "Keep this note");
        assert_eq!(restored.tags, vec!["computing"]);
        assert!(restored.trashed_at.is_none());
        let same = create(
            root,
            "Alternate title".into(),
            Bibliography {
                doi: "10.1000/EXAMPLE".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(same.id, doc.id);
        assert_eq!(list(root).unwrap().len(), 1);
    }
    #[test]
    fn legacy_pdf_is_migrated_and_identical_reimport_preserves_reading_state() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("paper.pdf");
        std::fs::write(&source, b"%PDF-1.7 same bytes").unwrap();
        let root = temp.path().join("library");
        let mut doc = import(&root, &source).unwrap();
        doc.page = 7;
        doc.note_draft = "legacy note".into();
        let mut old = serde_json::to_value(&doc).unwrap();
        for key in [
            "bibliography",
            "collectionIds",
            "tags",
            "status",
            "favorite",
            "trashedAt",
            "addedAt",
            "updatedAt",
            "lastOpenedAt",
            "hasPdf",
            "fingerprint",
        ] {
            old.as_object_mut().unwrap().remove(key);
        }
        std::fs::write(
            document_dir(&root, &doc.id).unwrap().join("document.json"),
            serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        let same = import(&root, &source).unwrap();
        assert_eq!(same.id, doc.id);
        assert_eq!(same.page, 7);
        assert_eq!(same.note_draft, "legacy note");
        assert!(same.has_pdf);
        assert_eq!(same.status, "unread");
        assert!(same.last_opened_at.is_empty());
        let reopened = list(&root).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened[0].fingerprint, same.fingerprint);
        assert!(!reopened[0].fingerprint.is_empty());
    }
    #[test]
    fn collection_cycles_do_not_replace_valid_saved_tree() {
        let temp = tempfile::tempdir().unwrap();
        let a = Uuid::new_v4().to_string();
        let b = Uuid::new_v4().to_string();
        let mut tree = vec![
            Collection {
                id: a.clone(),
                name: "Parent".into(),
                parent_id: None,
                deleted: false,
            },
            Collection {
                id: b.clone(),
                name: "Child".into(),
                parent_id: Some(a),
                deleted: false,
            },
        ];
        save_collections(temp.path(), &tree).unwrap();
        tree[0].parent_id = Some(b);
        assert!(save_collections(temp.path(), &tree).is_err());
        assert!(collections(temp.path()).unwrap()[0].parent_id.is_none());
    }
    #[test]
    fn ris_exchange_preserves_bibliographic_fields_and_rejects_incomplete_records() {
        let records = bibliography::parse_ris("TY  - JOUR\nTI  - 文献标题\nAU  - Ada Lovelace\nAU  - Alan Turing\nPY  - 2024/03/01\nT2  - Journal\nDO  - 10.1000/example\nAB  - First line\n continuation\nKW  - machine learning\nER  - \n").unwrap();
        let record = &records[0];
        assert_eq!(record.bibliography.abstract_text, "First line continuation");
        let mut doc = blank(record.name.clone());
        doc.bibliography = record.bibliography.clone();
        doc.tags = record.tags.clone();
        let exported = bibliography::export_ris(&[doc]);
        assert!(exported.contains("AU  - Alan Turing"));
        let again = bibliography::parse_ris(&exported).unwrap();
        assert_eq!(again[0].name, "文献标题");
        assert_eq!(again[0].bibliography.year, "2024");
        assert_eq!(again[0].tags, vec!["machine learning"]);
        assert!(bibliography::parse_ris("TY  - JOUR\nTI  - unfinished").is_err());
    }
}

/// Native counterpart of the Study package. Only the host calls lifecycle methods.
pub async fn activate(app: &AppHandle) -> Result<(), String> {
    // Word is optional: an occupied port must not make the reader or extension
    // manager unusable at startup. The explicit start action reports the error.
    if let Err(error) = writing::word::resume(app).await {
        eprintln!("Study Word resume: {error}");
    }
    Ok(())
}
pub async fn deactivate() {
    setup::shutdown().await;
    operations::shutdown().await;
    writing::word::suspend().await;
}
pub fn invoke(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    if !crate::extension_sdk::is_active("study") {
        invoke.resolver.reject("请先在扩展管理中启用 Kivio Study。");
        return true;
    }
    let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        crate::reading::setup::reading_setup_status,
        crate::reading::setup::reading_setup_save,
        crate::reading::setup::reading_setup_start,
        crate::reading::operations::reading_translation_status,
        crate::reading::operations::reading_versions,
        crate::reading::operations::reading_view_select,
        crate::reading::operations::reading_translation_start,
        crate::reading::operations::reading_translation_cancel,
        crate::reading::operations::reading_export,
        crate::reading::reading_list,
        crate::reading::reading_import_many,
        crate::reading::reading_import,
        crate::reading::reading_save,
        crate::reading::reading_pdf,
        crate::reading::chat::reading_prepare_chat,
        crate::reading::chat::reading_text,
        crate::reading::chat::reading_missing_conversations,
        crate::reading::reading_create,
        crate::reading::reading_attach,
        crate::reading::reading_collections,
        crate::reading::reading_save_collections,
        crate::reading::reading_lookup,
        crate::reading::reading_import_ris,
        crate::reading::reading_export_ris,
        crate::reading::writing::reading_export_bib,
        crate::reading::writing::word::reading_word_status,
        crate::reading::writing::word::reading_word_start,
        crate::reading::writing::word::reading_word_stop,
        crate::reading::writing::word::reading_word_install,
    ];
    handler(invoke)
}
