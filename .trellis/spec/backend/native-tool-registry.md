# Native Tool Registry

> Single source of truth for built-in (native) tool metadata and dispatch.

## Design Stance (2026-06-12)

Before P0.c, adding one native tool required editing 5-6 hardcoded lists that silently drifted: the `list_native_builtin_tool_defs` if-chain (`mcp/types.rs`), the 17-arm `call_native_tool` match (`mcp/registry.rs`), `BUILTIN_NAMES` + `builtin_tool_bypasses_approval` (`chat/agent/prepare.rs`), the parallel whitelist (`chat/agent/rounds.rs`), the executor todo special-case (`chat/commands.rs`), and the `is_read_only_tool` native arm (`mcp/types.rs`). All of these now read from one static table.

**A static table, not runtime registration**: `&'static [NativeToolDef]` is lock-free, testable, and sufficient for a desktop app. MCP and skill tools remain dynamic sources layered on top by the registry list phase (TTL cache unchanged).

## Scenario: Adding or Changing a Native Tool

### 1. Scope / Trigger

- Trigger: adding a built-in tool, changing a tool's exposure gate, parallel/approval/read-only classification, or dispatch path.
- The ONLY place to do this is `src-tauri/src/mcp/native_registry.rs` (`NATIVE_TOOLS`). Do not reintroduce name lists elsewhere.

### 2. Signatures

```rust
// src-tauri/src/mcp/native_registry.rs
pub(crate) struct NativeToolDef {
    pub name: &'static str,
    pub def: fn(&Settings) -> ChatToolDefinition,   // schema constructor
    pub enabled: fn(&Settings) -> bool,             // exposure gate
    pub parallel_safe: bool,                        // narrow read-only set ONLY
    pub bypasses_approval: bool,
    pub read_only: bool,
    pub call: NativeToolCall,                       // enum dispatch
}
pub(crate) static NATIVE_TOOLS: &[NativeToolDef] = &[ /* ordered */ ];
```

- `NativeToolCall` variants encode the execution strategy: `SyncText` / `SyncResult` / `BlockingText` / `BlockingMutation` (spawn_blocking + per-path locks) / `Async` / `Conversation` (needs conversation_id, dispatched before workspace resolution) / `HostMediated` (e.g. `ask_user`, intercepted upstream in execute.rs; registry arm errors).
- Consumers read the table: `list_native_builtin_tool_defs` (exposure), `call_native_tool` (dispatch), `builtin_tool_names` (prepare), `builtin_tool_bypasses_approval`, `tool_call_parallel_eligible`, `is_read_only_tool`.

### 3. Contracts

- **Table order is exposure order** — frozen by `registry_order_and_names_match_legacy_exposure_order`.
- `parallel_safe: true` is exactly the narrow set: `web_search`, `web_fetch`, `read_file`, `list_dir`, `search_files`, `glob_files`, `stat_path` — and parallel execution still requires `!tool_requires_approval` at the call site. Do not widen this set when adding tools (see [Agent Runtime](./agent-runtime.md)); `memory_read` and `memory_search` are read-only but NOT parallel-safe.
- `bypasses_approval: true` includes the memory tools (`memory_read`, `memory_modify`, `memory_search`), the conversation/host tools (`todo_write`, `todo_update`, `ask_user`), and the sub-agent tools (`agent`, `check_agent_result`, `list_agent_tasks`). Skill tools keep their own branch in `prepare.rs`.
- `memory_search` is a read-only L2 keyword-retrieval tool (gated by `chat_memory.enabled`, exposed via `list_native_builtin_tool_defs` right after `memory_read`/`memory_modify`). It scores `#`-led sections by query-token overlap (heading hits weighted) and returns top-N `{heading, snippet}`; it is the fuzzy counterpart to `memory_read`'s exact-heading match. Pure string/token matching only — no vector store, no new dependency.
- `Conversation` tools (`todo_write`/`todo_update`) have `enabled` constantly false — they are appended by the chat runtime, never exposed via `list_native_builtin_tool_defs`, and dispatch to `chat/todo.rs::handle_conversation_tool_call` (load → apply → save → emit `chat-todo` → result) without resolving a workspace.
- `mixer_generate_image` is not in the table (separate source); `prepare.rs` unions it via `EXTRA_BUILTIN_NAMES`.
- Removed tool names (`write_file_chunk`, `patch`, `begin/append/finish/abort_file_write`) must not be added (see [File Tools](./file-tools.md)).

### 4. Validation & Error Matrix

| Condition | Behavior |
|---|---|
| Unknown native tool name reaches `call_native_tool` | `Unknown native tool: {name}` error (exact legacy copy) |
| `HostMediated` tool reaches the registry arm | Same unknown-tool error; upstream execute.rs interception is the real path |
| Tool added to table without updating guard tests | Exposure-snapshot / set tests fail — update them deliberately, with review |

### 5. Good/Base/Bad Cases

- Good: a new `grep_files` tool = one impl function + one table entry; exposure, dispatch, parallel/approval metadata all follow automatically; guard tests updated in the same commit.
- Base: changing a tool's settings gate = editing only its `enabled` closure.
- Bad: adding a name check (`matches!(tool.name.as_str(), ...)`) in prepare/rounds/commands instead of a table field — recreates the drift the registry eliminated.

### 6. Tests Required

Guard tests live in `native_registry.rs` and are mutation-verified:

- `builtin_exposure_snapshot_per_settings_combination` — exposure per settings gate combination (the most important regression rail).
- `parallel_safe_set_is_exactly_the_narrow_read_whitelist` (exactly 7).
- `approval_bypass_set_matches_legacy_list` (exactly 5).
- `read_only_set_matches_legacy_is_read_only_tool_arm`.
- `registry_order_and_names_match_legacy_exposure_order`, `registry_defs_match_entry_names`, `conversation_tools_are_never_listed_via_builtin_exposure`.
- Run `cargo test --manifest-path src-tauri/Cargo.toml mcp::native_registry` after any table change, plus full `cargo test`.

### 7. Wrong vs Correct

#### Wrong

```rust
// somewhere far from the registry
if matches!(name, "read_file" | "list_dir" | "my_new_tool") { ... }
```

#### Correct

```rust
// native_registry.rs — one new entry, metadata travels with the tool
NativeToolDef { name: "my_new_tool", def: my_new_tool_def, enabled: |s| s.chat.native_tools.read_file, parallel_safe: false, bypasses_approval: false, read_only: true, call: NativeToolCall::Async(my_new_tool_call) },
```
