# Agent Ask User Clarification

## Goal

Add an interactive clarification capability to Kivio Chat: the model can call an `ask_user` tool during an agent run, Kivio renders an inline question card in the assistant timeline, the user selects one or more answers or enters custom text, and the answer is returned to the model as a tool result so the same run can continue.

## What I Already Know

- The current Plan mode can encourage the assistant to ask clarifying questions in natural language, but it cannot block the same agent run and resume with structured answers.
- The target UX should match Cursor / Codex style inline question cards in the message timeline, not the current centered modal used for sensitive tool approvals.
- MVP question capabilities should include single choice, multiple choice, and an `Other` free-text path.
- Kivio already has a tool loop that can block on user action for sensitive tool approval.
- Tool records are rendered as assistant timeline segments and persisted on the assistant message.
- Hidden provider replay messages are stored separately in assistant message metadata as `api_messages` and `model_messages`, which is the right place for `role: tool` results.
- A peer PRD proposes a useful implementation split: a dedicated `ask_user.rs` backend module, a `phase`-based `structured_content`, an inline `AskUserBlock`, a longer wait timeout than tool approval, explicit serial execution, and tests for schema/answer validation.

## Research References

- [research/local-architecture.md](research/local-architecture.md) - Local code paths and reusable mechanisms for implementing `ask_user`.
- [research/protocol-reference.md](research/protocol-reference.md) - External protocol/product patterns for tool-call results and elicitation-style user input.

## Requirements

- Expose an `ask_user` model-facing tool in Chat agent runs.
- Render an inline assistant timeline card when the model calls `ask_user`.
- Allow single-select, multi-select, and custom free-text answers.
- Support a `questions` array in the tool schema so one tool call can ask 1-4 tightly related questions.
- Resume the same backend run after the user answers.
- Feed the answer back to the model as a tool result associated with the original tool call id.
- Persist enough metadata so completed questions remain visible after reload and provider-compatible replay remains valid.
- Keep existing sensitive tool approval behavior intact.
- Cancel/stop should unblock a pending question and produce a deterministic cancellation/skipped result.

## Recommended MVP

Implement `ask_user` as a native Kivio tool with special execution handling inside the agent runtime.

This keeps it aligned with normal tool-calling providers: the model emits a tool call, the app executes it by asking the user, then the app appends a tool result and continues the tool loop. The frontend should render the question through the existing timeline segment system, but with a dedicated question card instead of the generic tool call block.

Adopt the peer PRD's multi-question data model, but keep the first implementation ergonomically simple: one inline card can show all questions and submit once. Cursor/Codex-style pagination (`1 of N`) can be added later without changing the backend transcript shape.

## Tool Schema

Create `src-tauri/src/chat/ask_user.rs` for the tool definition, argument normalization, answer validation, and result formatting.

Model-facing tool name: `ask_user`.

Recommended input schema:

```json
{
  "type": "object",
  "properties": {
    "title": { "type": "string", "maxLength": 120 },
    "questions": {
      "type": "array",
      "minItems": 1,
      "maxItems": 4,
      "items": {
        "type": "object",
        "properties": {
          "id": { "type": "string", "minLength": 1, "maxLength": 40 },
          "prompt": { "type": "string", "minLength": 1, "maxLength": 500 },
          "options": {
            "type": "array",
            "minItems": 2,
            "maxItems": 6,
            "items": {
              "type": "object",
              "properties": {
                "id": { "type": "string", "minLength": 1, "maxLength": 40 },
                "label": { "type": "string", "minLength": 1, "maxLength": 200 },
                "description": { "type": "string", "maxLength": 500 }
              },
              "required": ["id", "label"],
              "additionalProperties": false
            }
          },
          "allow_multiple": { "type": "boolean", "default": false },
          "allow_custom": { "type": "boolean", "default": false }
        },
        "required": ["id", "prompt", "options"],
        "additionalProperties": false
      }
    }
  },
  "required": ["questions"],
  "additionalProperties": false
}
```

