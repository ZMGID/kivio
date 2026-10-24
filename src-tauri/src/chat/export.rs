use super::storage::load_conversation;
use std::path::Path;
use tauri::AppHandle;
#[path = "conversation_export.rs"]
mod rendering;
#[cfg(test)]
use super::{
    Attachment, ChatMessage, ChatMessageSegmentKind, ChatMessageSegmentPhase, Conversation,
};
pub(crate) use rendering::render_conversation_markdown;

#[tauri::command]
pub(crate) fn chat_export_conversation_markdown(
    app: AppHandle,
    conversation_id: String,
    path: String,
    language: String,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("export path is empty".to_string());
    }
    let conversation = load_conversation(&app, &conversation_id)?;
    let markdown = render_conversation_markdown(&conversation, &language);
    std::fs::write(Path::new(&path), markdown)
        .map_err(|err| format!("write conversation export: {err}"))
}

#[tauri::command]
pub(crate) async fn chat_export_set_backup(
    app: AppHandle,
    set_id: String,
    path: String,
) -> Result<(), String> {
    super::repository::repository(&app)
        .export_set_backup(&app, set_id, path.into())
        .await
        .map_err(super::repository::repository_error)
}

#[tauri::command]
pub(crate) async fn chat_import_set_backup(
    app: AppHandle,
    path: String,
) -> Result<super::ChatSet, String> {
    super::repository::repository(&app)
        .import_set_backup(&app, path.into())
        .await
        .map_err(super::repository::repository_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{
        AgentPlanState, AgentRuntimeConfig, AgentTodoState, ConversationContextState,
    };
    use std::collections::HashMap;

    #[test]
    fn exported_body_matches_visible_body_fixtures() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/message-body.json"))
                .unwrap();
        for case in cases.as_array().unwrap() {
            let mut conversation = conversation();
            let mut message = conversation.messages.remove(1);
            message.content = case["content"].as_str().unwrap().to_string();
            message.segments = serde_json::from_value(case["segments"].clone()).unwrap();
            message.attachments.clear();
            let before = serde_json::to_value(&message).unwrap();
            conversation.messages = vec![message.clone()];
            let rendered = render_conversation_markdown(&conversation, "en");
            let mut expected = message.clone();
            expected.content = case["expected"].as_str().unwrap().to_string();
            expected.segments.clear();
            conversation.messages = vec![expected];
            assert_eq!(
                rendered,
                render_conversation_markdown(&conversation, "en"),
                "{}",
                case["name"]
            );
            assert_eq!(serde_json::to_value(&message).unwrap(), before);
        }
    }

    fn conversation() -> Conversation {
        Conversation {
            id: "conv_test".to_string(),
            revision: 0,
            title: "Export test".to_string(),
            provider_id: "secret-provider".to_string(),
            model: "gpt-test".to_string(),
            messages: vec![
                ChatMessage {
                    id: "user_1".to_string(),
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                    attachments: vec![Attachment {
                        id: "att_1".to_string(),
                        attachment_type: "file".to_string(),
                        name: "notes.pdf".to_string(),
                        path: "private/notes.pdf".to_string(),
                        content: None,
                    }],
                    reasoning: None,
                    artifacts: vec![],
                    tool_calls: vec![],
                    segments: vec![],
                    agent_plan: None,
                    api_messages: vec![],
                    model_messages: vec![],
                    active_skill_id: None,
                    run_entry: None,
                    stream_outcome: None,
                    usage: None,
                    anchor_usage: None,
                    cache_pair_input: None,
                    cache_pair_read: None,
                    group_id: None,
                    provider_id: None,
                    model: None,
                    timestamp: 1_700_000_000,
                    degraded: None,
                },
                ChatMessage {
                    id: "assistant_1".to_string(),
                    role: "assistant".to_string(),
                    content: "Final answer".to_string(),
                    attachments: vec![Attachment {
                        id: "att_2".to_string(),
                        attachment_type: "image".to_string(),
                        name: "chart.png".to_string(),
                        path: "private/chart.png".to_string(),
                        content: None,
                    }],
                    reasoning: Some("private reasoning".to_string()),
                    artifacts: vec![],
                    tool_calls: vec![],
                    segments: vec![],
                    agent_plan: None,
                    api_messages: vec![serde_json::json!({"secret": "api transcript"})],
                    model_messages: vec![],
                    active_skill_id: None,
                    run_entry: None,
                    stream_outcome: Some("completed".to_string()),
                    usage: None,
                    anchor_usage: None,
                    cache_pair_input: None,
                    cache_pair_read: None,
                    group_id: None,
                    provider_id: None,
                    model: None,
                    timestamp: 1_700_000_001,
                    degraded: None,
                },
            ],
            agent_runtime: AgentRuntimeConfig::default(),
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_001,
            pinned: false,
            archived: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState::default(),
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            goal_state: None,
            knowledge_base_ids: vec![],
            force_knowledge_search: false,
            additional_directories: Vec::new(),
            thinking_level: None,
            web_search_mode: None,
            reply_models: vec![],
            group_selections: HashMap::new(),
            forked_from: None,
        }
    }

    #[test]
    fn renders_readable_chinese_markdown_without_internal_fields() {
        let markdown = render_conversation_markdown(&conversation(), "zh");
        assert!(markdown.contains("# Export test"));
        assert!(markdown.contains("**用户** ·"));
        assert!(markdown.contains("**助手** ·"));
        assert!(markdown.contains("[附件: notes.pdf]"));
        assert!(markdown.contains("[图片: chart.png]"));
        assert!(markdown.contains("Final answer"));
        assert!(!markdown.contains("private reasoning"));
        assert!(!markdown.contains("api transcript"));
        assert!(!markdown.contains("secret-provider"));
        assert!(!markdown.contains("private/chart.png"));
    }

    #[test]
    fn renders_memory_text_attachment_body_inline() {
        let mut conversation = conversation();
        conversation.messages[0].attachments.push(Attachment {
            id: "att_mem".to_string(),
            attachment_type: "file".to_string(),
            name: "已粘贴的文本.txt".to_string(),
            path: "memory://abc".to_string(),
            content: Some("第一行日志\n第二行日志".to_string()),
        });

        let markdown = render_conversation_markdown(&conversation, "zh");
        assert!(markdown.contains("[附件: 已粘贴的文本.txt]"));
        assert!(markdown.contains("附件正文"));
        assert!(markdown.contains("第一行日志\n第二行日志"));
    }

    #[test]
    fn renders_english_labels_and_skips_empty_internal_only_messages() {
        let mut conversation = conversation();
        conversation.messages.push(ChatMessage {
            id: "assistant_empty".to_string(),
            role: "assistant".to_string(),
            content: String::new(),
            attachments: vec![],
            reasoning: Some("hidden".to_string()),
            artifacts: vec![],
            tool_calls: vec![],
            segments: vec![],
            agent_plan: None,
            api_messages: vec![],
            model_messages: vec![],
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            anchor_usage: None,
            cache_pair_input: None,
            cache_pair_read: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 1_700_000_002,
            degraded: None,
        });

        let markdown = render_conversation_markdown(&conversation, "en");
        assert!(markdown.contains("- Created:"));
        assert!(markdown.contains("**User** ·"));
        assert!(markdown.contains("**Assistant** ·"));
        assert!(markdown.contains("[Attachment: notes.pdf]"));
        assert!(markdown.contains("[Image: chart.png]"));
        assert_eq!(markdown.matches("**Assistant** ·").count(), 1);
        assert!(!markdown.contains("hidden"));
    }
}
