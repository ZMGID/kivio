# Agent todo runtime research

## Scope

The requested feature is an agent-internal todo list for coding/agent work. It is not a user task-management feature.

## Current Kivio Runtime Findings

### Conversation persistence

`Conversation` is serialized to `conversations/{id}.json` and currently contains messages, active skill/assistant metadata, timestamps, folder/pin metadata, and `context_state`. A new `agent_todo_state` field can be added with `#[serde(default)]` for backward compatibility.

Recommended shape:

```rust
pub struct AgentTodoItem {
    pub id: String,
    pub content: String,
    pub status: AgentTodoStatus,
}

pub struct AgentTodoState {
    pub items: Vec<AgentTodoItem>,
    pub updated_at: i64,
}
```

### Prompt construction

`complete_assistant_reply` builds the system prompt before `build_chat_api_messages`. The todo state should be formatted into a concise system/runtime segment before model messages are assembled, so it is not lost during message compression or UI-only metadata handling.

### Tool execution context

The existing `ToolExecutionContext` already contains `conversation_id`, `run_id`, `message_id`, `generation`, and `round`. This means todo tools do not need model-visible conversation IDs.

### Tool registry

Native tools are listed by `list_native_builtin_tool_defs` and dispatched through `mcp::registry::call_tool`. Current `call_tool` does not receive `ToolExecutionContext`; the chat executor does. For conversation-specific todo writes, the cleaner path is either:

1. Extend registry/native dispatch to receive the execution context.
2. Intercept todo tools in `RegistryToolExecutor.call` before calling the general registry.

Option 1 is cleaner long-term if more native tools need conversation/run context. Option 2 is narrower for MVP.

### UI events

The frontend already listens to `chat-context` and `chat-tool`. Add a `chat-todo` event with `{ conversationId, todoState }`, patch `currentConversation.agent_todo_state`, and render read-only progress.

## External Findings

### MCP

MCP treats tools as model-controlled actions with JSON Schema inputs and tool-call results. It distinguishes tools, resources, and prompts: tools are model-controlled actions; resources are application-controlled context; prompts are user-controlled templates. The schema supports `structuredContent`, `outputSchema`, and tool annotations such as `readOnlyHint`, `destructiveHint`, and `openWorldHint`.

Implication: a todo write/update feature should be modeled as a tool because the model decides when to update working state. The result should include structured todo state. Annotations should mark it non-read-only, non-open-world, and non-destructive.

Sources:

* https://modelcontextprotocol.io/docs/learn/server-concepts
* https://modelcontextprotocol.io/specification/2025-11-25/schema

### OpenAI / Codex

OpenAI function calling docs describe a model tool loop: send tool definitions, receive tool calls, execute app-side logic, return tool output, then let the model continue. Codex documentation publicly describes plan/progress concepts and JSONL event streams that include plan updates. The Agents SDK exposes function tools and run context, but importing it would duplicate Kivio's existing runtime.

Implication: Kivio should keep its current Rust/Tauri tool loop and add native todo tools, not replace the runtime with Agents SDK or Responses-specific orchestration.

Sources:

* https://developers.openai.com/api/docs/guides/function-calling
* https://openai.github.io/openai-agents-js/guides/tools/
* https://developers.openai.com/codex/codex-manual.md

### OpenCode

OpenCode's public `todowrite` prompt describes a structured task list for the current coding session. It recommends proactive use for multi-step tasks, new user instructions, starting work, finishing work, and real-time status updates. It uses statuses including `pending`, `in_progress`, `completed`, and `cancelled`, with exactly one active `in_progress` item.

Implication: Kivio's MVP should adopt the same core behavior but omit `cancelled` unless needed.

Source:

* https://github.com/sst/opencode/blob/dev/packages/opencode/src/tool/todowrite.txt

### Qwen Code

Qwen Code documents `todo_write` as a session-specific tool that the assistant manages automatically during complex multi-step work. It stores todos per session and shows real-time progress. Its item schema includes `content`, `status`, and `activeForm`.

Implication: session/conversation isolation and automatic agent management are established patterns.

Source:

* https://qwenlm.github.io/qwen-code-docs/en/developers/tools/todo-write/

### Claude Code

Anthropic's public Claude Code docs were discoverable, but the specific TodoWrite behavior was not clearly surfaced in official docs during this research pass. Treat Claude Code TodoWrite details as product-observed behavior or community-known behavior unless backed by an official page later.

Source checked:

* https://docs.anthropic.com/en/docs/claude-code/overview

## Recommendation

Build a native, conversation-scoped todo runtime:

1. Add `AgentTodoState` to `Conversation`.
2. Add model-facing native tools:
   * `todo_write`: replace the full list.
   * `todo_update`: update one item status/content by id.
   * Optional later: `todo_clear`, only if needed.
3. Return structured todo state from tool calls.
4. Inject the current todo state into the system prompt as a runtime context segment.
5. Emit `chat-todo` events whenever state changes.
6. Render a compact read-only todo panel in Chat.

Avoid using external SDK/framework orchestration for this feature. The existing runtime already has the necessary tool loop, prompt construction, streaming, persistence, and UI event patterns.

## Open Implementation Choices

* Whether to expose both `todo_write` and `todo_update`, or only a single full-replacement `todo_write` tool.
* Whether to include `cancelled` in MVP. Current requirement says pending/in-progress/completed is enough.
* Where the UI should sit: near the current assistant response, above the composer, or in a collapsible progress area.
