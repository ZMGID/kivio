# Research: Probe hook points (where/how to wire the file-watched probe)

- **Query**: lib.rs spawn point, app-data dir fn, notify-vs-poll, command registration, debug-gating; recommend backend-direct vs frontend-queue.
- **Scope**: internal
- **Date**: 2026-07-03

## Findings

### A. Startup spawn point (lib.rs `.setup(...)`)

`.setup(|app| { ... })` runs at `src-tauri/src/lib.rs:186-409`. Inside it, background tasks are spawned with `tauri::async_runtime::spawn` after `AppState` is managed. Existing spawn precedents:

- Update check (one-shot sleep+spawn): `lib.rs:315-334`.
- **MCP idle reaper (poll loop) — the best template for the probe**: `lib.rs:338-361`
  ```rust
  let app_handle = app.handle().clone();
  tauri::async_runtime::spawn(async move {
      let mut ticker = tokio::time::interval(Duration::from_secs(60));
      loop {
          ticker.tick().await;
          let Some(state) = app_handle.try_state::<AppState>() else { continue }; // race-hardening
          // ... use &*state (Deref to &AppState) ...
      }
  });
  ```
- MCP warmup: `lib.rs:365-396`. Launch-open-chat: `lib.rs:399-407`.

The probe watcher should be one more `tauri::async_runtime::spawn` in this block. `app.handle().clone()` gives an owned `AppHandle` (`'static`), and `app_handle.try_state::<AppState>()` yields the managed `State<AppState>` for the generation call. **Recommended insertion: right after the MCP reaper block (~lib.rs:361), gated by `#[cfg(debug_assertions)]`.**

### B. App-data dir resolution

The app-data dir is obtained via `app.path().app_data_dir()` (Tauri v2). Confirmed usage in `chat/storage.rs:149-158` (`conversations_dir`):
```rust
let base = app.path().app_data_dir().map_err(|e| ...)?;
let dir = base.join("conversations");
```
This is the same base that houses `settings.json`, `conversations/`, `knowledge_base/`, `memory`, etc. The probe should base `chat_probe/` on it:
```rust
let base = app_handle.path().app_data_dir()?;      // needs `use tauri::Manager;` (already imported in lib.rs)
let probe_dir = base.join("chat_probe");           // <app_data>/chat_probe/{request.json,result.json}
```

### C. notify vs poll — deps in Cargo.toml

`src-tauri/Cargo.toml` has **no `notify` (file-watch) crate**. It does have:
- `tokio = { version = "1", features = ["time", "process", "io-util", "sync", "rt-multi-thread", "macros"] }` (line 47) — `time` feature ⇒ `tokio::time::interval` is available.
- `chrono` (43), `serde`/`serde_json` (used throughout).

**Recommendation: a simple poll loop** (`tokio::time::interval`, e.g. every 500ms–1s) that stats `<app_data>/chat_probe/request.json`, mirroring the MCP reaper. Rationale: (1) no new dependency; (2) matches an existing, proven pattern in the same file; (3) file-watch latency is irrelevant for a debug testbed; (4) avoids `notify`'s cross-platform edge cases (Windows/macOS backends, editor atomic-rename semantics). Debounce by tracking the file's mtime or by deleting/renaming `request.json` once consumed.

### D. Command registration list

`invoke_handler(tauri::generate_handler![ ... ])` at `lib.rs:410`. Relevant already-registered commands (for reference / if a manual-trigger command is also wanted): `chat::commands::get_request_debug_records` (463), `chat_create_conversation` (471), `chat_take_external_sends` (490), `chat_send_message` (493). A probe-trigger command is optional — the file-watch loop is the primary mechanism; a command would only add a synchronous "run once" entry point.

### E. Debug-gating precedent

Only `main.rs:1` uses `debug_assertions` today (`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`). There is **no existing `#[cfg(debug_assertions)]` inside lib.rs**, so the probe would introduce the pattern. Gating the spawn block (and the module) with `#[cfg(debug_assertions)]` compiles it out of release builds, satisfying the "must be compiled out of release" requirement. (Alternative: an env-var / settings flag checked at runtime, but `cfg` is the cleaner compile-out guarantee.)

## Decision: backend-direct invocation (recommended) vs frontend-queue

**Recommendation: BACKEND-DIRECT.** The watcher should run the generation itself, entirely backend-side, and write `result.json` from the inline result. Do NOT route through `chat-external-send-ready` → `Chat.tsx` → submit.

