# MCP tool definitions missing from the first model request (2026-07-11)

## User-visible evidence

Conversation: `conv_27c2b09b-942a-448a-b50d-19625d58bf71`.

- 10:00:28 (Asia/Hong_Kong): user starts the PPT task.
- The request-debug panel records 19 tool definitions and no OfficeCLI MCP tool.
- The assistant consequently uses native shell/read tools and makes zero OfficeCLI MCP calls.
- After the user asks Kivio to check installation, a later request records 21 tools and successfully calls `mcp__plugin-officecli__officecli`.
- The OfficeCLI MCP child process existed before the first task, proving that process existence and model tool exposure are separate states.

## Root cause

`mcp::registry::list_enabled_tool_defs` previously handled an MCP `tools/list` failure by:

1. logging the server error;
2. returning the remaining native/skill tools;
3. caching that incomplete aggregate under the normal five-minute TTL.

The chat UI calls `chat_mcp_list_tools` during idle startup prefetch. If this overlaps MCP startup or otherwise encounters a transient listing failure, it can seed the shared aggregate cache with the 19-tool native-only list. The actual model request then hits that cache and has no ability to call OfficeCLI. Once the cache is rebuilt after recovery, the count becomes 21.

## Fix

- Cache aggregate chat tools only if every eligible enabled MCP server listed successfully.
- Continue returning partial tools for graceful UI degradation, but leave them uncached so the next request retries discovery.
- Clear the aggregate cache after asynchronous startup warmup settles to invalidate any pre-warm snapshot.
- Add a regression proving partial results are not cached and a recovered OfficeCLI tool id is cached.

## Live validation

The Tauri dev watcher rebuilt and restarted Kivio at 2026-07-11 10:30:30 (Asia/Hong_Kong). The OfficeCLI MCP child started at 10:30:30. The first fresh headless chat probe after that restart (`mcp-first-request-cache-fix-finalbuild-20260711-1030`) was instructed to avoid shell tools and call `["--version"]` exactly once.

Result:

- tool call: `officecli` (MCP), one call;
- arguments: `{"command":["--version"]}`;
- status: success;
- answer: `1.0.135`;
- shell calls: zero.

Because `officecli` has no native-tool fallback, this proves the OfficeCLI MCP definition was present in the first post-restart model request and dispatch succeeded without a user reminder.


## Second-pass audit (2026-07-11)

The full startup -> eligibility -> aggregate cache -> reload -> model request chain exposed two additional cache-consistency gaps:

1. The aggregate key serialized settings only, while plugin eligibility came from runtime plugin meta and binary detection. If those gates changed without a settings write, a native-only aggregate could remain valid under the old key for five minutes. The fix computes one eligible-server snapshot, includes its server ids in the key, and reuses that exact snapshot for listing.
2. `chat_mcp_reload_server` discarded the session but left the aggregate definitions cached. Reload now clears the chat tool-list cache so the next request performs a fresh discovery.

Startup warmup and MCP tool execution now use the same runtime eligibility predicate as discovery. This prevents warming disabled/uninstalled plugin servers and prevents executing a stale cached plugin tool after its runtime gate closes.

Verification on the rebuilt backend:

- registry tests: 5 passed;
- manager tests: 7 passed;
- `cargo check`: passed (existing warnings only);
- fresh probe `mcp-second-review-finalbuild-20260711-1058`: actual `officecli` MCP call ``["--version"]`` succeeded and returned `1.0.135`; zero shell calls.

One protocol-level limitation remains: session tool schemas are initialized once and `notifications/tools/list_changed` is not handled. Explicit reload now refreshes correctly, but automatic dynamic schema updates should be a separate enhancement.


## Headless path follow-up (2026-07-11)

A final cross-consumer search found that `kivio_code/mcp_setup.rs` still used `server.enabled` directly in tool collection and dispatch, and `/mcp` status could probe a plugin-backed server after its plugin runtime gate closed. The headless path now reuses `mcp_server_is_runtime_eligible` for collection, status probes, and execution. This keeps GUI chat and kivio-code from disagreeing about whether a plugin MCP is loadable.
