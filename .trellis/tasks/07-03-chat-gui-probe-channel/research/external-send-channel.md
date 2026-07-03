# Research: Lens→Chat external-send channel (the pattern to reuse/mirror)

- **Query**: Map the Lens→chat external-send channel end-to-end (state shapes, push, emit, drain, frontend submit) as the pattern the file-watched probe would mirror.
- **Scope**: internal
- **Date**: 2026-07-03

## Findings

### End-to-end flow

The Lens→Chat handoff is a **backend-queue + frontend-drain** channel:

1. Lens command builds a `PendingChatExternalSend`, pushes it into `AppState.pending_chat_external_sends` (a `Mutex<Vec<...>>`), ensures the chat window, and emits `chat-external-send-ready` to the `chat` webview.
2. The chat frontend (`Chat.tsx`) listens for `chat-external-send-ready` (and also drains on focus / mount), calls the `chat_take_external_sends` command to **drain** the queue, then **replays each request through the normal user-send path** (`handleSendMessage`).

Key point for the probe design: **the actual generation is driven by the frontend** in this channel — the backend only parks a request and rings a bell. The frontend calls `handleSendMessage`, which eventually calls `chat_send_message`. So this channel does NOT itself run the agent loop; it just injects a message into the GUI. (See `probe-hook-points.md` for why the probe should instead call the generation backend-direct.)

### Files Found

| File Path | Description |
|---|---|
| `src-tauri/src/lens_commands.rs` | `lens_send_to_chat` (~998), `lens_send_history_to_chat` (~1068) — build + enqueue + emit |
| `src-tauri/src/state.rs` | Pending payload structs (~39-56) + `pending_chat_external_sends` field (line 124, init 226/753) |
| `src-tauri/src/chat/commands.rs` | `chat_take_external_sends` (~1058) — drain semantics |
| `src/chat/Chat.tsx` | `drainExternalSends` (~2737), listener wiring (~2807-2844), `externalSendQueueRef` (666) |

### Code Patterns

**Enqueue (backend), `lens_commands.rs:1028-1054`:**
```rust
let request_id = format!("lens_send_{}", Uuid::new_v4());
let request = PendingChatExternalSend { id: request_id.clone(), content: question, attachments, messages: Vec::new() };
{
    let mut pending = state.pending_chat_external_sends.lock().unwrap_or_else(|e| e.into_inner());
    pending.push(request);
}
open_chat_window(&app)?;                    // ensure chat window exists (rollback on err, 1043-1053)
let _ = app.emit_to("chat", "chat-external-send-ready", serde_json::json!({}));
```
`lens_send_history_to_chat` is the same shape but sets `content: String::new()` and a non-empty `messages` vec (history-preseed branch) — `lens_commands.rs:1106-1119`.

**State shapes, `state.rs:39-56` (all `#[serde(rename_all = "camelCase")]`):**
```rust
pub struct PendingChatExternalMessage { pub role: String, pub content: String }
pub struct PendingChatExternalSend {
    pub id: String,
    pub content: String,
    pub attachments: Vec<PendingChatExternalAttachment>,
    #[serde(default)] pub messages: Vec<PendingChatExternalMessage>, // empty = single-msg path; non-empty = history preseed
}
// PendingChatExternalAttachment { id, r#type, name, path } is at state.rs:26 (id/type/name/path)
```
Field: `pub pending_chat_external_sends: Mutex<Vec<PendingChatExternalSend>>` (`state.rs:124`).

**Drain (backend), `chat/commands.rs:1058-1074` — `std::mem::take`, idempotent (empty = no-op):**
```rust
#[tauri::command]
pub(crate) fn chat_take_external_sends(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let requests = {
        let mut pending = state.pending_chat_external_sends.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *pending)
    };
    Ok(serde_json::json!({ "success": true, "requests": requests }))
}
```

**Frontend drain + submit, `Chat.tsx:2737-2805`:**
- Calls `api.chatTakeExternalSends()`, pushes results into `externalSendQueueRef`.
- History-preseed branch (`request.messages.length > 0`): `importExternalConversationRef.current(...)` — builds a new conversation, **no reply triggered** (2768-2772).
- Single-message branch: maps attachments then calls
  `handleSendMessageRef.current(request.content ?? '', attachments, { forceNewConversation: true })` (2782-2786).
  So a drained single-message request routes through **the same `handleSendMessage` path as a user-typed message** (with `forceNewConversation: true`), which is the function that ultimately invokes the `chat_send_message` command.
- Listener wiring (`Chat.tsx:2807-2844`): drains on mount, on `onChatExternalSendReady`, on window focus, and when streaming finishes with a pending re-request. Robust against dropped events.

### Related Specs

- No dedicated spec file for this channel; behavior is documented inline in `lens_commands.rs:1062-1066` (comment explains the shared pipe + `messages` empty/non-empty branch semantics).

## Caveats / Not Found

- This channel is **frontend-driven**: nothing in it awaits generation completion or captures the answer/tool-calls. It is a good structural template (backend `Mutex<Vec<...>>` queue + drain command) but the probe needs to CAPTURE results, which the Lens channel never does. The probe should prefer backend-direct generation (see `generation-entry-and-result.md` + `probe-hook-points.md`).
