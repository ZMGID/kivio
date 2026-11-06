// Chat 模块：AI 客户端核心功能
pub mod agent;
pub(crate) mod artifacts;
pub mod ask_user;
pub mod attachments;
pub mod commands;
mod completion_notification;
pub mod draft_journal;
pub mod dsml_tools;
pub mod export;
pub(crate) mod external_send;
pub mod gc;
pub mod goal;
pub mod hooks;
mod image_collage;
pub mod image_generation;
mod image_prep;
pub(crate) mod interaction_state;
pub mod knowledge_base;
mod mcp_image_feedback;
pub mod media_station;
pub mod memory;
pub mod model;
pub(crate) mod model_database;
pub mod model_metadata;
pub(crate) mod notification_viewing;
pub mod plan;
pub mod plan_document;
pub mod popout;
#[cfg(debug_assertions)]
pub mod probe;
pub mod protocol;
pub(crate) mod provider_runtime;
pub mod repository;
pub mod request_debug;
pub(crate) mod resources;
pub(crate) mod route_contract;
pub(crate) mod runtime_state;
pub(crate) mod slash_commands;
pub mod storage;
pub mod sub_agent;
pub mod todo;
pub mod types;
pub(crate) mod video;
mod video_analysis;
pub(crate) mod video_generation;
mod vision;
pub(crate) mod voice_orchestrator;
pub mod workflow_hooks;

pub use types::*;

pub(crate) fn format_chat_login_required_error(provider_name: &str) -> String {
    let provider = provider_name.trim();
    if provider.is_empty() {
        "Chat 模型供应商需要账号登录，请到设置 > 模型中登录后再发送。".to_string()
    } else {
        format!("Chat 模型供应商「{provider}」需要账号登录，请到设置 > 模型中登录后再发送。")
    }
}

pub(crate) fn chat_missing_model_error() -> String {
    "请先为当前 Chat 对话选择模型，或到设置 > AI 客户端配置默认模型。".to_string()
}

/// 混音器未单独指定压缩模型时，用当前会话的 provider/model（顶栏主模型），
/// 而不是设置里的全局 Chat 默认（`effective_chat_model`）。
pub(crate) fn session_model_for_conversation(
    conversation: &Conversation,
) -> crate::settings::SessionModel<'_> {
    crate::settings::SessionModel {
        provider_id: conversation.provider_id.as_str(),
        model: conversation.model.as_str(),
    }
}
