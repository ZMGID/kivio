# Chat Agent Runtime

## Scenario: Per-Round Tool Scheduling

### 1. Scope / Trigger

- Trigger: changes under `src-tauri/src/chat/agent/**` that alter how model-emitted tool calls are matched, executed, recorded, or replayed.
- The Chat agent loop is a Rust-native model-step loop. Provider adapters may parse multiple tool calls from one assistant response, but local execution concurrency is controlled by the runtime scheduler.

### 2. Signatures

- `run_agent_loop(config, host, executor) -> Result<AgentRunResult, String>`
- `execute_tool_call(host, executor, settings, ctx, tool, call, skill_cache) -> (ToolCallRecord, String)`
- `validate_tool_arguments(tool, arguments) -> Result<(), String>`
- `ToolExecutor::call(ctx, tool, arguments, skill_cache) -> ToolExecutorFuture`
- `skill_cache` is optional so non-skill tools can run without borrowing the per-run `SkillRunCache`.
- `ChatToolDefinition` must carry `input_schema`, optional MCP `annotations`, optional MCP `output_schema`, and `sensitive`.
- `ToolCallRecord` must carry lifecycle fields plus optional `trace_id`, `span_id`, and `structured_content`.

### 3. Contracts

- Record one assistant message containing all requested `tool_calls` before appending any tool result messages.
- Append generated tool result messages as OpenAI-compatible objects:

```json
{ "role": "tool", "tool_call_id": "<original id>", "content": "<tool output>" }
```

- Tool result messages must remain in the same order as the model's original `PendingToolCall` list, even if execution completes out of order.
- Every executable tool still emits lifecycle records through `AgentHost::emit_tool_record`: pending, running, then final success/error/skipped/cancelled.
- Validate every executable tool call against `ChatToolDefinition.input_schema` before approval and before `ToolExecutor::call`. Validation failure returns an error tool result and must not ask for approval or invoke the executor.
- Approval-gated tools must be serial. Do not start execution before `AgentHost::request_tool_approval` resolves.
- The native parallel-safe set is intentionally narrow: `native:web_search`, `native:web_fetch`, and `native:read_file`, and only when `tool_requires_approval` returns false.
- MCP tools are serial by default. A MCP tool may join a parallel batch only when it has explicit `annotations.readOnlyHint == true`, no `destructiveHint == true`, and `tool_requires_approval` returns false.
- MCP approval/sensitivity must prefer tool metadata over name guessing: `destructiveHint == true`, `openWorldHint == true`, or `readOnlyHint == false` imply sensitive/confirmation behavior under confirm policies; `readOnlyHint == true` allows auto-approval for trusted non-sensitive tools. User-selected `approval_policy == "auto"` still bypasses approval prompts but must not make non-read-only MCP tools parallel.
- Preserve MCP metadata across all backend/frontend boundaries: `annotations`, `outputSchema`, and tool result `structuredContent` must not be dropped. When a MCP result includes `structuredContent`, persist it on `ToolCallRecord` and include it in the model-facing tool content unless the text result already contains the same JSON.
- Tool records emitted from the agent loop should include `trace_id = run_id` and a deterministic `span_id` such as `tool_round_<round>_<tool_call_id>` so future tracing/export can correlate events without changing storage shape.
- Serial by default: writes/edits, command execution, `run_python`, Skill runtime tools, Mixer image generation, memory mutation, arbitrary MCP tools, unknown calls, and invalid arguments.
- Keep `SkillRunCache` on the serial path unless it is redesigned as a shared concurrency-safe cache with tests.
- Keep timeout and cancellation inside `execute_tool_call`; schedulers should call this helper rather than duplicating lifecycle logic.
- If generation is cancelled during a tool round, stop launching any unstarted calls in that round. Append ordered `cancelled` tool result messages and records for every unstarted call so provider replay remains valid.
- A cancelled tool round that already produced tool transcript messages should return an `AgentRunResult` with stopped content instead of bubbling `Err("cancelled")`, allowing the assistant message and tool records to persist.

### 4. Validation & Error Matrix

