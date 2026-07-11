# MCP protocol runtime contract

## 1. Scope / Trigger

Applies whenever Kivio implements or changes MCP JSON-RPC routing, tool discovery, approval decisions, request cancellation, protocol negotiation, or Streamable HTTP session lifecycle in `src-tauri/src/mcp/` and `src-tauri/src/chat/agent/execute.rs`.

The server is an untrusted protocol peer. Server-provided capabilities and tool annotations describe behavior but do not grant local security permissions.

## 2. Signatures

Key runtime boundaries:

```rust
fn negotiated_protocol_version(initialize_result: &Value, streamable_http: bool) -> Result<String, String>;
async fn stdio_list_tools(conn: &StdioConn, timeout: Duration) -> Result<Vec<McpTool>, String>;
async fn http_list_tools(..., session_id: Option<&str>, protocol_version: &str) -> Result<Vec<McpTool>, String>;
async fn http_delete_session(..., session_id: &str, protocol_version: &str) -> Result<(), String>;
```

A long-lived stdio connection owns a pending-response map keyed by normalized JSON-RPC id and a monotonically increasing tool-list revision.

## 3. Contracts

### JSON-RPC routing

- A message containing both `method` and `id` is a server request, never a response.
- `ping` must receive `{"jsonrpc":"2.0","id":<same>,"result":{}}`.
- Other unsupported server requests receive JSON-RPC `-32601 Method not found`.
- A message containing `method` without `id` is a notification.
- Only a message without `method` and with an id may resolve a pending client request.
- Numeric `1` and string `"1"` response ids normalize to the same pending key for compatibility with non-conforming servers.

### Tool discovery

- `tools/list` is paginated. Send the returned opaque string `nextCursor` as the next request's `params.cursor` until absent.
- Reject non-string cursors, repeated cursors, and more than 100 pages.
- On stdio `notifications/tools/list_changed`, increment the connection revision and invalidate the aggregate chat-tool cache. The next list operation must fetch all pages again.
- Streamable HTTP push notifications are not claimed as supported until a persistent server-event listener exists; current POST response parsing is not a subscription.

### Approval trust boundary

- MCP `ToolAnnotations`, including `readOnlyHint=true`, are untrusted display hints.
- An annotation may make local handling stricter, but must never bypass confirmation.
- Only a user-owned policy such as explicit global `auto` approval may waive confirmation. Tool enablement and tool approval remain separate decisions.

### Cancellation and lifecycle

- When a non-initialize request times out, send best-effort `notifications/cancelled` with the original `requestId` and a reason.
- Never automatically retry a timed-out tool call because its outcome is unknown.
- Validate the initialize result's `protocolVersion`. Unknown versions fail connection setup.
- Stdio supports `2025-06-18`, `2025-03-26`, and `2024-11-05`. Streamable HTTP rejects `2024-11-05` explicitly because legacy HTTP+SSE is not implemented.
- After initialization, every Streamable HTTP request, notification, cancellation, and DELETE uses the negotiated protocol-version header and current session id.
- When an HTTP session is discarded, send best-effort DELETE; HTTP 404 counts as already deleted.

## 4. Validation & Error Matrix

| Condition | Required behavior |
|---|---|
| Server request id collides with pending client id | Answer server request; leave pending client request intact |
| Unsupported server request | Return `-32601`; do not enter pending routing |
| EOF with pending stdio requests | Drain pending map with `MCP server closed stdout` |
| `nextCursor` is not a string | Fail tool discovery with a cursor validation error |
| Cursor repeats or page count exceeds 100 | Fail instead of looping forever |
| Missing or unsupported initialize version | Fail connection setup |
| Streamable HTTP negotiates `2024-11-05` | Fail with explicit legacy HTTP+SSE limitation |
| Non-initialize request times out | Return timeout/unknown-outcome error and best-effort cancel |
| HTTP DELETE returns 404 | Treat as success |
| `readOnlyHint=true` from MCP server | Preserve annotation for display; still require approval under default policy |

## 5. Good / Base / Bad Cases

- Good: server sends a colliding-id ping while `tools/call` is in flight; Kivio answers ping and later delivers the real response.
- Good: a two-page tool list is merged, then `tools/list_changed` causes the next list to include a dynamically registered tool.
- Base: a server with one tools page and current protocol version behaves unchanged.
- Bad: routing every numeric id into the pending map allows a server request to consume a client response slot.
- Bad: trusting `readOnlyHint` allows an arbitrary MCP server to self-authorize execution.
- Bad: hard-coding the latest protocol header after negotiating an older supported version violates the negotiated session contract.

## 6. Tests Required

- Cross-platform stdio fake server: colliding-id ping, string response id, unsupported server request, real response delivery.
- Cross-platform stdio fake server: paginated `tools/list`, repeated refresh after `notifications/tools/list_changed`.
- Timeout regression: later requests are not head-of-line blocked, no automatic retry occurs, cancellation is observed.
- Approval unit test: `readOnlyHint=true` still requires confirmation under default policy and is waived only by user `auto` policy.
- Protocol-version unit test: accepted versions, unknown version rejection, and legacy Streamable HTTP rejection.
- HTTP integration test: initialize negotiates `2025-03-26`; initialized and later requests carry that version plus the session id; disconnect sends one DELETE with both headers.

## 7. Wrong vs Correct

### Wrong

```rust
if let Some(id) = value.get("id").and_then(Value::as_u64) {
    pending.remove(&id); // server ping can consume a client response slot
}

if tool.annotations.read_only_hint == Some(true) {
    requires_confirmation = false; // trusts an untrusted server hint
}
```

### Correct

```rust
if value.get("method").is_some() {
    handle_server_request_or_notification(value).await;
} else if let Some(id) = normalize_json_rpc_id(value.get("id")) {
    resolve_pending(id, value);
}

let requires_confirmation = tool.source == "mcp" && user_policy != "auto";
```
