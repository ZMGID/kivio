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

use serde::Deserialize;
use serde_json::Value;
use tauri::AppHandle;

use crate::chat::types::{
    AgentTodoItem, AgentTodoState, AgentTodoStatus, ChatMessage, ToolCallRecord, ToolCallStatus,
};
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;

pub const TODO_WRITE_TOOL_NAME: &str = "todo_write";

const MAX_TODO_ITEMS: usize = 50;
/// Same thresholds as ZCode / Claude Code (`TURNS_SINCE_WRITE` / `TURNS_BETWEEN_REMINDERS`).
const STEPS_SINCE_WRITE_BEFORE_REMINDER: usize = 10;
const STEPS_BETWEEN_REMINDERS: usize = 10;
/// Marks a reminder message so later steps (and later turns, after replay) can find it.
const REMINDER_OPEN_TAG: &str = "<todo-reminder>";
const REMINDER_CLOSE_TAG: &str = "</todo-reminder>";

const TOOL_DESCRIPTION: &str = "Maintain a short checklist of your own work in this conversation. The user sees it live in the app but cannot edit it.

Use it when the work needs 3 or more distinct steps, when the user gives several tasks at once, or when new requirements arrive mid-task. Skip it for single-step requests, quick questions and plain conversation.

- Send the complete list on every call; it replaces the previous list. Send an empty list to clear it.
- Keep exactly one item in_progress while work remains, and mark an item in_progress before starting it.
- Mark an item completed right after finishing it, and only once it is actually done (including any verification you planned). Do not batch completions.
- If you are blocked, keep the item in_progress and add an item describing the blocker. Mark items that are no longer needed as cancelled.
- Keep items short, specific and actionable. Preserve commands the user gave verbatim.
- When every item is completed or cancelled the list is cleared automatically.
- Do not repeat the list in your reply; the user already sees it.";

/// Model-facing item. Unknown fields (the old `id` / `blocks` / `blocked_by` /
/// `owner`) are ignored so replayed legacy calls still parse.
#[derive(Debug, Deserialize)]
struct TodoWriteItem {
    content: String,
    status: AgentTodoStatus,
    #[serde(default)]
    description: Option<String>,
}

/// `todos` is required: a malformed call must fail, never silently wipe the list.
#[derive(Debug, Deserialize)]
struct TodoWriteArgs {
    todos: Vec<TodoWriteItem>,
}

/// Result of one `todo_write` call.
pub struct TodoToolOutcome {
    /// The state to persist (empty once every item is resolved).
    pub state: AgentTodoState,
    /// Every item was completed or cancelled, so the list was cleared.
    pub cleared_as_done: bool,
    /// The list as written (after corrections), for the receipt.
    written: Vec<AgentTodoItem>,
    /// Corrections applied to the written list, reported back to the model.
    notes: Vec<String>,
}

pub fn is_agent_todo_tool_name(name: &str) -> bool {
    name == TODO_WRITE_TOOL_NAME
}

pub fn append_tool_definitions(tools: &mut Vec<ChatToolDefinition>) {
    let tool = todo_write_tool();
    if !tools
        .iter()
        .any(|existing| existing.openai_tool_name() == tool.openai_tool_name())
    {
        tools.push(tool);
    }
}

pub fn todo_write_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__todo_write".to_string(),
        name: TODO_WRITE_TOOL_NAME.to_string(),
        description: TOOL_DESCRIPTION.to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "maxItems": MAX_TODO_ITEMS,
                    "description": "The complete updated list, in order",
                    "items": todo_item_schema()
                }
            },
            "required": ["todos"],
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: Some(serde_json::json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "openWorldHint": false
        })),
        output_schema: Some(todo_output_schema()),
    }
}

