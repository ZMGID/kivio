//! Built-in agent todo list (`todo_write`).
//!
//! The conversation's `agent_todo_state` stays the single authority: `todo_write`
//! writes it, rewind/fork/regenerate recompute it from the retained history
//! ([`state_from_messages`]), and external CLIs write it through their own adapters.
//!
//! The list is deliberately **not** part of the system prompt: a changing list there
//! would invalidate the whole prompt cache every time it changed. The model sees it
//! the way Codex / Claude Code / ZCode do: through its own earlier `todo_write`
//! calls in history. [`reminder_for_next_step`] appends a reminder only when those
//! calls are no longer visible (compaction, context clear) or have not happened for
//! a while (ZCode / Claude Code's 10-step todo reminder).

use tauri::AppHandle;
include!("todo_runtime.rs");

/// Conversation-scoped registry handler: apply the write, persist, emit the typed
/// protocol update, and return the tool result. Deliberately does not resolve a
/// native tool workspace.
pub fn handle_conversation_tool_call<'a>(
    app: &'a AppHandle,
    ctx: &'a crate::mcp::registry::NativeToolContext,
    tool_name: &'a str,
    arguments: Value,
) -> crate::mcp::native_registry::NativeToolFuture<'a> {
    Box::pin(async move {
        let conversation_id = ctx.conversation_id.as_str();
        if !is_agent_todo_tool_name(tool_name) {
            return Err(format!("Unknown todo tool: {tool_name}"));
        }
        let outcome = apply_todo_write(arguments)?;
        let persisted = crate::chat::repository::repository(app)
            .update_todo(app, conversation_id, outcome.state.clone())
            .await
            .map_err(crate::chat::repository::repository_error)?;
        emit_chat_todo_state(
            app,
            conversation_id,
            persisted.revision,
            &persisted.agent_todo_state,
        );
        Ok(tool_result(&outcome))
    })
}

