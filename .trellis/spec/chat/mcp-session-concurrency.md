# MCP Session Concurrency

> Applies to `src-tauri/src/mcp/manager.rs`: MCP session lifecycle, stdio JSON-RPC request routing, timeout handling, reload, disconnect, and idle cleanup.

## Core contract

- A session lock protects lifecycle transitions; it must not serialize ordinary stdio RPC response waits.
- A stdio connection may have multiple in-flight requests. Match responses by JSON-RPC request id through independent pending entries.
- Serialize only the physical stdin write. Release the stdin lock immediately after the complete request line is flushed.
- Waiting for request A must not prevent request B from being written or receiving its response.
- Keep Streamable HTTP session-id mutation serialized unless that transport is redesigned around an equivalent explicit concurrency contract.

## Timeout and retry semantics

- A `tools/call` timeout has an unknown execution outcome. The server may have applied a mutation even when Kivio did not receive the response.
- Remove only the timed-out request's pending entry.
- Do not kill an otherwise healthy stdio process merely because one response timed out.
- Do not automatically replay a timed-out tool call. Non-idempotent calls such as add, set, remove, and batch may already have executed.
- The error must state that the outcome is unknown and that the request was not retried.
- Reconnect and retry are allowed only when the transport is genuinely dead or closed, while preserving the existing single-flight reconnect behavior.

## Lifecycle safety

- Configuration changes rebuild the session.
- Reload and disconnect intentionally terminate the old connection and fail its pending requests.
- Update `last_used` when a request starts and after success so normal in-flight work is not mistaken for an idle session.
- Review idle timeout versus tool timeout whenever either value changes.

## Required regression shape

Use a fake stdio MCP server that:

1. accepts request A and deliberately drops only A's response;
2. continues reading stdin;
3. accepts request B and returns B's response;
4. records call counts.

The regression must prove that B completes before A's timeout barrier, A reports unknown outcome / not retried, and A is observed exactly once. A test that merely waits for both futures is insufficient because it can pass while head-of-line blocking still exists.

## Real-server validation

When an installed MCP server exposes the original failure mode, run a local-only live probe through `AppState::mcp_call_tool`. Do not keep that test in CI when it depends on a machine-local binary. Record the server version, exact sequence, timeout, and relative completion times.