pub fn apply_todo_write(arguments: Value) -> Result<TodoToolOutcome, String> {
    let args: TodoWriteArgs = serde_json::from_value(arguments)
        .map_err(|err| format!("Invalid todo_write arguments: {err}"))?;
    if args.todos.len() > MAX_TODO_ITEMS {
        return Err(format!(
            "Too many todos ({}); keep the list at {MAX_TODO_ITEMS} items or fewer",
            args.todos.len()
        ));
    }
    let mut written = Vec::with_capacity(args.todos.len());
    for (index, item) in args.todos.into_iter().enumerate() {
        let content = item.content.trim().to_string();
        if content.is_empty() {
            return Err(format!("Todo item {} content cannot be empty", index + 1));
        }
        written.push(AgentTodoItem {
            // Positional ids: the model never references them; they only key the UI rows.
            id: (index + 1).to_string(),
            content,
            description: item
                .description
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
            status: item.status,
            ..Default::default()
        });
    }

    // At most one in_progress item. Accept the write and say what changed instead of
    // failing it: ZCode found that rejecting the call derails the rest of the turn.
    let mut notes = Vec::new();
    let mut active: Option<String> = None;
    let mut demoted = Vec::new();
    for item in &mut written {
        if item.status != AgentTodoStatus::InProgress {
            continue;
        }
        if active.is_none() {
            active = Some(item.content.clone());
        } else {
            item.status = AgentTodoStatus::Pending;
            demoted.push(format!("\"{}\"", item.content));
        }
    }
    if let (Some(active), false) = (&active, demoted.is_empty()) {
        notes.push(format!(
            "Only one item can be in_progress: kept \"{active}\"; set {} back to pending.",
            demoted.join(", ")
        ));
    }

    let cleared_as_done = !written.is_empty() && written.iter().all(is_resolved);
    let items = if cleared_as_done {
        Vec::new()
    } else {
        written.clone()
    };
    Ok(TodoToolOutcome {
        state: stamped(items),
        cleared_as_done,
        written,
        notes,
    })
}

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

/// Short receipt for the model: the list itself is already in its own call
/// arguments, so echoing it back would only double the tokens. `todoState` in the
/// structured content is the persisted state that the UI and rewind read.
pub fn tool_result(outcome: &TodoToolOutcome) -> McpToolCallResult {
    let structured = serde_json::json!({
        "todoState": outcome.state,
        "cleared": outcome.cleared_as_done,
    });
    let mut content = receipt_line(outcome);
    for note in &outcome.notes {
        content.push('\n');
        content.push_str(note);
    }
    McpToolCallResult {
        content,
        is_error: false,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
        follow_up_user_messages: Vec::new(),
    }
}

fn receipt_line(outcome: &TodoToolOutcome) -> String {
    let written = &outcome.written;
    if written.is_empty() {
        return "Todo list cleared.".to_string();
    }
    if outcome.cleared_as_done {
        return format!(
            "All {} todos are completed or cancelled; the list has been cleared.",
            written.len()
        );
    }
    let completed = written
        .iter()
        .filter(|item| item.status == AgentTodoStatus::Completed)
        .count();
    let total = written
        .iter()
        .filter(|item| item.status != AgentTodoStatus::Cancelled)
        .count();
    match written
        .iter()
        .find(|item| item.status == AgentTodoStatus::InProgress)
    {
        Some(item) => format!(
            "Todo list updated: {completed}/{total} done. In progress: \"{}\". Mark it completed as soon as it is done, before starting the next item.",
            item.content
        ),
        None => format!(
            "Todo list updated: {completed}/{total} done. Nothing is in_progress; mark the next item in_progress before working on it."
        ),
    }
}

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
pub(crate) fn latest_recorded_state(records: &[ToolCallRecord]) -> Option<AgentTodoState> {
    records
        .iter()
        .rev()
        .filter(|record| record.status == ToolCallStatus::Success)
        .find_map(|record| {
            let todo = record.structured_content.as_ref()?.get("todoState")?;
            serde_json::from_value::<AgentTodoState>(todo.clone()).ok()
        })
}

/// The reminder to append before the next model step, if any.
///
/// - The list has items but no `todo_write` call or reminder is visible in
///   `messages` (compaction summarized them, context was cleared, history was
///   edited): restate the list so the model does not lose it.
/// - Otherwise, after [`STEPS_SINCE_WRITE_BEFORE_REMINDER`] model steps without a
///   write and [`STEPS_BETWEEN_REMINDERS`] since the last reminder: a gentle nudge.
///
/// Reminders are user messages appended at the end of the request, so they never
/// change the cached prefix.
pub(crate) fn reminder_for_next_step(
    messages: &[Value],
    current: &AgentTodoState,
) -> Option<Value> {
    let mut steps = 0usize;
    let mut since_write = None;
    let mut since_reminder = None;
    for message in messages.iter().rev() {
        if since_reminder.is_none() && is_reminder_message(message) {
            since_reminder = Some(steps);
        }
        if message["role"] != "assistant" {
            continue;
        }
        if since_write.is_none() && calls_todo_write(message) {
            since_write = Some(steps);
        }
        steps += 1;
        if since_write.is_some() && since_reminder.is_some() {
            break;
        }
    }
    let list = format_items(&current.items);
    if since_write.is_none() && since_reminder.is_none() && !current.items.is_empty() {
        return Some(reminder_message(&format!(
            "Your todo list (current and authoritative; earlier todo_write calls are no longer in context):\n{list}\nKeep it updated with todo_write as you work. Do not mention this reminder to the user."
        )));
    }
    if since_write.unwrap_or(steps) < STEPS_SINCE_WRITE_BEFORE_REMINDER
        || since_reminder.unwrap_or(steps) < STEPS_BETWEEN_REMINDERS
    {
        return None;
    }
    let mut body = "The todo_write tool hasn't been used recently. If you're working on multi-step work that would benefit from tracking progress, update it; if the list is stale or no longer matches the current work, rewrite it or clear it. This is only a gentle reminder: ignore it if it isn't relevant, and do not mention it to the user.".to_string();
    if !current.items.is_empty() {
        body.push_str("\n\nCurrent todo list:\n");
        body.push_str(&list);
    }
    Some(reminder_message(&body))
}

