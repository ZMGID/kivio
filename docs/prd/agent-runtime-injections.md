# Agent runtime injections

Kivio routes runtime context through the SDK's ordered asynchronous injection sources. The SDK supplies admission, typed queues, per-kind batching, lazy freshness checks, receipts, clear/unregister and readiness signals. Product rules and autonomous run scheduling stay in the application.

| Source | Application owner | Delivery |
| --- | --- | --- |
| Child task results | Subagent delivery | Existing claim/save/commit transaction publishes committed inputs |
| Agent IRC | IRC bus and repository | Saved messages publish committed inputs |
| Paired advisor | Child policy checkpoint | Saved advice publishes committed inputs with existing digest freshness |
| Background shell | Background command registry | First terminal transition queues an owner-scoped notice with a `proc://` output reference |
| MCP resource change | Authorized resource listener | Resource URI notices; content is read only on request |
| Late LSP diagnostics | Native LSP service | File/server version checked again when the model receives the notice |
| Todo | Todo injection source | Recomputes reminders from current tool results at each boundary |
| Continuation notes | Context-notes reminder | Near-budget reminder uses existing `write` and `read` tools |

Main notifications reuse the idle send reservation. Child notifications reuse the IRC wake reservation and parked-session revival. Busy runs consume notices at their next boundary. User stop suppresses autonomous waking. Plan-mode child notices wait for an allowed run. LSP and continuation-note reminders do not start idle turns.

The queue is volatile. `Applied` means that an input entered live runtime history. It is not a durable delivery receipt or proof that the provider read the message. Durable child results and IRC retain their existing repository ownership. A later source failure cannot discard facts already committed by an earlier save.

## MCP resources

The native `read` tool accepts `mcp://` to discover the run's authorized servers. `mcp://<server-id>` lists resources. The returned encoded paths read resources and subscribe when the server supports subscriptions. Listeners start before subscription requests and filter both the runtime's authorized server IDs and the URIs it has read. Removing authority, disconnecting or reconnecting invalidates old queued notices. Subscription lag produces a notice instead of pretending that every update arrived.

Server grants come from enabled eligible servers intersected with the conversation's allowed servers. They do not require the server to expose a tool. Child grants remain within the parent's server grants and any saved session contract. Tool grants still contain only tools exposed by the run; granting resource access does not grant additional tool calls. Resource reads also recheck the current server configuration.

## LSP configuration

Configure installed language servers in the existing application settings JSON. `chatTools.lspServers` defaults to an empty array. Kivio does not install executables or enable commands from workspace `.lsp.json` or plugin `lspServers` settings.

```json
{
  "chatTools": {
    "lspServers": [
      {
        "command": "language-server-command",
        "args": ["--stdio"],
        "languageId": "typescript",
        "extensions": ["ts", "tsx"]
      }
    ]
  }
}
```

Replace the command and arguments with the installed server's stdio invocation. Successful file writes and edits notify matching servers. No-op writes do not advance document versions. Processes and documents are scoped to the actual main or child runtime and canonical workspace. Multiple configured servers can share a language ID. Removing a configuration closes its sessions on the next run or mutation.

Kivio supports versioned push diagnostics and request-correlated pull diagnostics. After a file changes, versionless push diagnostics cannot be attributed safely and are ignored. Servers providing only ambiguous versionless pushes will not produce late diagnostics after subsequent edits. Frames, files, document counts, sessions and shutdown waits are bounded.

## Continuation notes

When the estimated context approaches the compaction budget and both native file tools are available, the model receives one reminder per context window. The suggested `.kivio/context-notes/` filename includes the runtime identity and starting history entry. The model decides whether to write a note under normal tool authorization. This adds a reminder using Kivio's file tools; it does not add OMP's `context_notes` or `new_context` tools.

## Verification

The SDK consumer tests exercise actual model requests, committed partial history, lazy stale filtering, receipt outcomes, clear/unregister races and user-input precedence. App tests use real short shell processes and local fake MCP/LSP stdio servers. They check owner and URI isolation, connection replacement, old document versions, process cleanup and queued delivery to the model. SDK contracts and the offline injection example document the public API.
