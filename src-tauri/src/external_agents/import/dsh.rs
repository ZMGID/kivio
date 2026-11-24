use std::path::Path;

use serde::Deserialize;

use super::ImportableSession;
use crate::external_agents::{
    import_history::{parse_dsh_history, ImportedMessage},
    providers::dsh::DSH_AGENT_DEF,
    session::dsh_jsonrpc::query_import,
    spawn::resolve_binary,
    types::CapabilitySupport,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSession {
    session_id: String,
    cwd: String,
    title: Option<String>,
    updated_at: Option<i64>,
    message_count: usize,
}

pub(super) async fn list_sessions(project_root: &str) -> Result<Vec<ImportableSession>, String> {
    #[derive(Deserialize)]
    struct Listed {
        sessions: Vec<StoredSession>,
    }

    let Some(binary) = resolve_binary(&DSH_AGENT_DEF).await else {
        return Ok(Vec::new());
    };
    let result = query_import(&binary, Path::new(project_root), None).await?;
    let listed = serde_json::from_value::<Listed>(result)
        .map_err(|error| format!("dsh import session list: {error}"))?;
    Ok(listed
        .sessions
        .into_iter()
        .map(|session| ImportableSession {
            agent_id: DSH_AGENT_DEF.id.to_owned(),
            import_history: CapabilitySupport::Native,
            session_id: session.session_id,
            title: session.title,
            cwd: session.cwd,
            updated_at: session.updated_at,
            message_count: Some(session.message_count),
            already_imported: false,
            bound_conversation_id: None,
        })
        .collect())
}

pub(super) async fn load_history(
    project_root: &str,
    session_id: &str,
) -> Result<Vec<ImportedMessage>, String> {
    #[derive(Deserialize)]
    struct History {
        events: Vec<serde_json::Value>,
    }

    let binary = resolve_binary(&DSH_AGENT_DEF)
        .await
        .ok_or_else(|| "找不到 DeepSeek Harness（dsh）".to_owned())?;
    let result = query_import(&binary, Path::new(project_root), Some(session_id)).await?;
    let history = serde_json::from_value::<History>(result)
        .map_err(|error| format!("dsh import history: {error}"))?;
    let messages = parse_dsh_history(&history.events);
    if messages.is_empty() {
        return Err("DeepSeek Harness 会话没有可显示的历史".into());
    }
    Ok(messages)
}
