//! Tauri commands for knowledge base management (library CRUD + document
//! list/delete). Ingest (upload/index) commands are added in `ingest.rs`.

use tauri::{AppHandle, Manager};

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
    // 防呆：库引用的 embedding 供应商必须已保存（存在于运行时设置）。否则会留下
    // 悬空 provider 引用 —— 检索时静默查不到模型（代码审查发现的根因）。
    {
        let state = app.state::<crate::state::AppState>();
        let settings = state.settings_read();
        if settings.get_provider(&provider_id).is_none() {
            return Err(format!(
                "供应商「{provider_id}」尚未保存或不存在，请先在「设置」中保存该供应商再建库。"
            ));
        }
    }
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