pub fn emit_chat_todo_state(
    app: &AppHandle,
    conversation_id: &str,
    revision: u64,
    todo_state: &AgentTodoState,
) {
    crate::chat::protocol::emit_conversation_event(
        app,
        conversation_id,
        revision,
        crate::chat::protocol::ChatConversationEvent::TodoUpdated {
            todo_state: todo_state.into(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::conversation_data::{ChatMessage, ToolCallRecord, ToolCallStatus};
    use crate::chat::types::Conversation;
    use serde_json::json;

    fn write(todos: Value) -> TodoToolOutcome {
        apply_todo_write(json!({ "todos": todos })).expect("todo write should succeed")
    }

    fn item(content: &str, status: AgentTodoStatus) -> AgentTodoItem {
        AgentTodoItem {
            id: content.to_string(),
            content: content.to_string(),
            status,
            ..Default::default()
        }
    }

    fn state(items: Vec<AgentTodoItem>) -> AgentTodoState {
        AgentTodoState {
            items,
            updated_at: 1,
        }
    }

    fn assistant(tool: Option<&str>) -> Value {
        match tool {
            Some(name) => json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "id": "c", "type": "function", "function": { "name": name, "arguments": "{}" } }]
            }),
            None => json!({ "role": "assistant", "content": "step" }),
        }
    }

    fn tool_record(
        name: &str,
        status: ToolCallStatus,
        todo: Option<AgentTodoState>,
    ) -> ToolCallRecord {
        let mut record: ToolCallRecord = serde_json::from_value(json!({
            "id": format!("call_{name}"), "name": name, "status": "success"
        }))
        .expect("minimal tool record");
        record.status = status;
        record.structured_content = todo.map(|state| json!({ "todoState": state }));
        record
    }

    fn message(id: &str, group: Option<&str>, records: Vec<ToolCallRecord>) -> ChatMessage {
        let mut message: ChatMessage = serde_json::from_value(json!({
            "id": id, "role": "assistant", "content": "", "timestamp": 1
        }))
        .expect("minimal message");
        message.group_id = group.map(str::to_string);
        message.tool_calls = records;
        message
    }

    #[test]
    fn emit_uses_caller_supplied_revision_and_state() {
        // 回归钉子：emit 必须用调用方传来的 (revision, state)，不能自己读盘取 revision，
        // 否则绕过 repository 的 per-conversation 锁，并发下会发出偏低的 revision。
        let _: fn(&AppHandle, &str, u64, &AgentTodoState) = emit_chat_todo_state;
    }

    #[test]
    fn old_conversation_json_defaults_todo_state() {
        let conversation: Conversation = serde_json::from_value(json!({
            "id": "conv_test",
            "title": "test",
            "provider_id": "provider",
            "model": "model",
            "messages": [],
            "created_at": 1,
            "updated_at": 1
        }))
        .expect("old conversation should deserialize");
        assert!(conversation.agent_todo_state.items.is_empty());
        assert_eq!(conversation.agent_todo_state.updated_at, 0);
    }

    #[test]
    fn old_item_json_with_legacy_fields_deserializes() {
        let item: AgentTodoItem = serde_json::from_value(json!({
            "id": "a", "content": "old", "status": "completed", "blocked_by": ["b"], "owner": "x"
        }))
        .expect("old item should deserialize");
        assert_eq!(item.blocked_by, vec!["b".to_string()]);
        assert_eq!(item.owner.as_deref(), Some("x"));
    }

    #[test]
    fn write_assigns_positional_ids_and_ignores_legacy_fields() {
        let outcome = write(json!([
            { "id": "a", "content": " First ", "status": "in_progress", "blocks": ["b"], "owner": "me" },
            { "content": "Second", "status": "pending", "description": "  " }
        ]));
        let items = &outcome.state.items;
        assert_eq!(items[0].id, "1");
        assert_eq!(items[0].content, "First");
        assert!(items[0].blocks.is_empty());
        assert!(items[0].owner.is_none());
        assert_eq!(items[1].id, "2");
        assert!(items[1].description.is_none());
        assert_eq!(
            tool_result(&outcome).content,
            "Todo list updated: 0/2 done. In progress: \"First\". Mark it completed as soon as it is done, before starting the next item."
        );
    }

    #[test]
    fn missing_todos_field_is_an_error_not_a_wipe() {
        assert!(apply_todo_write(json!({})).is_err());
        assert!(
            apply_todo_write(json!({ "todos": [{ "content": " ", "status": "pending" }] }))
                .is_err()
        );
    }

    #[test]
    fn extra_in_progress_items_are_demoted_and_reported() {
        let outcome = write(json!([
            { "content": "First", "status": "in_progress" },
            { "content": "Second", "status": "in_progress" }
        ]));
        assert_eq!(outcome.state.items[0].status, AgentTodoStatus::InProgress);
        assert_eq!(outcome.state.items[1].status, AgentTodoStatus::Pending);
        let content = tool_result(&outcome).content;
        assert!(
            content.contains("kept \"First\"; set \"Second\" back to pending"),
            "{content}"
        );
    }

    #[test]
    fn receipt_counts_exclude_cancelled_and_prompt_for_next_step() {
        let outcome = write(json!([
            { "content": "A", "status": "completed" },
            { "content": "B", "status": "cancelled" },
            { "content": "C", "status": "pending" }
        ]));
        let content = tool_result(&outcome).content;
        assert!(
            content.starts_with("Todo list updated: 1/2 done. Nothing is in_progress"),
            "{content}"
        );
    }

    #[test]
    fn fully_resolved_list_is_cleared() {
        let outcome = write(json!([
            { "content": "A", "status": "completed" },
            { "content": "B", "status": "cancelled" }
        ]));
        assert!(outcome.cleared_as_done);
        assert!(outcome.state.items.is_empty());
        let result = tool_result(&outcome);
        assert!(result.content.contains("the list has been cleared"));
        assert_eq!(result.structured_content.as_ref().unwrap()["cleared"], true);
        assert_eq!(
            result.structured_content.as_ref().unwrap()["todoState"]["items"],
            json!([])
        );

        let emptied = write(json!([]));
        assert!(!emptied.cleared_as_done);
        assert_eq!(tool_result(&emptied).content, "Todo list cleared.");
    }

    #[test]
    fn state_is_recomputed_from_the_retained_history() {
        let first = state(vec![item("one", AgentTodoStatus::InProgress)]);
        let later = state(vec![item("two", AgentTodoStatus::Pending)]);
        let messages = vec![
            message(
                "m1",
                None,
                vec![tool_record(
                    "todo_write",
                    ToolCallStatus::Success,
                    Some(first.clone()),
                )],
            ),
            message(
                "m2",
                None,
                vec![tool_record("read", ToolCallStatus::Success, None)],
            ),
            message(
                "m3",
                None,
                vec![tool_record(
                    "todo_write",
                    ToolCallStatus::Error,
                    Some(later.clone()),
                )],
            ),
        ];
        let empty = std::collections::HashMap::new();
        // A failed call does not count; the last successful snapshot wins.
        assert_eq!(state_from_messages(&messages, &empty).items, first.items);
        assert!(state_from_messages(&messages[1..], &empty).items.is_empty());

        // Multi-answer groups: only the selected (or, unselected, the first) arm counts.
        let grouped = vec![
            message(
                "a1",
                Some("g"),
                vec![tool_record(
                    "todo_write",
                    ToolCallStatus::Success,
                    Some(first.clone()),
                )],
            ),
            message(
                "a2",
                Some("g"),
                vec![tool_record(
                    "TaskCreate",
                    ToolCallStatus::Success,
                    Some(later.clone()),
                )],
            ),
        ];
        assert_eq!(state_from_messages(&grouped, &empty).items, first.items);
        let selected = std::collections::HashMap::from([("g".to_string(), "a2".to_string())]);
        assert_eq!(state_from_messages(&grouped, &selected).items, later.items);
    }

    #[test]
    fn reminder_restates_a_list_whose_writes_left_the_context() {
        let current = state(vec![item("Ship it", AgentTodoStatus::InProgress)]);
        let history = vec![
            json!({ "role": "user", "content": "[context summary] ..." }),
            assistant(None),
        ];
        let reminder = reminder_for_next_step(&history, &current).expect("restate");
        let text = reminder["content"].as_str().unwrap();
        assert!(text.starts_with(REMINDER_OPEN_TAG));
        assert!(text.contains("1. [in_progress] Ship it"), "{text}");

        // Once restated, it is not repeated on the next step.
        let mut with_reminder = history.clone();
        with_reminder.push(reminder);
        with_reminder.push(assistant(None));
        assert!(reminder_for_next_step(&with_reminder, &current).is_none());

        // Nothing to restate for an empty list in a short history.
        assert!(reminder_for_next_step(&history, &AgentTodoState::default()).is_none());
    }

    #[test]
    fn final_check_lists_open_items_and_skips_a_resolved_list() {
        let open = state(vec![
            item("Done", AgentTodoStatus::Completed),
            item("Ship it", AgentTodoStatus::InProgress),
            item("Skip", AgentTodoStatus::Cancelled),
        ]);
        let reminder = final_check_reminder(&open).expect("open items");
        assert!(is_reminder_message(&reminder));
        let text = reminder["content"].as_str().unwrap();
        assert!(text.contains("1 todo item(s)"), "{text}");
        assert!(text.contains("2. [in_progress] Ship it"), "{text}");

        let resolved = state(vec![
            item("Done", AgentTodoStatus::Completed),
            item("Skip", AgentTodoStatus::Cancelled),
        ]);
        assert!(final_check_reminder(&resolved).is_none());
        assert!(final_check_reminder(&AgentTodoState::default()).is_none());
    }

    #[test]
    fn reminder_nudges_after_ten_steps_without_a_write() {
        let current = state(vec![item("Ship it", AgentTodoStatus::Pending)]);
        let mut history = vec![
            json!({ "role": "user", "content": "go" }),
            assistant(Some("todo_write")),
        ];
        for _ in 0..9 {
            history.push(assistant(None));
        }
        assert!(reminder_for_next_step(&history, &current).is_none());
        history.push(assistant(None));
        let reminder = reminder_for_next_step(&history, &current).expect("nudge");
        assert!(reminder["content"]
            .as_str()
            .unwrap()
            .contains("hasn't been used recently"));

        // The next nudge waits another ten steps.
        history.push(reminder);
        for _ in 0..9 {
            history.push(assistant(None));
        }
        assert!(reminder_for_next_step(&history, &current).is_none());
        history.push(assistant(None));
        assert!(reminder_for_next_step(&history, &current).is_some());
    }
}
