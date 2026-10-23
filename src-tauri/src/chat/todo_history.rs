//! Reconstruct the todo state from retained canonical conversation history.
use crate::chat::conversation_data::{AgentTodoState, ChatMessage, ToolCallRecord, ToolCallStatus};

/// The todo state a conversation should have when its history is `messages`: the
/// last todo snapshot recorded on a successful tool call (native `todo_write` and
/// external CLI todo tools both record `structuredContent.todoState`), or empty.
/// Used after rewind / regenerate / fork so the list matches the retained history.
pub fn state_from_messages(
    messages: &[ChatMessage],
    group_selections: &std::collections::HashMap<String, String>,
) -> AgentTodoState {
    let mut first_arm: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for message in messages {
        if let Some(group) = message.group_id.as_deref() {
            first_arm.entry(group).or_insert(message.id.as_str());
        }
    }
    messages
        .iter()
        .rev()
        .filter(|message| match message.group_id.as_deref() {
            // Only the selected answer of a multi-answer group counts.
            Some(group) => match group_selections.get(group) {
                Some(selected) => selected == &message.id,
                None => first_arm.get(group) == Some(&message.id.as_str()),
            },
            None => true,
        })
        .find_map(|message| latest_recorded_state(&message.tool_calls))
        .unwrap_or_default()
}

/// Last todo snapshot recorded on a successful tool call in `records`.
pub fn latest_recorded_state(records: &[ToolCallRecord]) -> Option<AgentTodoState> {
    records
        .iter()
        .rev()
        .filter(|record| record.status == ToolCallStatus::Success)
        .find_map(|record| {
            let todo = record.structured_content.as_ref()?.get("todoState")?;
            serde_json::from_value::<AgentTodoState>(todo.clone()).ok()
        })
}