Tool metadata:

- `source: "native"`
- `sensitive: false`
- `annotations.readOnlyHint: true`
- bypasses ordinary tool approval because the user's answer is the execution approval/input
- allowed in Plan mode
- excluded from parallel tool execution

## Structured Content

Persist prompt and answer state in `ToolCallRecord.structured_content`:

```json
{
  "askUser": {
    "phase": "awaiting",
    "title": "Questions",
    "questions": [
      {
        "id": "ui_location",
        "prompt": "Where should the question options UI live?",
        "options": [
          { "id": "inline", "label": "Inline in assistant timeline" },
          { "id": "modal", "label": "Centered modal" }
        ],
        "allow_multiple": false,
        "allow_custom": true
      }
    ],
    "answers": {}
  }
}
```

Valid `phase` values:

- `awaiting` - UI is interactive and backend is waiting
- `answered` - user submitted a valid answer
- `skipped` - user explicitly skipped
- `timeout` - wait exceeded the runtime timeout
- `cancelled` - generation was stopped/cancelled

Tool result text for provider replay should be stable JSON:

```json
{
  "phase": "answered",
  "answers": {
    "ui_location": {
      "selected_option_ids": ["inline"],
      "custom_text": null
    }
  }
}
```

## Backend Plan

- Add `src-tauri/src/chat/ask_user.rs` and export it from `src-tauri/src/chat/mod.rs`.
- Add a pending map to `AppState`, keyed by `tool_call_id`: `pending_chat_user_prompts: Mutex<HashMap<String, oneshot::Sender<AskUserResponseResult>>>`.
- Extend `AgentHost` with a `request_user_response` method that receives `ToolExecutionContext`, the current `ToolCallRecord`, and the normalized prompt payload.
- Implement `ChatAgentHost::request_user_response` in `commands.rs` using a Tauri event plus oneshot wait.
- Add a Tauri command such as `chat_submit_user_choice(tool_call_id, answers)` to resolve the pending prompt after backend answer validation.
- Register the native tool whenever the provider supports tools. It should still be available when user-configured MCP is off.
- Allow `ask_user` through Plan-mode tool filtering.
- Add `ask_user` to disabled builtin feedback so the model receives a clear message if it calls it when unavailable.
- Add `ask_user` to `builtin_tool_bypasses_approval`.
- Exclude `ask_user` from `tool_call_parallel_eligible` and flush any pending parallel batch before executing it.
- On cancel/stop, remove the pending entry and return a cancelled/skipped tool result so strict provider replay remains valid.
- Use a longer wait than sensitive approval. Recommendation: 10 minutes for MVP, with cancellation always available.

## Frontend Plan

- Add `ChatUserPromptPayload`, `AskUserQuestion`, `AskUserAnswer`, and `AskUserStructuredContent` types in `src/chat/types.ts` and `src/api/tauri.ts`.
- Add `api.onChatUserPrompt` and `api.chatSubmitUserChoice`.
- In `Chat.tsx`, listen for `chat-user-prompt`, merge the prompt into the matching streaming tool record, and keep the conversation marked as generating while the backend waits.
- Add `src/chat/AskUserBlock.tsx`.
- In `ToolCallBlock.tsx`, special-case `ask_user` similarly to the existing todo rendering path.
- Keep the centered tool approval modal only for sensitive tool approval; `ask_user` must render inline in the assistant timeline.
- Completed/reloaded cards are read-only and show the selected labels/custom text summary.

AskUserBlock UI states:

| Phase | UI |
|---|---|
| `awaiting` | Title, question prompts, single-select radio rows, multi-select checkbox rows, optional Other text input, Skip and Submit buttons |
| `answered` | Read-only selected answer summary |
| `skipped` | Read-only skipped note |
| `timeout` | Read-only timeout note |
| `cancelled` | Read-only cancelled note |

Validation rules:

