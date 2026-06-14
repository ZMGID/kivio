# Agent todo list runtime

## Goal

Add a conversation-scoped, agent-owned todo list to Kivio Chat so the assistant can track multi-step work, show progress to the user, and carry that working state into later turns.

## What I already know

* This is not a user-facing task manager.
* The todo list should behave like the internal todo/progress tools used by coding agents such as Codex, OpenCode, Qwen Code, and Claude Code-style environments.
* Users should not manually edit todos now or later.
* The todo state must persist as conversation context and be maintained by the agent across turns.
* The existing Kivio Chat runtime already has a model tool loop, tool records, streaming events, and conversation JSON persistence.

## Requirements

* Store the canonical todo state on the conversation, not only in transient tool records.
* Include the current todo state in the next model request so the agent can continue maintaining it.
* Expose model-facing native todo tools for the agent to create/replace/update the todo list.
* Keep the user UI read-only.
* Stream todo updates to the Chat UI while the assistant is running.
* Support only simple statuses for MVP: `pending`, `in_progress`, `completed`.
* Enforce or normalize the invariant that at most one item is `in_progress`.
* Keep old conversation JSON backward-compatible.

## Out of Scope

* Reminders, due dates, recurring tasks, notifications, calendar sync, or external task-manager integration.
* Manual user editing of todo items.
* Project management features such as assignees, priorities, hierarchy, or dependencies.
* Exposing this as a general external MCP server in the MVP.

## Acceptance Criteria

* [ ] A new conversation can receive todo updates from model tool calls.
* [ ] Reloading the conversation preserves its todo state.
* [ ] Sending the next user message injects the previous todo state into model context.
* [ ] The Chat UI displays the current todo list without edit controls.
* [ ] Tool results include structured todo state for traceability.
* [ ] Old conversations without todo fields still load successfully.

## Technical Notes

* `src-tauri/src/chat/types.rs` contains the persistent `Conversation` model.
* `src-tauri/src/chat/commands.rs` builds prompts, lists tools, runs the agent loop, saves conversations, and emits chat events.
* `src-tauri/src/chat/agent/execute.rs` already passes `ToolExecutionContext` with `conversation_id`, `run_id`, and `message_id`.
* `src-tauri/src/mcp/types.rs` already supports tool `annotations` and `output_schema`; `McpToolCallResult` already supports `structured_content`.
* `src/chat/types.ts` mirrors conversation types for the frontend.
* `src/chat/Chat.tsx` already listens for stream, context, and tool events and can be extended with a `chat-todo` event.

## Research Notes

See `research/agent-todo-runtime.md`.