| Condition | Runtime behavior |
|---|---|
| Unknown enabled tool name | Emit an error `ToolCallRecord`; append a matching `role: tool` error message. |
| Disabled built-in requested through fallback markup | Append hidden model feedback; do not emit a visible tool record. |
| Invalid tool argument JSON | Emit an error `ToolCallRecord`; append retry guidance as the tool result; do not request approval or call the executor. |
| Tool arguments violate declared schema | Emit an error `ToolCallRecord`; append schema retry guidance; do not request approval or call the executor. |
| MCP `annotations.readOnlyHint == true` and trusted/non-sensitive | May skip approval under confirm policies and may parallelize if no other risk hints are present. |
| MCP `destructiveHint == true`, `openWorldHint == true`, or `readOnlyHint == false` | Treat as sensitive under confirm policies; keep serial even if approval is skipped by `"auto"`. |
| Tool requires approval | Execute serially after approval; skipped result if denied. |
| MCP result includes `structuredContent` | Preserve it on the tool record, emit it through `chat-tool`, and include it in replay content without duplicating identical text JSON. |
| Generation cancelled while a tool is running | Mark active and unstarted tool records cancelled where possible, append matching tool result messages in original order, and stop launching remaining calls. |
| Tool timeout | Mark the tool record error and return the timeout message as tool content. |

### 5. Good/Base/Bad Cases

- Good: a model emits `read_file` and `web_fetch` in one round; both enter running state before either finishes, but replay messages preserve model order.
- Good: a trusted MCP server exposes two tools with `readOnlyHint: true`; both may overlap, and their `structuredContent` remains visible in records/events/model replay.
- Base: a model emits only `run_python`; calls execute one at a time and keep old lifecycle behavior.
- Bad: a scheduler parallelizes `skill_activate` or an arbitrary MCP stdio tool without explicit read-only annotations and races shared state or external side effects.
- Bad: schema validation happens inside one executor implementation only; other executors can still receive invalid arguments or approval prompts can show invalid payloads.

### 6. Tests Required

- Prove two eligible tools overlap by recording start/finish events.
- Prove explicitly read-only MCP tools overlap while destructive/open-world/non-read-only MCP tools remain serial.
- Prove returned `response_messages` and persisted `tool_records` follow original call order.
- Prove schema-invalid arguments produce error records and never call the executor or approval hook.
- Prove MCP `annotations`, `outputSchema`, and result `structuredContent` survive parse/registry/command/TypeScript boundaries.
- Prove serial-only tools never overlap.
- Prove unknown and invalid calls flush pending parallel batches and preserve result ordering.
- Run `cargo test --manifest-path src-tauri/Cargo.toml chat::agent:: -- --nocapture` for targeted changes.
- Run `cargo test --manifest-path src-tauri/Cargo.toml` before completion when practical.

### 7. Wrong vs Correct

#### Wrong

```rust
for call in tool_calls {
    tokio::spawn(execute_any_tool(call));
}
```

This loses transcript order, approval sequencing, cache safety, and cancellation ownership.

```rust
request_tool_approval(ctx, record).await;
validate_tool_arguments(tool, &call.arguments)?;
```

This can ask the user to approve a payload that will never execute and makes guardrail behavior inconsistent.

#### Correct

```rust
// Validate first, classify next, run only explicitly safe read-only tools together,
// then append all result messages in original model-call order.
```

Keep provider-side multiple tool-call support separate from local execution concurrency.

## Scenario: Assistant Timeline Segments

### 1. Scope / Trigger

- Trigger: changes that alter Chat assistant message storage, `chat-stream` payloads, streaming preview state, final assistant replay, or frontend assistant rendering.
- Assistant output is a time-ordered timeline, not three independent buckets. Runtime text, reasoning, and tool calls must be representable as interleaved `segments`.

### 2. Signatures

- Persistent model: `ChatMessage.segments: Vec<ChatMessageSegment>` with `#[serde(default)]` for old conversations.
- Segment shape: `ChatMessageSegment { id, kind, phase, order, step_number?, round?, text?, tool_call_id? }`.
- Segment enums:
  - `kind`: `text | reasoning | tool`
  - `phase`: `auxiliary | plain | tool_loop | synthesis`
