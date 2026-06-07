use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;

use crate::chat::types::{AgentTodoItem, AgentTodoState, AgentTodoStatus};
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;

pub const TODO_WRITE_TOOL_NAME: &str = "todo_write";
pub const TODO_UPDATE_TOOL_NAME: &str = "todo_update";

#[derive(Debug, Deserialize)]
struct TodoWriteArgs {
    #[serde(default)]
    todos: Vec<AgentTodoItem>,
}

#[derive(Debug, Deserialize)]
struct TodoUpdateArgs {
    id: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    status: Option<AgentTodoStatus>,
}

pub fn is_agent_todo_tool_name(name: &str) -> bool {
    matches!(name, TODO_WRITE_TOOL_NAME | TODO_UPDATE_TOOL_NAME)
}

pub fn append_tool_definitions(tools: &mut Vec<ChatToolDefinition>) {
    for tool in tool_definitions() {
        if !tools
            .iter()
            .any(|existing| existing.openai_tool_name() == tool.openai_tool_name())
        {
            tools.push(tool);
        }
    }
}

pub fn tool_definitions() -> Vec<ChatToolDefinition> {
    vec![todo_write_tool(), todo_update_tool()]
}

pub fn todo_write_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__todo_write".to_string(),
        name: TODO_WRITE_TOOL_NAME.to_string(),
        description: "Replace the assistant's internal todo list for this conversation. Use for multi-step work progress; users cannot edit this list.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "maxItems": 50,
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

pub fn todo_update_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__todo_update".to_string(),
        name: TODO_UPDATE_TOOL_NAME.to_string(),
        description: "Update one item in the assistant's internal todo list by id. Use when work starts, switches, or completes.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 80,
                    "description": "Existing todo item id"
                },
                "content": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 240
                },
                "status": status_schema()
            },
            "required": ["id"],
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

pub fn apply_tool(
    current: &AgentTodoState,
    tool_name: &str,
    arguments: Value,
) -> Result<AgentTodoState, String> {
    match tool_name {
        TODO_WRITE_TOOL_NAME => apply_todo_write(arguments),
        TODO_UPDATE_TOOL_NAME => apply_todo_update(current, arguments),
        other => Err(format!("Unknown todo tool: {other}")),
    }
}

pub fn apply_todo_write(arguments: Value) -> Result<AgentTodoState, String> {
    let args: TodoWriteArgs = serde_json::from_value(arguments)
        .map_err(|err| format!("Invalid todo_write arguments: {err}"))?;
    normalized_state(args.todos, None)
}

pub fn apply_todo_update(
    current: &AgentTodoState,
    arguments: Value,
) -> Result<AgentTodoState, String> {
    let args: TodoUpdateArgs = serde_json::from_value(arguments)
        .map_err(|err| format!("Invalid todo_update arguments: {err}"))?;
    let id = args.id.trim();
    if id.is_empty() {
        return Err("todo_update id is empty".to_string());
    }
    if args.content.is_none() && args.status.is_none() {
        return Err("todo_update requires content or status".to_string());
    }

    let mut items = current.items.clone();
    let item = items
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("Todo item not found: {id}"))?;
    if let Some(content) = args.content {
        item.content = content;
    }
    if let Some(status) = args.status {
        item.status = status;
    }
    let preferred_in_progress =
        matches!(&item.status, AgentTodoStatus::InProgress).then(|| id.to_string());
    normalized_state(items, preferred_in_progress.as_deref())
}

pub fn format_prompt(state: &AgentTodoState, language: &str, todo_tools_available: bool) -> String {
    let current = format_state_lines(state);
    if language.starts_with("zh") {
        let tool_hint = if todo_tools_available {
            "你可以使用 todo_write 和 todo_update 维护这个列表。"
        } else {
            "当前请求没有可用的 todo 工具；只能把它作为上下文参考。"
        };
        format!(
            "Agent todo list（内部工作状态）：这个 todo list 由助手自己维护，用户不能手动编辑。{tool_hint} 对复杂、多步骤、需要持续跟进的任务，应保持简洁、可执行的条目；开始或切换任务时标记 in_progress，完成后标记 completed，最多只能有一个 in_progress。不要告诉用户他们可以编辑 todo。\n\n当前 todo 状态：\n{current}"
        )
    } else {
        let tool_hint = if todo_tools_available {
            "Use todo_write and todo_update to maintain it."
        } else {
            "Todo tools are unavailable for this request; use it as context only."
        };
        format!(
            "Agent todo list (internal working state): this list is owned by the assistant; the user cannot edit it manually. {tool_hint} For complex, multi-step, or continuing work, keep concise actionable items; mark one item in_progress when starting or switching work, mark items completed when done, and keep at most one in_progress item. Do not tell the user they can edit todos.\n\nCurrent todo state:\n{current}"
        )
    }
}

