//! Tauri commands for knowledge base management (library CRUD + document
//! list/delete). Ingest (upload/index) commands are added in `ingest.rs`.

use tauri::AppHandle;

use super::{KnowledgeDocument, KnowledgeLibrary};

#[tauri::command]
pub(crate) fn kb_list_libraries(app: AppHandle) -> Result<Vec<KnowledgeLibrary>, String> {
    super::load_libraries(&app)
}

#[tauri::command]
pub(crate) fn kb_create_library(
    app: AppHandle,
    name: String,
    provider_id: String,
    model: String,
) -> Result<KnowledgeLibrary, String> {
    super::create_library(&app, &name, &provider_id, &model)
}

#[tauri::command]
pub(crate) fn kb_rename_library(app: AppHandle, kb_id: String, name: String) -> Result<(), String> {
    super::rename_library(&app, &kb_id, &name)
}

#[tauri::command]
pub(crate) fn kb_delete_library(app: AppHandle, kb_id: String) -> Result<(), String> {
    super::delete_library(&app, &kb_id)
}

#[tauri::command]
pub(crate) fn kb_list_documents(
    app: AppHandle,
    kb_id: String,
) -> Result<Vec<KnowledgeDocument>, String> {
    super::load_docs(&app, &kb_id)
}

#[tauri::command]
pub(crate) fn kb_delete_document(
    app: AppHandle,
    kb_id: String,
    doc_id: String,
) -> Result<(), String> {
    super::delete_document(&app, &kb_id, &doc_id)
}
