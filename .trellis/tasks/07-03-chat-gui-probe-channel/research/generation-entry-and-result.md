# Research: Generation entry + result shape (what the probe must invoke and capture)

- **Query**: `chat_send_message` internals, tool assembly, `run_agent_loop` result fields, `ToolCallRecord` shape, conversation create/save — everything the probe must call and serialize.
- **Scope**: internal
- **Date**: 2026-07-03

## Findings

### The real generation entry chain

```
chat_send_message  (Tauri cmd, commands.rs:1182)      — public entry the GUI uses
  └─ complete_assistant_reply  (commands.rs:1720)     — single-model path (fan-out is separate)
       └─ complete_assistant_reply_inner (1748)       — assembles tools + system prompt, builds host/executor
            └─ crate::chat::agent::run_agent_loop(AgentRunConfig{..}, &host, &executor)  (commands.rs:2172)
            └─ push_assistant_message(...)  (commands.rs:2269)  → save_conversation
```

`chat_send_message` is a **self-contained async fn** that **awaits generation to completion and returns the full result inline** (not fire-and-forget). It returns:
```rust
Ok(serde_json::json!({ "success": true, "conversation": conversation }))   // commands.rs:1404
```
The returned `conversation` is passed through `strip_transcripts_for_frontend` first (only clears `model_messages`/`api_messages` on completed assistant messages — **keeps `content` and `tool_calls`**, `commands.rs:274-289`). So the LAST assistant message in the returned conversation carries `content` (answer) + `tool_calls: Vec<ToolCallRecord>`. This is exactly the probe's `result.json` payload, available inline without re-reading disk.

### `chat_send_message` signature (commands.rs:1182-1189)

```rust
#[tauri::command]
pub(crate) async fn chat_send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    attachments: Vec<String>,       // attachment file paths (probe: pass vec![])
    active_skill_id: Option<String>,
) -> Result<serde_json::Value, String>
```
Gated at entry by `ChatSendReservation::try_acquire` (busy check per conversation, 1194) — returns `{success:false, error: CHAT_REPLY_BUSY_ERROR}` if that conversation already has a run. A fresh scratch conversation is never busy, so this is fine for the probe.

### Tool assembly (the full native+skill+mcp+todo+ask_user+agent set)

All in `complete_assistant_reply_inner`, `commands.rs:2012-2058`:

1. **Base tools** — `list_tools_for_chat(app, state, &settings, provider.supports_tools, session)` (2012). This is the aggregate: native builtin + MCP + skills + knowledge_search + image-gen tools (per settings). `list_tools_for_chat` is the load-bearing assembler in `commands.rs`.
2. `agent_prepare::apply_assistant_mcp_restrictions` (2020), builder-mode clamp (2024-2029), active-skill filter (2030-2032), inline-code filter (2033), plan-mode filter → `blocked_tool_calls` (2034).
3. `append_agent_ask_user_tools(&mut tools, provider.supports_tools)` (2041) → `crate::chat::ask_user::append_tool_definitions` (commands.rs:4316-4325).
4. `append_agent_todo_tools(&mut tools, provider.supports_tools)` (2042) → `crate::chat::todo::append_tool_definitions` (commands.rs:4305-4314).
5. `crate::chat::sub_agent::append_tool_definitions(&mut tools, true)` — the `agent` spawn tool, added when `provider.supports_tools && !plan_mode && !builder_mode` (2046-2048).

So driving `chat_send_message` on a normal (non-plan, non-builder) scratch conversation reproduces the FULL GUI tool set. Nothing extra to wire — this is the crux the probe wants.

### `run_agent_loop` call + result fields

Called at `commands.rs:2172` with `AgentRunConfig { entry, state, conversation_id, tool_conversation_id, depth, run_id, message_id, generation, provider, model, runtime_messages, tools, blocked_tool_calls, settings, effective_chat_tools, language, has_image, thinking_*, stream_enabled, max_output_tokens, retry_attempts, skill_registry, active_skill_id, active_skill_detail, assistant_snapshot, custom_system_prompt, provider_tools_fallback_system_prompt }`, plus `&host` and `&executor`.

`run_agent_loop` is defined at `src-tauri/src/chat/agent/loop_.rs:135`. It returns `Result<AgentRunResult, String>`.

**`AgentRunResult` (`chat/agent/types.rs:85-107`)** — confirmed field names:
```rust
pub struct AgentRunResult {
    pub content: String,                              // ← answer text
    pub reasoning: Option<String>,
    pub tool_records: Vec<ToolCallRecord>,            // ← toolCalls source
    pub segments: Vec<ChatMessageSegment>,
    pub api_messages: Vec<Value>,
    pub steps: Vec<AgentStepResult>,
    pub stream_outcome: String,                       // "completed" | "cancelled" | "interrupted" | ...
    pub usage: Option<ModelUsage>,
    pub compacted_history: Option<Vec<Value>>,
    pub compaction_boundary: Option<CompactionBoundaryRecord>,
    pub compaction_summary: Option<ConversationContextSummary>,
}
```
On the single-model path this result is folded into the conversation by `push_assistant_message(...)` (commands.rs:2269-2288) which then `save_conversation`s. `run_agent_loop` errors bubble up **before** `push_assistant_message`, so on hard failure the disk stays "user message, no assistant" (see the error comment at commands.rs:1416-1421).

