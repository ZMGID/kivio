# MCP protocol compliance review (2026-07-11)

## Verified defects

1. Long-lived stdio reader treated any numeric id as a response. This allowed server requests such as ping to collide with pending client ids and left ping unanswered.
2. Tool discovery ignored `nextCursor` and exposed only the first page.
3. `readOnlyHint=true` could relax local approval even though annotations are untrusted server hints.
4. Stdio ignored `notifications/tools/list_changed`, leaving cached tool definitions stale.
5. Timed-out requests did not send best-effort cancellation.
6. Initialize protocol versions were not validated; Streamable HTTP reused a constant header and did not DELETE discarded sessions.
7. Response routing accepted only unsigned numeric ids.

## Implemented resolution

- Route server requests/notifications before pending responses; answer ping and reject unsupported requests with `-32601`.
- Normalize numeric and numeric-string response ids.
- Page `tools/list` to completion with cursor validation and loop guards.
- Use a stdio tool revision plus aggregate-cache invalidation for `tools/list_changed`.
- Require confirmation for MCP tools under default policy regardless of annotations; explicit user `auto` remains authoritative.
- Send best-effort `notifications/cancelled` after non-initialize timeouts and never retry unknown-outcome tool calls.
- Validate negotiated versions, use the negotiated HTTP header, and DELETE HTTP sessions on teardown.

## Deliberate remaining scope

- Legacy `2024-11-05` HTTP+SSE transport is not implemented. Stdio accepts that protocol version; Streamable HTTP rejects it explicitly.
- Streamable HTTP persistent server-notification subscription is not implemented, so HTTP `tools/list_changed` push refresh requires a transport enhancement.
- Resources and prompts are product features, not correctness fixes in this task.
