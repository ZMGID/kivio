use crate::chat::types::{AgentPlanMode, AgentPlanState, AgentPlanStatus, Conversation};
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub use crate::chat::conversation_data::PlanDocument;

pub fn tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__save_plan".into(), name: "save_plan".into(),
        description: "Save a Markdown plan document and return its file link. Provide title and content to create a plan; include plan_id to update that same plan. Before updating, read its current file and pass that text as previous_content to preserve user edits. This only writes plan documents; it does not start implementation. Reply with a short summary and the returned file link.".into(),
        source: "native".into(), server_id: None, server_name: Some("Kivio".into()),
        input_schema: json!({"type":"object","properties":{
            "title":{"type":"string"}, "content":{"type":"string"},
            "plan_id":{"type":"string"}, "previous_content":{"type":"string"}
        },"required":["title","content"]}),
        output_schema: None, annotations: None, sensitive: false,
    }
}

pub fn append_tools(tools: &mut Vec<ChatToolDefinition>, plan_mode: bool) {
    if plan_mode {
        tools.push(tool());
    }
}

fn plan_directory(app: &AppHandle, conversation: &Conversation) -> Result<PathBuf, String> {
    let settings = crate::settings::load_settings(app);
    match crate::chat::storage::resolve_conversation_working_directory(
        app,
        conversation,
        &settings.chat_tools.native_tools.working_directory,
    ) {
        Ok(root) => Ok(root.join(
            if conversation.project_id.is_some() || conversation.folder.is_some() {
                "docs/plans"
            } else {
                "plans"
            },
        )),
        Err(_) => Ok(crate::app_data::app_data_dir()
            .ok_or("App data directory unavailable")?
            .join("plans")
            .join(&conversation.id)),
    }
}