- Runtime result: `AgentRunResult.segments` and `AgentStepResult.segments`.
- Stream host: `AgentHost::emit_stream_delta(..., segment: Option<&ChatMessageSegment>)`.
- Tauri event: `chat-stream` includes `segmentId`, `segmentKind`, `phase`, `order`, `stepNumber`, `round`, `toolCallId`, and full `segment`.

### 3. Contracts

- The backend is the source of truth for segment order. The frontend must not synthesize guessed tool segments from `chat-tool` progress events.
- `SegmentBuilder` starts assistant-runtime segment ordering at `1000`. Auxiliary tool segments may use lower orders such as `100`.
- Planning/tool-loop narration uses `kind = text`, `phase = tool_loop`. Final answer text uses `phase = synthesis` when tools are involved, otherwise `plain`.
- Reasoning is represented by `kind = reasoning` segments and may appear during tool loop or synthesis. Within the same model step, reasoning must display before that step's text; reserve reasoning segments before text segments and keep frontend rendering compatible with older persisted messages that had the reverse order.
- Tool calls must get `kind = tool` segments before or alongside visible tool progress. Visible skipped, cancelled, blocked, auxiliary, unknown, and invalid tool calls still need tool segments so the timeline has no holes.
- Hidden disabled built-in feedback such as disabled `web_search` retry hints must not create visible tool segments when it does not create a visible `ToolCallRecord`.
- Persisted legacy fields are derived by `push_assistant_message` / edit helpers:
  - `content` is the `\n\n` join of all non-empty `kind = text` segments whose phase is `plain` or `synthesis`.
  - `reasoning` is the `\n\n` join of all non-empty `kind = reasoning` segments.
  - `tool_calls` remains the full record list in runtime/model order.
- `model_messages` for storage must use the same final legacy content/reasoning and preserve the tool transcript. Editing an assistant final answer must replace only final `plain`/`synthesis` text segments and rewrite replay messages to the edited final answer.
- Streaming and persisted rendering share the same segment shape. `chat-tool` events update tool record state only; `chat-stream` segment fields update timeline layout.
- Finish must not blank the assistant preview before persisted content is available. If `done` is delayed until after the invoke response, the frontend should patch from the returned conversation or await the pending done handler before clearing the snapshot.

### 4. Validation & Error Matrix

| Condition | Runtime behavior |
|---|---|
| Old assistant message has no `segments` | Deserialize successfully; synthesize fallback segments from `content`, `reasoning`, and `tool_calls` when rewriting or normalizing. |
| Runtime result has content/reasoning but no matching final segment | `normalize_assistant_segments` appends a final text/reasoning segment before save. |
| Tool record has no matching tool segment | `normalize_assistant_segments` appends a tool segment with `auxiliary` for round `0`/Mixer or `tool_loop` otherwise. |
| Plan-mode blocked or approval-denied tool is skipped/cancelled | Persist and stream a visible tool segment plus the matching `ToolCallRecord`. |
| Disabled built-in fallback feedback is hidden | Append model-facing feedback only; do not emit a visible segment or record. |
| Assistant final answer is edited | Preserve prior tool-loop text/reasoning/tool segments, replace final text segments, clear stale `api_messages`, and rewrite `model_messages`. |
| Stream `done` arrives after invoke success | Do not create an empty preview; finish from the returned persisted conversation or the pending done handler. |

### 5. Good/Base/Bad Cases

- Good: model narrates a plan, calls `read_file`, narrates another step, then writes a synthesis. The UI renders text -> tool -> text -> final in the same order during streaming and after reload.
- Good: auxiliary vision analysis runs before the main model. It appears as an auxiliary tool segment before the main assistant timeline.
- Base: old messages without `segments` still render with the legacy tool/reasoning/content fallback.
- Bad: frontend receives `chat-tool` and inserts a guessed tool segment based on arrival time, causing stream order to differ from persisted order.
- Bad: save writes only `segments` and leaves legacy `content` stale, so copy/export/replay uses the wrong final answer.
- Bad: send/regenerate clears streaming preview in `finally` before reload or returned conversation has been applied, causing a blank frame.

### 6. Tests Required

