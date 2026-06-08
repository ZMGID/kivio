# Local Architecture Research

## Current Runtime Shape

Kivio Chat already has a multi-step agent runtime:

- `src-tauri/src/chat/agent/loop_.rs` calls the provider with tools, extracts pending tool calls, emits timeline segments, executes tools, appends tool result messages, and either continues tool rounds or synthesizes a final answer.
- `src-tauri/src/chat/agent/execute.rs` builds `ToolCallRecord`, emits pending/running/final status updates, optionally waits for approval, calls the executor, and returns model-facing tool result text.
- `src-tauri/src/chat/agent/host.rs` abstracts frontend-facing runtime effects: stream deltas, stream done, tool record events, approval requests, and cancellation checks.
- `src-tauri/src/chat/commands.rs` implements `ChatAgentHost` for Tauri events and owns the existing sensitive-tool approval request flow.

The existing approval flow proves Kivio can already block a tool execution inside the same run:

1. Backend inserts a `oneshot::Sender<bool>` into `AppState.pending_chat_tool_approvals`.
2. Backend emits `chat-tool-confirm`.
3. Frontend shows UI and calls `chat_confirm_tool_call`.
4. Backend receives the oneshot result and the same async run continues.

`ask_user` should use the same pattern, but with a richer answer type and inline timeline UI.

## Message and Replay Contracts

Assistant UI messages separate display content from provider replay:

- `ChatMessage.content` / `reasoning` / `segments` / `tool_calls` are visible UI metadata.
- `ChatMessage.api_messages` stores hidden OpenAI-compatible replay messages produced during the answer.
- `ChatMessage.model_messages` stores provider-agnostic replay messages.

This is important because `ask_user` answers should not become ordinary visible `role: user` messages. They should become a tool result tied to the model's original tool call id. The question card itself should be visible as assistant timeline metadata.

## Frontend Rendering Shape

The frontend already renders assistant timeline segments:

- `src/chat/MessageBubble.tsx` sorts `message.segments` and routes `kind === 'tool'` to `ToolCallBlock`.
- `ToolCallBlock` understands generic `ToolCallRecord` status, arguments, result preview, error, artifacts, and structured content.
- Streaming state already carries `streamingToolCalls` and `streamingSegments`.

For MVP there are two viable UI paths:

1. Special-case `ToolCallBlock` when the tool name is `ask_user`.
2. Add a new segment kind, e.g. `question`, and render `AskUserCard` separately.

Recommendation: start with a special `ask_user` renderer inside the existing tool segment path. It minimizes backend and replay risk because the question remains a tool call. If later question cards need non-tool origins, split into a new segment kind.

## Native Tool Registration

Native tools are listed through `src-tauri/src/mcp/types.rs` and `src-tauri/src/mcp/registry.rs`.

`ask_user` should be exposed as a Kivio-owned native tool, not an MCP tool, because:

- It is a client UI control primitive, not an external server capability.
- It must be available even when MCP is disabled.
- It needs a custom frontend event and answer command.
- It should bypass normal sensitive approval policy; the user interaction is the tool execution itself.

Suggested model-facing schema:

```json
{
  "type": "object",
  "properties": {
    "question": { "type": "string" },
    "description": { "type": "string" },
    "selection_mode": { "type": "string", "enum": ["single", "multiple"] },
    "options": {
      "type": "array",
      "minItems": 1,
      "maxItems": 8,
      "items": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "label": { "type": "string" },
          "description": { "type": "string" }
        },
        "required": ["id", "label"]
      }
    },
    "allow_other": { "type": "boolean" },
    "other_label": { "type": "string" }
  },
  "required": ["question", "options"]
}
```

## Recommended Backend Design

Add a typed pending answer:

```rust
pub struct ChatUserQuestionAnswer {
    pub selected_option_ids: Vec<String>,
    pub other_text: Option<String>,
    pub skipped: bool,
}
```

Add `pending_chat_user_questions: Mutex<HashMap<String, oneshot::Sender<ChatUserQuestionAnswer>>>` keyed by `tool_call_id`.

Add a new host method:

```rust
fn request_user_input<'a>(
    &'a self,
    ctx: &'a ToolExecutionContext<'a>,
    record: &'a ToolCallRecord,
    prompt: &'a AskUserPrompt,
) -> AgentHostFuture<'a, AskUserOutcome>;
```

Special-case `ask_user` execution before normal MCP/native registry calls:

1. Validate and normalize arguments.
2. Emit/update a `ToolCallRecord` with structured prompt content and status `pending`.
3. Emit `chat-user-question` with conversation/run/message/tool ids and prompt data.
4. Await answer or cancellation.
5. Emit/update record with status `success`, `skipped`, or `cancelled` and structured answer content.
6. Return a concise model-facing tool result string/JSON.

The returned tool result should include stable JSON:

```json
{
  "answered": true,
  "selection_mode": "single",
  "selected_options": [{"id": "a", "label": "..."}],
  "other_text": null
}
```

## Recommended Frontend Design

Add:

- `ChatUserQuestionPayload` event type in `src/api/tauri.ts`.
- `chatAnswerUserQuestion(toolCallId, answer)` invoke command.
- `pendingUserQuestionsRef` keyed by conversation id or tool call id, included in generating-state calculation.
- `AskUserCard` rendered for `ToolCallBlock` records whose name is `ask_user`.

The inline card should live in the assistant timeline with:

- Header: `Questions` or `Question`.
- Question text.
- Single-select radio-like option rows.
- Multi-select checkbox-like rows.
- Optional `Other` row with text input.
- `Skip` and `Submit`/`Next` actions.
- Completed state showing selected answer(s).

Avoid reusing the centered tool approval modal for this feature. That modal is appropriate for risk approval, not product clarification.

## Risks and Edge Cases

- Parallel tool calls: `ask_user` should probably force the round to wait before executing unrelated side-effect tools. Mixing multiple simultaneous user questions with parallel tool execution is risky for MVP.
- Timeout: current approvals time out after 60 seconds. For user questions, a hard 60-second timeout is hostile. Prefer no timeout while generation remains active, or a much longer timeout with visible cancel.
- App reload: MVP can cancel unresolved in-memory questions after reload. Persisting unresolved suspended runs is a larger runtime feature.
- Provider compatibility: strict providers require a matching tool result for every assistant tool call. Cancellation/skipping must still produce a tool result message.
- Prompt behavior: system prompt must say `ask_user` is for blocking decisions/preferences, not for laziness. The agent should inspect/read/search first when it can derive the answer.
- Multiple questions: MVP should support one question per tool call. Wizard-style `1 of N` can be layered by repeated calls later.