fn create_document(directory: &Path, title: &str, content: &str) -> Result<PlanDocument, String> {
    use std::io::Write;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let slug: String = title
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect();
    let path = directory.join(format!(
        "{}-{}-{}.md",
        chrono::Local::now().format("%Y-%m-%d"),
        if slug.is_empty() { "plan" } else { &slug },
        &id[..8]
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(PlanDocument {
        id,
        title: title.into(),
        path: path.to_string_lossy().into(),
    })
}

fn update_document(document: &PlanDocument, content: &str, previous: &str) -> Result<(), String> {
    let path = Path::new(&document.path);
    let current =
        std::fs::read_to_string(path).map_err(|e| format!("Cannot read plan document: {e}"))?;
    if current != previous {
        return Err(
            "Plan changed since it was read. Read the current file and apply your changes to it."
                .into(),
        );
    }
    crate::chat::storage::atomic_write(path, content, "plan document")?;
    Ok(())
}

pub fn handle<'a>(
    app: &'a AppHandle,
    ctx: &'a crate::mcp::registry::NativeToolContext,
    _: &'a str,
    args: Value,
) -> crate::mcp::native_registry::NativeToolFuture<'a> {
    Box::pin(async move {
        let title = args["title"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("Provide a plan title")?
            .trim();
        let content = args["content"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("Provide the plan content")?;
        let conversation = crate::chat::storage::load_conversation(app, &ctx.conversation_id)?;
        if !crate::chat::plan::is_plan_mode(&conversation.agent_plan_state) {
            return Err("save_plan is available in Plan mode".into());
        }
        let directory = plan_directory(app, &conversation)?;
        let persisted = crate::chat::repository::repository(app)
            .mutate(app, &ctx.conversation_id, |conversation| {
                if !crate::chat::plan::is_plan_mode(&conversation.agent_plan_state) {
                    return Err("Plan mode changed".into());
                }
                let document = if let Some(id) = args["plan_id"].as_str() {
                    let mut document = conversation
                        .agent_plan_state
                        .document
                        .as_ref()
                        .into_iter()
                        .chain(conversation.messages.iter().filter_map(|m| {
                            m.agent_plan.as_ref().and_then(|s| s.document.as_ref())
                        }))
                        .find(|d| d.id == id)
                        .cloned()
                        .ok_or("Unknown plan document")?;
                    let previous = args["previous_content"]
                        .as_str()
                        .ok_or("Read the plan and provide previous_content when updating")?;
                    update_document(&document, content, previous)?;
                    document.title = title.into();
                    document
                } else {
                    create_document(&directory, title, content)?
                };
                conversation.agent_plan_state = AgentPlanState {
                    document: Some(document),
                    mode: AgentPlanMode::Plan,
                    status: AgentPlanStatus::Draft,
                    plan: None,
                    updated_at: chrono::Local::now().timestamp(),
                };
                Ok(())
            })
            .await
            .map_err(crate::chat::repository::repository_error)?;
        crate::chat::protocol::emit_conversation_event(
            app,
            &persisted.id,
            persisted.revision,
            crate::chat::protocol::ChatConversationEvent::PlanUpdated {
                plan_state: (&persisted.agent_plan_state).into(),
            },
        );
        let value = json!({"document":persisted.agent_plan_state.document});
        Ok(McpToolCallResult {
            content: value.to_string(),
            is_error: false,
            raw: value.clone(),
            artifacts: vec![crate::native_tools::build_delivery_artifact_for_path(
                Path::new(&persisted.agent_plan_state.document.as_ref().unwrap().path),
            )?],
            structured_content: Some(value),
            follow_up_user_messages: vec![],
        })
    })
}

pub fn prepare_execution(
    app: &AppHandle,
    conversation: &mut Conversation,
    message_id: &str,
) -> Result<String, String> {
    prepare_execution_in(
        conversation,
        message_id,
        &plan_directory(app, conversation)?,
    )
}

pub(crate) fn prepare_execution_in(
    conversation: &mut Conversation,
    message_id: &str,
    directory: &Path,
) -> Result<String, String> {
    let mut selected = if message_id.is_empty() {
        conversation.agent_plan_state.clone()
    } else {
        conversation
            .messages
            .iter()
            .find(|m| m.id == message_id && m.role == "assistant")
            .and_then(|m| {
                m.agent_plan.clone().or_else(|| {
                    (conversation.agent_plan_state.document.is_none()
                        && conversation
                            .agent_plan_state
                            .plan
                            .as_deref()
                            .is_some_and(|text| text.trim() == m.content.trim()))
                    .then(|| conversation.agent_plan_state.clone())
                })
            })
            .ok_or("Plan message not found")?
    };
    let content = if let Some(document) = &selected.document {
        std::fs::read_to_string(&document.path)
            .map_err(|e| format!("Cannot read plan document: {e}"))?
    } else {
        let content = crate::chat::plan::current_plan_text(&selected)
            .ok_or("No plan selected")?
            .to_string();
        selected.document = Some(create_document(directory, "Plan", &content)?);
        content
    };
    if content.trim().is_empty() {
        return Err("The plan document is empty".into());
    }
    prepare_snapshot(&mut selected, &conversation.agent_plan_state.mode, &content);
    if let Some(message) = conversation
        .messages
        .iter_mut()
        .find(|m| m.id == message_id)
    {
        message.agent_plan = Some(selected.clone());
    }
    conversation.agent_plan_state = selected;
    Ok(content)
}

fn prepare_snapshot(selected: &mut AgentPlanState, mode: &AgentPlanMode, content: &str) {
    selected.mode = if *mode == AgentPlanMode::Orchestrate {
        AgentPlanMode::Orchestrate
    } else {
        AgentPlanMode::Act
    };
    selected.status = AgentPlanStatus::Draft;
    selected.plan = Some(content.to_string());
    selected.updated_at = chrono::Local::now().timestamp();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn execution_snapshot_survives_later_file_edits() {
        let dir = tempfile::tempdir().unwrap();
        let document = create_document(dir.path(), "Plan", "current version").unwrap();
        let mut selected = AgentPlanState {
            document: Some(document.clone()),
            ..Default::default()
        };
        let content = std::fs::read_to_string(&document.path).unwrap();
        prepare_snapshot(&mut selected, &AgentPlanMode::Orchestrate, &content);
        update_document(&document, "later edit", "current version").unwrap();
        assert_eq!(selected.plan.as_deref(), Some("current version"));
        assert_eq!(selected.mode, AgentPlanMode::Orchestrate);
        assert_eq!(selected.status, AgentPlanStatus::Draft);
        let saved = serde_json::to_string(&selected).unwrap();
        let restored: AgentPlanState = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.document, selected.document);
    }
    #[test]
    fn documents_keep_unicode_and_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let a = create_document(dir.path(), "中文 / 计划", "方案正文，无需列表").unwrap();
        let b = create_document(dir.path(), "中文 / 计划", "another").unwrap();
        assert_ne!(a.path, b.path);
        assert_eq!(
            std::fs::read_to_string(&a.path).unwrap(),
            "方案正文，无需列表"
        );
        update_document(&a, "新内容", "方案正文，无需列表").unwrap();
        assert!(update_document(&a, "覆盖", "方案正文，无需列表").is_err());
        assert_eq!(std::fs::read_to_string(&a.path).unwrap(), "新内容");
    }
}