pub fn tool_result(state: &AgentTodoState) -> McpToolCallResult {
    let structured = serde_json::json!({ "todoState": state });
    McpToolCallResult {
        content: format!("Todo list updated.\n\n{}", format_state_lines(state)),
        is_error: false,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
    }
}

fn normalized_state(
    items: Vec<AgentTodoItem>,
    preferred_in_progress_id: Option<&str>,
) -> Result<AgentTodoState, String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for item in items {
        let id = item.id.trim().to_string();
        let content = item.content.trim().to_string();
        if id.is_empty() {
            return Err("Todo item id cannot be empty".to_string());
        }
        if content.is_empty() {
            return Err(format!("Todo item `{id}` content cannot be empty"));
        }
        if !seen.insert(id.clone()) {
            return Err(format!("Todo item id must be unique: {id}"));
        }
        normalized.push(AgentTodoItem {
            id,
            content,
            status: item.status,
        });
    }

    let mut active_seen = false;
    for item in &mut normalized {
        let keep_active = if item.status == AgentTodoStatus::InProgress {
            match preferred_in_progress_id {
                Some(preferred) => item.id == preferred,
                None => !active_seen,
            }
        } else {
            false
        };
        if keep_active {
            active_seen = true;
        } else if item.status == AgentTodoStatus::InProgress {
            item.status = AgentTodoStatus::Pending;
        }
    }

    Ok(AgentTodoState {
        items: normalized,
        updated_at: chrono::Local::now().timestamp(),
    })
}

fn format_state_lines(state: &AgentTodoState) -> String {
    if state.items.is_empty() {
        return "- No current todos.".to_string();
    }
    state
        .items
        .iter()
        .map(|item| {
            format!(
                "- [{}] {}: {}",
                status_name(&item.status),
                item.id,
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
    }
}

fn todo_item_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "minLength": 1,
                "maxLength": 80,
                "description": "Stable id for future todo_update calls"
            },
            "content": {
                "type": "string",
                "minLength": 1,
                "maxLength": 240,
                "description": "Concise actionable task"
            },
            "status": status_schema()
        },
        "required": ["id", "content", "status"],
        "additionalProperties": false
    })
}

fn status_schema() -> Value {
    serde_json::json!({
        "type": "string",
        "enum": ["pending", "in_progress", "completed"]
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
                        "items": todo_item_schema()
                    },
                    "updated_at": {
                        "type": "integer"
                    }
                },
                "required": ["items", "updated_at"],
                "additionalProperties": false
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

    #[test]
    fn old_conversation_json_defaults_todo_state() {
        let json = serde_json::json!({
            "id": "conv_test",
            "title": "test",
            "provider_id": "provider",
            "model": "model",
            "messages": [],
            "created_at": 1,
            "updated_at": 1
        });
        let conversation: Conversation =
            serde_json::from_value(json).expect("old conversation should deserialize");

        assert!(conversation.agent_todo_state.items.is_empty());
        assert_eq!(conversation.agent_todo_state.updated_at, 0);
    }

    #[test]
    fn write_normalizes_to_single_in_progress_item() {
        let state = apply_todo_write(serde_json::json!({
            "todos": [
                { "id": "a", "content": "First", "status": "in_progress" },
                { "id": "b", "content": "Second", "status": "in_progress" }
            ]
        }))
        .expect("todo write should succeed");

        assert_eq!(state.items[0].status, AgentTodoStatus::InProgress);
        assert_eq!(state.items[1].status, AgentTodoStatus::Pending);
    }

    #[test]
    fn update_prefers_new_in_progress_item() {
        let current = apply_todo_write(serde_json::json!({
            "todos": [
                { "id": "a", "content": "First", "status": "in_progress" },
                { "id": "b", "content": "Second", "status": "pending" }
            ]
        }))
        .expect("todo write should succeed");

        let state = apply_todo_update(
            &current,
            serde_json::json!({ "id": "b", "status": "in_progress" }),
        )
        .expect("todo update should succeed");

        assert_eq!(state.items[0].status, AgentTodoStatus::Pending);
        assert_eq!(state.items[1].status, AgentTodoStatus::InProgress);
    }
}