### `ToolCallRecord` shape (`chat/types.rs:202-234`) — what to serialize into result.json's toolCalls

```rust
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,                        // ← {name}
    #[serde(default)] pub source: String,    // "native" | "mcp" | "skill" | ...
    #[serde(default)] pub server_id: Option<String>,
    #[serde(default)] pub arguments: String, // ← {args} — a JSON string, not an object
    pub status: ToolCallStatus,              // ← {status} enum: Pending/Running/Success/Error/Cancelled/Skipped (types.rs:~190-199)
    #[serde(default)] pub result_preview: Option<String>,
    #[serde(default)] pub error: Option<String>,
    #[serde(default)] pub duration_ms: Option<u64>,
    #[serde(default)] pub started_at / completed_at: Option<i64>,
    #[serde(default)] pub round: u32,
    #[serde(default)] pub sensitive: bool,
    #[serde(default)] pub artifacts: Vec<ChatToolArtifact>,
    #[serde(default)] pub trace_id / span_id: Option<String>,
    #[serde(default)] pub structured_content: Option<Value>,
}
```
For the probe's `toolCalls:[{name,args,status}]`: `name` ← `record.name`, `args` ← `record.arguments` (already a JSON string), `status` ← `record.status`. `ToolCallStatus` derives `Serialize` (serializes to its variant name).

### Conversation create / load / save (scratch conversation programmatically)

| Fn | File:line | Notes |
|---|---|---|
| `create_chat_conversation_internal(app: &AppHandle, state: &AppState, provider_id, model, folder, project_id, set_id, assistant_id) -> Result<Conversation, String>` | `commands.rs:320` | **Callable from a spawned task** (takes `&AppHandle` + `&AppState`, not `State<>`). `provider_id`/`model` `None` → falls back to `settings.effective_chat_model()` (357-373). Wrapped by the `chat_create_conversation` command (293). |
| `load_conversation(app, id)` | `storage.rs:358` | reads `{app_data}/conversations/{id}.json` |
| `save_conversation(app, conversation)` | `storage.rs:368` | atomic write + index update |
| Conversation id scheme | `commands.rs:423`, `762` | `format!("conv_{}", Uuid::new_v4())` |

`create_chat_conversation_internal` is the right primitive for the probe to mint a scratch conversation with the default (or request-specified) provider/model. (Confirm whether it internally `save_conversation`s before returning — the wrapper command returns it directly; the probe can `save_conversation` defensively before calling `chat_send_message`, which itself does `load_conversation(&app, &conversation_id)` at 1202.)

### The AgentHost used by chat_send_message

`ChatAgentHost<'a> { app: AppHandle, state: &'a AppState, suppress_partial_persist: bool }` — `commands.rs:4848-5002`. It implements `crate::chat::agent::AgentHost` by delegating to the module's `emit_chat_*` event helpers, `request_tool_approval`, `request_session_consent`, `request_user_response`, and generation-active checks. The tool executor is `RegistryToolExecutor { app, state }` (commands.rs:5004).

Implication for the probe: **the generation is self-contained in the command** — `ChatAgentHost` is constructed locally from `AppHandle` + `&AppState` (both available to a spawned background task via `app_handle.state::<AppState>()`). The probe does **not** need any frontend-driven state to run generation; it only needs `app` + `state` + a conversation id. The host's `request_tool_approval` / `request_user_response` / `request_session_consent` emit events and await a oneshot — those are the only frontend-dependent paths, and they only fire for sensitive tools / ask_user under a non-`auto` approval policy (see risks in `probe-hook-points.md`).

## Caveats / Not Found

- `chat_send_message` will branch into **fan-out** (`run_reply_fan_out`, commands.rs:2305) when the conversation has ≥2 `reply_models` and it's not plan/orchestrate mode (1241-1247). A scratch conversation has no `reply_models`, so it takes the single-model path — good, but the probe must not set reply_models.
- It also branches to **direct image generation** for image-capable models (1828-1854) — avoid image-gen models for the probe, or expect no tool_calls there.
- The exact internals of `list_tools_for_chat` (which native tool defs it enumerates, e.g. via `mcp::native_registry`) were not opened line-by-line here; it is the single aggregate call at commands.rs:2012 and is the same one the GUI uses, so faithfulness is guaranteed by reuse rather than re-derivation.
