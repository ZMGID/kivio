# MCP Connection Lifecycle

> Persistent MCP connection manager: session pooling, liveness/reconnect, idle reaping, image artifacts, and status surfacing. Landed in P2-A (`src-tauri/src/mcp/manager.rs`).

## Where it lives

- `src-tauri/src/mcp/manager.rs` — `McpSession`, `McpTransport` (`Stdio(StdioConn)` | `Http{session_id}` | `None`), `StdioConn` (child + reader/stderr tasks + pending oneshot table), and the `impl AppState` API (`mcp_get_or_connect`, `mcp_call_tool`, `mcp_list_session_tools`, `mcp_reload_server`, `mcp_disconnect_all`, `mcp_server_status`, `mcp_warmup`, `mcp_reap_idle`).
- Pool: `AppState.mcp_sessions: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<McpSession>>>>` (mirrors the `key_cooldowns` map pattern).
- `src-tauri/src/mcp/client.rs` — stdio handshake helpers + `parse_tool_result` (now maps image content → artifacts).
- `src-tauri/src/main.rs` — startup `mcp_warmup` (JoinSet, non-blocking) + idle reaper spawn + `RunEvent::ExitRequested` teardown.

## Executable contracts (do not regress)

1. **One handshake per server.** Repeated `call_tool`/`list_tools` to the same server reuse the pooled session — exactly one `initialize` handshake per live session. `McpSession.handshake_count` is the test hook (`ten_calls_one_handshake`). Adding a code path that builds a throwaway client per call breaks this.
2. **Timeout ≠ dead.** A plain read/write timeout from `StdioConn::request` (`"... timed out"`) MUST be surfaced to the caller **without** killing the child or retrying. Only reconnect+retry-once when the connection is genuinely dead — gate on `conn.is_dead()` (try_wait) or `is_connection_closed_error` (closed stdout). Retrying on timeout silently double-executes non-idempotent MCP tools (file write / payment / send-message) and was a real HIGH bug — see `timeout_on_healthy_child_does_not_kill_or_reexecute`.
3. **Single-flight connect.** `mcp_get_or_connect` resolves the pool entry under one pool-lock acquisition: `match pool.get { Some(arc) => lock+recheck-Connected, None => insert placeholder while holding the pool lock }`. Never hold the pool lock across a handshake await. Concurrent callers for the same server share one session/handshake (`concurrent_get_or_connect_share_one_handshake`).
4. **No lock across await.** Never hold the outer `mcp_sessions` mutex across a handshake/RPC await. Clone the per-session `Arc` out, drop the pool lock, then lock the session.
5. **Idle reaper.** `mcp_reap_idle` removes sessions idle beyond `chat_tools.mcp_idle_timeout_ms` (configurable, serde default `600_000`, `sanitize_settings` clamps to 60s..24h). Reaper uses `try_lock` per session and SKIPS in-flight ones, so it never evicts a session mid-call. After eviction the next call transparently reconnects (a fresh session — `handshake_count == 1` on the new arc).
6. **No orphan processes on exit.** `StdioConn` is spawned with `kill_on_drop(true)`; its `Drop` aborts the reader/stderr tasks and `start_kill()`s the child. `RunEvent::ExitRequested` (real quit, `code.is_some()`) runs `block_on(timeout(2s, mcp_disconnect_all()))`; `mcp_disconnect_all` drains the pool then `try_lock`s each session (non-blocking, relies on `kill_on_drop`). Both the timeout bound and try_lock are required — do not make teardown block on in-flight session locks.
7. **HTTP reconnect only on session loss.** HTTP transport reuses the stored `session_id`; reconnect (clear id + re-`initialize` + retry once) ONLY on HTTP 404 / "session not found". 500 and other errors propagate without reconnect.
8. **Image content → artifacts.** `parse_tool_result` maps `content[].type == "image"` → `ChatToolArtifact { name, mime_type, data_url, size_bytes }` and leaves a `[image: <mime>]` text placeholder. Audio → `[audio: <mime>]` placeholder, no artifact. The `ChatToolArtifact`/`ChatToolCallResult` serde shapes are unchanged (frontend contract preserved).
9. **State events.** Every `Connecting/Connected/Error/Disconnected` transition emits `mcp-server-state` via the `McpEventSink` trait (impl for `AppHandle` = real emit; impl for `()` = no-op used in unit tests). Production paths (registry/commands/warmup) MUST pass the real `AppHandle`. `chat_mcp_server_status` / `chat_mcp_reload_server` back the settings status panel.

## Tests / verification

- Rust: `ten_calls_one_handshake`, `liveness_reconnect_on_dead_child`, `timeout_on_healthy_child_does_not_kill_or_reexecute`, `concurrent_get_or_connect_share_one_handshake`, `idle_reap_evicts_and_reconnects` (asserts fresh handshake after reap), `http_reconnect_only_on_404`, `http_500_does_not_reconnect`, `http_two_successful_calls_reuse_one_initialize`, `disconnect_all_kills_children`, `config_fingerprint_rebuilds_session`, `parse_tool_result_maps_image_to_artifact`.
- Tests use in-memory `Settings::default()` (via `test_app_state`) + fake stdio/HTTP servers only — never touch real settings.json/providers/API keys.
- Manual smoke: settings MCP panel shows per-server status dot + lastError + stderr tail + Reconnect; kill a server process and confirm transparent reconnect on next call; an image-returning MCP tool renders in chat.
