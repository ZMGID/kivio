//! Tauri commands for knowledge base management (library CRUD + document
//! list/delete). Ingest (upload/index) commands are added in `ingest.rs`.

use tauri::{AppHandle, Manager};

use super::{KnowledgeDocument, KnowledgeLibrary};

#[tauri::command]
pub(crate) fn kb_list_libraries(app: AppHandle) -> Result<Vec<KnowledgeLibrary>, String> {
    // 懒触发：首次打开知识库面板时才复位上次中断的 indexing 状态（避免在启动时同步开 SQLite）。
    super::heal_stale_indexing_once(&app);
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

/// Run a retrieval against the given libraries through the SAME core the
/// `knowledge_search` tool uses, with diagnostics on. Powers the Retrieval Test
/// UI: returns per-lane ranks/scores, per-stage timings, rerank status and the
/// effective config so the user can see exactly which stage surfaced or dropped
/// each passage. `top_k` overrides the configured context size when > 0.
#[tauri::command]
pub(crate) async fn kb_retrieval_test(
    app: AppHandle,
    kb_ids: Vec<String>,
    query: String,
    top_k: Option<u32>,
) -> Result<super::retrieval::RetrievalResponse, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err("查询不能为空".to_string());
    }
    if kb_ids.is_empty() {
        return Err("请至少选择一个知识库".to_string());
    }

    // Resolve config exactly like the tool path (see `call_knowledge_search`).
    let state = app.state::<crate::state::AppState>();
    let settings = state.settings_read().clone();
    let kbcfg = &settings.knowledge_base;
    let context_top_k = top_k
        .map(|n| n as usize)
        .filter(|n| *n > 0)
        .unwrap_or((kbcfg.top_k as usize).clamp(1, 20))
        .min(20);
    let (w_vec, w_kw) = if kbcfg.hybrid_enabled {
        (kbcfg.weight_vector, kbcfg.weight_keyword)
    } else {
        (1.0, 0.0)
    };
    let rerank = (!kbcfg.rerank_provider_id.trim().is_empty()
        && !kbcfg.rerank_model.trim().is_empty())
    .then(|| super::retrieval::RerankConfig {
        provider_id: kbcfg.rerank_provider_id.clone(),
        model: kbcfg.rerank_model.clone(),
    });
    let req = super::retrieval::RetrievalRequest {
        query,
        kb_ids,
        candidate_k: kbcfg.candidate_k_clamped(),
        rerank_top_k: kbcfg.rerank_top_k_clamped(),
        context_top_k,
        weight_vector: w_vec,
        weight_keyword: w_kw,
        rerank,
        min_score: kbcfg.min_score.clamp(0.0, 1.0),
    };
    super::retrieval::retrieve(&app, &state, &settings, &req).await
}
