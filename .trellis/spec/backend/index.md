# Backend Development Guidelines

> Runtime and Tauri backend conventions for this project.

## Guidelines Index

| Guide | Description | Status |
|---|---|---|
| [Agent Runtime](./agent-runtime.md) | Chat agent loop, tool execution, and transcript contracts | Active |
| [File Tools](./file-tools.md) | Minimal native file tool surface (read/edit/write), runtime-side protection contracts | Active |
| [HTTP Timeouts](./http-timeouts.md) | Provider HTTP client timeout contracts for normal requests, streaming SSE, downloads, and retries | Active |
| [Lens Chat Handoff](./lens-chat-handoff.md) | Lens-to-Chat transfer command, screenshot attachment handoff, and navigation events | Active |
| [MCP Connection Lifecycle](./mcp-connection.md) | Persistent MCP session pool, liveness/reconnect, idle reaping, image artifacts, status events | Active |
| [Native Tool Registry](./native-tool-registry.md) | Static NATIVE_TOOLS table: single source for built-in tool exposure, parallel/approval metadata, and dispatch | Active |
| [Skill Slash Commands](./skill-commands.md) | Skill `/`-trigger, `$ARGUMENTS` substitution, run-level registry cache, dynamic allowed_tools, read_file cap | Active |
| [Window Lifecycle](./window-lifecycle.md) | Tauri window restore/open behavior for Chat, Lens, translator, and macOS Dock activation | Active |

## Pre-Development Checklist

- Read [Agent Runtime](./agent-runtime.md) before changing `src-tauri/src/chat/agent/**`, provider replay messages, or tool execution behavior.
- Read [File Tools](./file-tools.md) before changing `src-tauri/src/native_tools/files.rs`, native file tool schemas in `src-tauri/src/mcp/types.rs`, file mutation dispatch in `src-tauri/src/mcp/registry.rs`, or Chat UI rendering of file tool results.
- Read [HTTP Timeouts](./http-timeouts.md) before changing `src-tauri/src/api.rs`, provider adapters in `src-tauri/src/chat/model/**`, `send_with_retry`, `send_with_failover`, or any SSE streaming request/response loop.
- Read [Lens Chat Handoff](./lens-chat-handoff.md) before changing `lens_send_to_chat`, Lens screenshot cleanup, Chat conversation routing, or Lens-to-Chat attachment transfer behavior.
- Read [MCP Connection Lifecycle](./mcp-connection.md) before changing `src-tauri/src/mcp/manager.rs`, the MCP branch of `call_tool`/`list_*` in `src-tauri/src/mcp/registry.rs`, `AppState.mcp_sessions`, MCP warmup/reaper/exit hooks in `main.rs`, or `parse_tool_result` image handling.
- Read [Native Tool Registry](./native-tool-registry.md) before adding a built-in tool or changing tool exposure gates, parallel/approval/read-only classification, or `call_native_tool` dispatch — the only edit point is `src-tauri/src/mcp/native_registry.rs`.
- Read [Skill Slash Commands](./skill-commands.md) before changing `substitute_arguments`, `SkillRunCache`/`registry_for`, `find_by_trigger`, `try_apply_skill_slash_trigger`, dynamic `allowed_tools` filtering in `chat/agent/{prepare,loop_}.rs`, `read_skill_file`, or the `/` popover in `InputBar.tsx`/`slashCommands.ts`.
- Read [Window Lifecycle](./window-lifecycle.md) before changing `src-tauri/src/main.rs` app reopen handling, `src-tauri/src/shortcuts.rs` Chat/window activation paths, or `src-tauri/src/windows.rs` window chrome/behavior helpers.

## Quality Check

- Run targeted Rust tests for the changed backend area.
- Run `cargo test --manifest-path src-tauri/Cargo.toml` when practical.
- For Chat agent changes, verify provider-compatible replay messages and deterministic tool result ordering.
- For Lens-to-Chat handoff changes, verify the screenshot survives Lens close cleanup and the Chat window navigates to the target conversation.
- For macOS Chat window restore changes, smoke-test an installed app by minimizing Chat to the Dock, clicking the Dock icon, and confirming the restored window behaves like a normal non-floating app window.