- Serde compatibility: old assistant JSON without `segments` loads successfully.
- Legacy derivation: `content_from_segments` joins only `plain`/`synthesis` text; `reasoning_from_segments` joins reasoning segments.
- Segment normalization: auxiliary and skipped/cancelled tool records receive matching tool segments.
- Stream order: visible tool segments are produced from model-call order and hidden disabled built-ins are excluded.
- Edit replay: editing an assistant reply rewrites replay to the edited final answer while preserving tool transcript output.
- Frontend type/build: `npm run typecheck` verifies `chat-stream` segment payloads and `MessageBubble` timeline rendering props.

### 7. Wrong vs Correct

#### Wrong

```typescript
api.onChatTool((payload) => {
  snapshot.segments.push(makeToolSegmentFromArrivalTime(payload))
})
```

This makes streaming layout depend on event timing instead of the model/tool-call order saved by the backend.

```rust
message.segments = segments;
message.content = content;
```

This can leave legacy fields, replay, copy/export, and old UI paths inconsistent with the timeline.

#### Correct

```rust
let segments = normalize_assistant_segments(&content, reasoning.as_deref(), &tool_calls, segments);
let stored_content = content_from_segments(&segments).unwrap_or_else(|| content.clone());
let stored_reasoning = reasoning_from_segments(&segments).or(reasoning);
```

```typescript
const segment = streamPayloadToSegment(payload)
if (segment) {
  snapshot.segments = upsertStreamSegment(snapshot.segments, segment, delta)
}
```

Backend-owned segments define the timeline; legacy fields are deterministic projections of that same timeline.

## Scenario: Agent Todo Runtime State

### 1. Scope / Trigger

- Trigger: changes that add or modify agent-owned conversation state maintained by model tools, especially `agent_todo_state`, `todo_write`, `todo_update`, prompt injection, or `chat-todo` events.
- This is agent runtime state, not a user task manager. Users may observe it in the Chat UI, but they must not manually edit it.

### 2. Signatures

- Persistent model: `Conversation.agent_todo_state: AgentTodoState` with `#[serde(default)]` for old conversation JSON.
- State shape: `AgentTodoState { items: Vec<AgentTodoItem>, updated_at: i64 }`.
- Item shape: `AgentTodoItem { id: String, content: String, status: AgentTodoStatus }`.
- Status enum: `pending | in_progress | completed`.
- Native tools: `todo_write({ todos })` replaces the full list; `todo_update({ id, content?, status? })` updates one existing item.
- Tauri event: `chat-todo` payload `{ conversationId, todoState }`.

### 3. Contracts

- Canonical todo state lives on `Conversation`, not only in tool records or message metadata.
- Current todo state must be injected into the model system/runtime prompt before `build_chat_api_messages`, and `compute_context_state` must include the same prompt segment for token estimates.
- Todo tools are appended by the Chat runtime when the provider supports tool calls; they are not governed by user MCP/native-tool settings, assistant tool presets, or data connector filters.
- Todo tools are serial state writes. They bypass approval but must not be added to the native parallel-safe set.
- Tool results must include `structured_content` with the latest `todoState`.
- The frontend treats todo state as read-only conversation data and updates it from `chat-todo`.
- If a tool writes conversation state during `run_agent_loop`, `complete_assistant_reply` must reload/merge the latest todo state before saving the assistant message, otherwise the older in-memory `Conversation` can overwrite the tool update.

### 4. Validation & Error Matrix

| Condition | Runtime behavior |
|---|---|
| Old conversation lacks `agent_todo_state` | Deserialize to an empty default state. |
| `todo_write.todos` contains empty `id` or `content` | Return a tool error; do not save state. |
| `todo_write.todos` contains duplicate ids | Return a tool error; do not save state. |
| More than one item is `in_progress` | Normalize to at most one `in_progress`; demote extras to `pending`. |
| `todo_update.id` is missing or unknown | Return a tool error; do not save state. |
| `todo_update` provides neither `content` nor `status` | Return a tool error; do not save state. |
| Provider does not support tools or is Apple local | Inject todo context as read-only; do not expose todo tools. |

### 5. Good/Base/Bad Cases