- Each question must have at least one selected option unless skipped.
- Single-select questions accept exactly one selected option.
- Multi-select questions accept one or more selected options.
- `custom_text` is accepted only when `allow_custom` is true.
- If the user chooses the custom/Other path, non-empty custom text is required.

## Prompt Guidance

Inject concise system prompt guidance when `ask_user` is available:

- Use `ask_user` for blocking product decisions, unclear requirements, or meaningful user preferences.
- Do not ask questions that can be answered by reading files, searching, inspecting context, or using available tools.
- Prefer 1-3 high-value questions per call.
- Make options concrete, mutually understandable, and actionable.
- Use custom input only when realistic options may not cover the user's intent.

Plan mode should explicitly say structured clarifications should use `ask_user` instead of only writing questions in assistant text.

## Acceptance Criteria

- [ ] A model can call `ask_user` with one question and 2-6 options.
- [ ] A model can call `ask_user` with 1-4 questions in one prompt payload.
- [ ] The frontend shows an inline question card inside the current assistant streaming message.
- [ ] The user can answer with exactly one option for single-select questions.
- [ ] The user can answer with multiple options for multi-select questions.
- [ ] The user can choose/type `Other` and submit custom text.
- [ ] The agent run resumes without requiring a new user message.
- [ ] The final stored assistant message includes the question segment, answer state, and hidden model/tool replay messages.
- [ ] Reloading the conversation shows the answered card as completed.
- [ ] Cancelling generation while waiting marks the question cancelled/skipped and releases the active run.

## Out of Scope

- Multi-question wizard pagination in the MVP; the payload can support multiple questions, but UI may render them in one card.
- Cross-run persistence of unanswered questions after app restart.
- Arbitrary JSON-schema forms beyond choices plus custom text.
- Voice input for question answers.
- Remote MCP server initiated elicitation in this task, unless it is routed through the same internal UI primitive later.
- User-authored `/ask` slash command.
- Lens/translator ask-user UI.

## Test Plan

Rust unit tests:

- `ask_user` schema normalization rejects missing `questions`, too many questions, too few options, duplicate question ids, and duplicate option ids.
- Answer validation covers single-select, multi-select, skipped, custom text allowed/disallowed, and missing custom text.
- Plan-mode filtering keeps `ask_user` available.
- Tool approval bypass does not trigger sensitive approval for `ask_user`.
- Cancel/timeout removes the pending map entry and yields a provider-compatible tool result.

Frontend/manual checks:

- Act mode ambiguous request -> inline question card -> answer -> same run continues.
- Plan mode investigation -> `ask_user` confirms scope -> final plan incorporates answer.
- Multi-select plus custom text.
- Stop while waiting -> card becomes cancelled and input state recovers.
- Reload conversation -> answered card is visible and read-only.

Quality commands:

- `cargo test --manifest-path src-tauri/Cargo.toml chat::ask_user`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `npm run typecheck`
- `npm run lint`

## Technical Notes

- `src-tauri/src/chat/agent/loop_.rs` owns the tool loop and segment emission.
- `src-tauri/src/chat/agent/execute.rs` owns tool execution, approval, status transitions, and tool result content.
- `src-tauri/src/chat/agent/host.rs` is the clean abstraction point for a new `request_user_input` method.
- `src-tauri/src/chat/commands.rs` implements `ChatAgentHost`, Tauri events, and pending user-action oneshot maps.
- `src-tauri/src/state.rs` already has `pending_chat_tool_approvals`; `ask_user` likely needs a parallel `pending_chat_user_questions`.
- `src-tauri/src/chat/todo.rs` is a good reference for a small native agent tool module with tool definitions, prompt text, result formatting, and unit tests.
- `src/chat/MessageBubble.tsx`, `src/chat/ToolCallBlock.tsx`, and `src/chat/segments.ts` are the main frontend rendering points.
- `src/api/tauri.ts` and `src/chat/types.ts` need payload/type additions.
- `.trellis/spec/backend/agent-runtime.md` should be updated after implementation to document schema, event names, serial execution, cancellation semantics, and replay contracts.