**Justification:**
1. **The generation is self-contained.** `chat_send_message` (commands.rs:1182) constructs `ChatAgentHost { app, state, .. }` and `RegistryToolExecutor { app, state }` locally from `AppHandle` + `&AppState` and `.await`s `run_agent_loop` to completion, returning the answer + tool_records inline (see `generation-entry-and-result.md`). Nothing in the core path requires frontend-driven state.
2. **It awaits completion and returns results inline** — the probe can synchronously capture `{answer, toolCalls, conversationId, error}` right after the await, then write `result.json`. The frontend-queue path (Lens channel) never captures results; it would require scraping events or polling disk, which is strictly worse.
3. **It reproduces the full GUI tool set** by construction — it calls the exact same `list_tools_for_chat` + todo/ask_user/sub_agent assembly the chat window uses (commands.rs:2012-2048), so a tool-rename check via the probe is faithful to the real client.
4. **No GUI/window required** — a spawned task with `AppHandle` can create a scratch conversation via `create_chat_conversation_internal(&app, state.inner(), ..)` (commands.rs:320) and call the generation without the chat window being open.

**Sketch of the backend-direct flow (watcher tick):**
```
read <app_data>/chat_probe/request.json → {prompt, provider?, model?}
let state = app_handle.state::<AppState>();
let conv = create_chat_conversation_internal(&app_handle, &state, provider, model, None,None,None,None)?;  // scratch
save_conversation(&app_handle, &conv)?;                                     // ensure load_conversation finds it
let out = chat_send_message(app_handle.clone(), state, conv.id.clone(), prompt, vec![], None).await;       // ← real path
// extract last assistant message content + tool_calls from out["conversation"] (or load_conversation(conv.id))
write <app_data>/chat_probe/result.json { answer, toolCalls:[{name,args,status}], error, conversationId }
delete/rename request.json (debounce)
```
Note: calling the `#[tauri::command] chat_send_message` fn directly requires a `State<'_, AppState>` — obtain via `app_handle.state::<AppState>()`. If borrow/lifetime friction arises, factor the body of `chat_send_message` into a `pub(crate) async fn` taking `&AppHandle` + `&AppState` (mirroring how `chat_create_conversation` delegates to `create_chat_conversation_internal`) and have both the command and the probe call it.

### Risks / decision inputs

- **Tool-approval / ask_user blocking (the main risk).** `ChatAgentHost::request_tool_approval` / `request_session_consent` / `request_user_response` (commands.rs:4931-4986) emit an event and `await` a `oneshot` that only the GUI answers. Under a non-`auto` `approval_policy`, a sensitive tool (`write`/`run_command`) or an `ask_user` call would hang the probe. Mitigations: (a) set `effective_chat_tools.approval_policy = "auto"` for probe runs (fan-out arms already do exactly this at commands.rs:1997) — but that's set deep in `complete_assistant_reply_inner`, so a probe flag would need threading, OR (b) pre-seed `state.chat_session_consent` with the scratch conversation id (covers file/command session consent) and rely on auto policy, OR (c) accept a timeout on `result.json` writes. **This is the key implementation decision to resolve when building** — recommend an explicit "auto-approve" probe mode.
- **`stream_outcome` values.** `AgentRunResult.stream_outcome` can be `"cancelled"`/`"interrupted"`; `chat_send_message` maps `err == "cancelled"` to `success:true` (commands.rs:1409). The probe should surface stream_outcome / error into `result.json` faithfully.
- **Concurrency / reentrancy.** Each probe run should mint a fresh `conv_{uuid}` (no id collisions). Serialize probe runs in the watcher (process one request.json at a time) to avoid overlapping generations and to keep result.json unambiguous. `ChatSendReservation` already blocks a second send on the *same* conversation, but different scratch convs are independent.
- **Scratch conversation litter.** Each run creates a persisted `conversations/{id}.json` + index entry. Consider deleting it after capture (`delete_conversation`, storage.rs:422) or reusing a fixed probe conversation id.
- **Debug-gating.** Wrap module + spawn in `#[cfg(debug_assertions)]` so it's absent from release (no existing in-lib precedent; introduce it).

## Files Found

| File Path | Description |
|---|---|
| `src-tauri/src/lib.rs` | `.setup` (186), spawn precedents (315/338/365/399), MCP reaper poll template (338-361), `invoke_handler!` (410) |
| `src-tauri/src/chat/storage.rs` | `conversations_dir` → `app.path().app_data_dir()` (149), `load/save/delete_conversation` (358/368/422) |
| `src-tauri/src/chat/commands.rs` | `create_chat_conversation_internal` (320), `chat_send_message` (1182), `ChatAgentHost` (4848) |
| `src-tauri/Cargo.toml` | tokio `time` feature (47); no `notify` dep |

## Caveats / Not Found

- Whether `create_chat_conversation_internal` internally persists the conversation before returning was not confirmed line-by-line (only its construction/return at 320-373 was read). The probe should `save_conversation` defensively (cheap, idempotent) before `chat_send_message` does its `load_conversation` at 1202.
- The exact ergonomics of calling the `#[tauri::command]` `chat_send_message` directly from a spawned task (vs. refactoring its body into a plain async fn) is an implementation choice; the cleaner refactor mirrors the existing `chat_create_conversation → create_chat_conversation_internal` split.