- Good: model calls `todo_write` at the start of a multi-step task, then `todo_update` as work advances; UI receives `chat-todo` and later turns see the persisted state in context.
- Base: conversation has no todos; prompt says there are no current todos and UI renders no panel.
- Bad: storing todo only in `ToolCallRecord.structured_content`; next turn loses the working state after reload or compaction.
- Bad: appending todo tools before assistant/data-connector filters; `tool_preset: none` can accidentally remove agent housekeeping.
- Bad: saving the old in-memory conversation after tool execution without merging latest `agent_todo_state`.

### 6. Tests Required

- Serde compatibility: old conversation JSON without `agent_todo_state` loads with an empty state.
- Normalization: multiple `in_progress` items collapse to one.
- Update behavior: setting a new item to `in_progress` demotes the previous active item.
- Prompt/context: todo prompt segment appears in both request construction and context estimates.
- Tool trace: successful todo tools persist `structured_content.todoState`.
- Frontend type/build: `npm run typecheck` verifies `chat-todo` payload and read-only panel wiring.

### 7. Wrong vs Correct

#### Wrong

```rust
let result = run_agent_loop(...).await?;
push_assistant_message(app, state, settings, conversation, ..., result.tool_records, ...).await?;
```

This can overwrite todo changes that a tool already saved to disk during the run.

#### Correct

```rust
let result = run_agent_loop(...).await?;
merge_latest_agent_todo_state(app, conversation);
push_assistant_message(app, state, settings, conversation, ..., result.tool_records, ...).await?;
```

Tool-owned conversation state must be merged back before the final conversation save.

## Scenario: Agent Plan Mode Runtime State

### 1. Scope / Trigger

- Trigger: changes that add or modify Plan/Act behavior, especially `agent_plan_state`, plan prompt injection, plan approval commands, Plan-mode tool filtering, or `chat-plan` events.
- Plan mode is an agent runtime permission mode, not a user task manager. It lets the assistant investigate and draft an implementation plan before side-effecting actions are allowed.
- The persisted plan is read-only from the user's perspective. Do not add user-editable plan fields without a product decision.

### 2. Signatures

- Persistent model: `Conversation.agent_plan_state: AgentPlanState` with `#[serde(default)]` for old conversation JSON.
- State shape: `AgentPlanState { mode: AgentPlanMode, status: AgentPlanStatus, plan: Option<String>, updated_at: i64 }`.
- Mode enum: `act | plan`.
- Status enum: `empty | draft | approved`.
- Tauri commands:
  - `chat_set_agent_plan_mode(conversation_id: String, mode: String) -> { success, conversation, planState }`
  - `chat_execute_agent_plan(conversation_id: String) -> { success, conversation, planState }`
- Tauri event: `chat-plan` payload `{ conversationId, planState }`.
- Prompt segment id: `agent_plan`.

### 3. Contracts

- New conversations start in `mode = act`, `status = empty`, `plan = None`.
- `chat_set_agent_plan_mode` only accepts `act` or `plan`; it preserves the saved plan text and status while changing mode.
- `chat_execute_agent_plan` switches to `act`; if a non-empty plan exists, status becomes `approved`, otherwise it remains `empty`.
- In Plan mode, the final assistant reply is captured as a draft plan only when the original turn started in Plan mode and the latest saved state is still Plan mode.
- Current plan state must be injected into the system/runtime prompt before `build_chat_api_messages`, and `compute_context_state` must include the same `agent_plan` segment for token estimates.
- In Act mode, approved or draft plan text remains contextual; if the user asks to execute/continue, the model should use it unless the latest user message changes requirements.
- Plan mode must filter side-effecting tools before model invocation. Allowed tools are:
  - native read-only tools: `web_search`, `web_fetch`, `read_file`, `memory_read`
  - MCP tools with explicit `readOnlyHint == true` and no destructive/open-world hints
  - skill discovery/read tools: `skill_activate`, `skill_read_file`
  - agent todo tools: `todo_write`, `todo_update`
