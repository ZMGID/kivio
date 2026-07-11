# MCP Tool Discovery Cache

> Applies to `src-tauri/src/mcp/registry.rs` aggregate chat-tool discovery, startup MCP warmup in `src-tauri/src/lib.rs`, and the headless `src-tauri/src/kivio_code/mcp_setup.rs` MCP path.

## Completeness contract

- The long-lived chat tool-list cache may contain only a complete snapshot of every currently eligible enabled MCP server.
- Native and skill tools may still be returned when one MCP server fails discovery so the UI remains usable, but that degraded aggregate is transient and must not enter the five-minute cache.
- A later request must retry failed MCP discovery immediately rather than inheriting a stale native-only snapshot.
- A running MCP child process is not proof that its tools were sent to the model. The authoritative evidence is the tool-definition array recorded for the actual model request.

## Startup ordering

- The chat UI may prefetch `chat_mcp_list_tools` while asynchronous MCP warmup is still running.
- After startup warmup settles, clear the aggregate chat-tool cache. This invalidates any snapshot assembled against pre-warm session state.
- Do not solve startup ordering with an arbitrary sleep. Session single-flight and cache invalidation are the synchronization mechanisms.

## Failure behavior

- One failed MCP server must not block native tools or healthy MCP servers from being returned.
- Log both the per-server listing error and that the aggregate was intentionally not cached.
- Do not silently treat a partial list as a healthy cache hit.
- Tool-discovery reconnects use bounded exponential cooldown after repeated failures so a dead server cannot impose a full spawn/handshake timeout on every chat turn.
- Explicit tool execution bypasses discovery cooldown and attempts reconnection immediately; cooldown must never make a user-requested stale tool artificially unavailable.

## Last-known schema snapshots

- The last successful non-empty tool schema is independent from the live session and is persisted under the usage directory.
- A snapshot is keyed by server id and a hashed fingerprint of the exact server configuration that produced it. Never persist raw credentials in the fingerprint.
- Idle reap, reload, disconnect, and process restart may discard sessions without discarding a matching last-known schema.
- A changed configuration must never reuse a snapshot from the previous fingerprint. Until the new configuration successfully lists tools, that server is unreachable rather than degraded-with-known-tools.
- Successful initialization and a successful refresh after `notifications/tools/list_changed` both replace the stored snapshot.

## Required regression

The cache-policy regression must demonstrate:

1. a degraded native-only result is not cached;
2. a subsequent recovered result containing the MCP tool can be cached;
3. the recovered cache contains the model-facing MCP tool id.

For live validation, restart Kivio, clear request-debug records, and verify that the first new model request already includes the expected `mcp__<server>__<tool>` definition without a user reminder.

## Runtime eligibility and invalidation

- Resolve the enabled MCP server snapshot once per aggregate build. The same snapshot must drive both the cache key and `tools/list`; do not recompute plugin gates after the cache lookup.
- Plugin-backed servers are eligible only when the settings entry is enabled and the plugin is both enabled and installed. Include the eligible server ids in the aggregate cache key because plugin meta/binary state can change without changing settings.
- Startup warmup, GUI aggregate discovery, headless tool collection/status probing, and both GUI/headless tool execution must share the same runtime eligibility predicate. A child process is not evidence that its tools should be exposed.
- An explicit MCP reload is also a tool-schema refresh action and must invalidate the aggregate tool-list cache.
- Idle session reaping does not require aggregate invalidation: reconnecting the same configuration preserves the advertised schema. Configuration fingerprints rebuild the session when server config changes.

## Dynamic schema limitation

Stdio `notifications/tools/list_changed` invalidates the aggregate cache and advances a per-connection revision; the next discovery fetches every page again and persists the refreshed schema. Streamable HTTP push notifications remain unsupported until a persistent server-event listener exists.