/// The reminder to append when the model gives its final answer while the list it
/// wrote in this run still has pending or in_progress items, if any. The loop sends
/// it at most once per run, so a list the model deliberately leaves open costs one
/// extra step, not a loop.
pub(crate) fn final_check_reminder(current: &AgentTodoState) -> Option<Value> {
    let open = current
        .items
        .iter()
        .filter(|item| !is_resolved(item))
        .count();
    if open == 0 {
        return None;
    }
    Some(reminder_message(&format!(
        "You are about to end your turn, but {open} todo item(s) are still pending or in_progress:\n{}\nCall todo_write now: mark finished items completed and items you will not do cancelled. Keep an item open only if work on it really remains, and say so. Then end with one short closing sentence; do not repeat your previous answer. Do not mention this reminder to the user.",
        format_items(&current.items)
    )))
}

fn reminder_message(body: &str) -> Value {
    serde_json::json!({
        "role": "user",
        "content": format!("{REMINDER_OPEN_TAG}\n{body}\n{REMINDER_CLOSE_TAG}"),
    })
}

/// Whether `message` is a todo reminder appended by the runtime, not user input.
pub(crate) fn is_reminder_message(message: &Value) -> bool {
    message["role"] == "user"
        && message["content"]
            .as_str()
            .is_some_and(|content| content.starts_with(REMINDER_OPEN_TAG))
}

fn calls_todo_write(message: &Value) -> bool {
    message["tool_calls"].as_array().is_some_and(|calls| {
        calls.iter().any(|call| {
            call["function"]["name"].as_str().is_some_and(|name| {
                crate::mcp::types::canonical_tool_name(name) == TODO_WRITE_TOOL_NAME
            })
        })
    })
}

fn is_resolved(item: &AgentTodoItem) -> bool {
    matches!(
        item.status,
        AgentTodoStatus::Completed | AgentTodoStatus::Cancelled
    )
}

fn stamped(items: Vec<AgentTodoItem>) -> AgentTodoState {
    AgentTodoState {
        items,
        updated_at: chrono::Local::now().timestamp(),
    }
}

fn format_items(items: &[AgentTodoItem]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            format!(
                "{}. [{}] {}",
                index + 1,
                status_name(&item.status),
                item.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_name(status: &AgentTodoStatus) -> &'static str {
    match status {
        AgentTodoStatus::Pending => "pending",
        AgentTodoStatus::InProgress => "in_progress",
        AgentTodoStatus::Completed => "completed",
        AgentTodoStatus::Cancelled => "cancelled",
    }
}

fn status_schema() -> Value {
    serde_json::json!({
        "type": "string",
        "enum": ["pending", "in_progress", "completed", "cancelled"]
    })
}

fn todo_item_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "string",
                "minLength": 1,
                "maxLength": 240,
                "description": "Short, actionable description of the step"
            },
            "status": status_schema(),
            "description": {
                "type": "string",
                "maxLength": 2000,
                "description": "Optional extra detail; usually omit"
            }
        },
        "required": ["content", "status"],
        "additionalProperties": false
    })
}

fn todo_output_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "todoState": {
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "content": { "type": "string" },
                                "status": status_schema(),
                                "description": { "type": "string" }
                            },
                            "required": ["id", "content", "status"]
                        }
                    },
                    "updated_at": { "type": "integer" }
                },
                "required": ["items", "updated_at"]
            },
            "cleared": {
                "type": "boolean",
                "description": "Every item was completed or cancelled, so the list was cleared"
            }
        },
        "required": ["todoState"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