- Plan mode must not expose writes/edits, command execution, `run_python`, memory mutation, Mixer image generation, `skill_run_script`, or arbitrary/non-read-only MCP tools.
- Tools removed by the Plan filter must be kept as blocked metadata for the current run. If the model still requests one through fallback markup or stale provider state, emit a visible `ToolCallRecord` with `status = skipped` and return model-facing feedback that the tool is blocked in Plan mode.
- If plan state is updated while a reply is being completed, `complete_assistant_reply` must reload/merge the latest plan state before saving the assistant message, otherwise an older in-memory `Conversation` can overwrite the plan update.
- The frontend treats plan state as read-only conversation data and updates it from `chat-plan`.

### 4. Validation & Error Matrix

| Condition | Runtime behavior |
|---|---|
| Old conversation lacks `agent_plan_state` | Deserialize to default Act/Empty state. |
| `chat_set_agent_plan_mode` receives an unknown mode | Return an error; do not save conversation state. |
| Plan mode assistant reply is blank | Do not replace the current plan. |
| User executes with no saved plan | Switch to Act and keep status `empty`. |
| User executes with a saved plan | Switch to Act and mark status `approved`. |
| Plan mode tool list includes write/command/Python/memory mutation/image/script tools | Remove them before provider invocation. |
| Model requests a tool removed by Plan filtering | Emit a visible skipped tool record and return a tool message explaining it is blocked in Plan mode. |
| Non-read-only MCP tool has `readOnlyHint == false`, missing read-only metadata, `destructiveHint == true`, or `openWorldHint == true` | Remove it in Plan mode. |
| Provider does not support tools or is Apple local | Inject plan context as prompt text; do not expose unavailable tools. |

### 5. Good/Base/Bad Cases

- Good: user switches to Plan, asks for implementation analysis, the agent reads files/searches web, returns a plan, and the draft is visible and persisted on the conversation.
- Good: user clicks Execute Plan, runtime switches to Act, sends a continuation request, and the saved plan is injected into the next model turn.
- Base: no plan exists; UI shows no plan panel, prompt says there is no saved plan, and Act behavior is unchanged.
- Bad: Plan mode only changes the system prompt while leaving `write_file`, `run_command`, or `run_python` in the tool schema.
- Bad: storing the plan only in assistant message text; next turn loses the accepted plan after reload, compaction, or route changes.
- Bad: treating the plan as a calendar/reminder/task-management object or allowing manual user edits in the MVP.

### 6. Tests Required

- Serde compatibility: old conversation JSON without `agent_plan_state` loads as Act/Empty.
- State helpers: draft capture trims non-empty assistant replies and approval marks saved plans as `approved`.
- Prompt/context: `agent_plan` prompt segment appears in both request construction and context estimates.
- Tool filter: Plan mode keeps read-only native/MCP, skill read tools, and todo tools while removing writes, commands, Python, memory mutation, image generation, script execution, and non-read-only MCP tools.
- Blocked-tool trace: a model request for a Plan-filtered tool yields a `skipped` record rather than silently disappearing.
- Command registration/API: new Tauri commands are registered and frontend types mirror the payload.
- Frontend type/build: `npm run typecheck` verifies `chat-plan` event wiring, Plan/Act controls, Execute Plan flow, and read-only plan panel props.

### 7. Wrong vs Correct

#### Wrong

```rust
let system_prompt = format!("{base}\nPlan mode: don't edit files");
let tools = list_tools_for_chat(...).await;
```

This asks the model not to act while still exposing side-effecting tools.

```rust
let result = run_agent_loop(...).await?;
capture_plan_from_reply(conversation, &result.content);
push_assistant_message(..., conversation, ...).await?;
```

This can overwrite a concurrently saved plan/mode change with stale in-memory conversation state.

#### Correct

```rust
apply_agent_plan_tool_filter(&mut tools, is_plan_mode);
let agent_plan_prompt = plan::format_prompt(&conversation.agent_plan_state, &language);
```

```rust
let result = run_agent_loop(...).await?;
merge_latest_agent_plan_state(app, conversation);
capture_agent_plan_draft_if_needed(app, conversation, is_plan_mode, &result.content);
push_assistant_message(..., conversation, ...).await?;
```

Plan mode must be enforced by both prompt context and backend tool availability, and persisted state must be merged before the final save.
