# Research: Native (built-in) tool inventory

- **Query**: Map every native/built-in agent tool — def fn, handler fn, enabled-gate, read-only?, sensitive?, config toggle
- **Scope**: internal
- **Date**: 2026-07-03

## Findings

### The static registry is the single source of truth

`src-tauri/src/mcp/native_registry.rs` — `NATIVE_TOOLS: &[NativeToolEntry]` (declared `native_registry.rs:118`) is the ordered table that drives exposure, dispatch, parallel/approval/read-only classification, and session-consent gating. The module header (`native_registry.rs:1-24`) explicitly says this table replaced five previously-drifting hardcoded lists. `find_entry(name)` (`native_registry.rs:367`) is the lookup used everywhere.

Each `NativeToolEntry` (struct at `native_registry.rs:96-113`) carries: `name`, `def: fn() -> ChatToolDefinition`, `enabled: NativeToolEnabledFn` (`fn(&ChatNativeToolsConfig, web_search_configured, memory_enabled) -> bool`), `parallel_safe`, `bypasses_approval`, `read_only`, `requires_session_consent`, and `call: NativeToolCall`.

Exposure list built by `list_native_builtin_tool_defs` (`mcp/types.rs:879-889`): filters `NATIVE_TOOLS` by `(entry.enabled)(...)` and maps `(entry.def)()`. Declaration order = model-facing order.

### Native tools table (registry order)

| # | name | def fn (types.rs) | handler (fn / file:line) | enabled gate | read-only | sensitive | config toggle |
|---|---|---|---|---|---|---|---|
| 1 | `web_search` | `native_web_search_tool` (types.rs:217) | `call_web_search` (native_registry.rs:442) → `web_search::search_web` | `native.web_search && web_search_configured` | ✅ | ✕ | `webSearch` |
| 2 | `web_fetch` | `native_web_fetch_tool` (types.rs:816) | `call_web_fetch` (native_registry.rs:478) → `native_tools::web_fetch` | `native.web_fetch` | ✅ | ✕ | `webFetch` |
| 3 | `knowledge_search` | `native_knowledge_search_tool` (types.rs:844) | `call_knowledge_search` (native_registry.rs:488) | `native.knowledge_search` | ✅ | ✕ | (backend `knowledge_search`; **not in FE `ChatNativeToolsConfig`**) |
| 4 | `read` | `native_read_file_tool` (types.rs:367) | `call_read_file` (native_registry.rs:392) → `native_tools::read_file` (files.rs:108) | `native.read_file` | ✅ | ✕ | `readFile` |
| 5 | `ls` | `native_list_dir_tool` (types.rs:390) | `native_tools::list_dir` (files.rs:1016) [`SyncText`] | `native.read_file` | ✅ | ✕ | `readFile` |
| 6 | `grep` | `native_search_files_tool` (types.rs:412) | `native_tools::search_files` (files.rs:1124) [`SyncText`] | `native.read_file` | ✅ | ✕ | `readFile` |
| 7 | `find` | `native_glob_files_tool` (types.rs:442) | `native_tools::glob_files` (files.rs:1074) [`SyncText`] | `native.read_file` | ✅ | ✕ | `readFile` |
| 8 | `write` | `native_write_file_tool` (types.rs:466) | `native_tools::write_file` (files.rs:255) [`BlockingMutation`] | `native.write_file` | ✕ | ✅ | `writeFile` |
| 9 | `edit` | `native_edit_file_tool` (types.rs:488) | `native_tools::edit_file` (files.rs:301) [`BlockingMutation`] | `native.edit_file` | ✕ | ✅ | `editFile` |
| 10 | `bash` | `native_run_command_tool` (types.rs:522) | `call_run_command` (native_registry.rs:685) → `native_tools::run_command` | `native.run_command` | ✕ | ✅ | `runCommand` |
| 11 | `bash_output` | `native_bash_output_tool` (types.rs:547) | `call_bash_output` (native_registry.rs:698) → `native_tools::bash_output` | `native.run_command` | ✅ | ✕ | `runCommand` |
| 12 | `list_background` | `native_list_background_tool` (types.rs:569) | `call_list_background` (native_registry.rs:705) → `native_tools::list_background` | `native.run_command` | ✅ | ✕ | `runCommand` |
| 13 | `kill_background` | `native_kill_background_tool` (types.rs:587) | `call_kill_background` (native_registry.rs:712) → `native_tools::kill_background` | `native.run_command` | ✕ | ✅ | `runCommand` |
| 14 | `save_assistant` | `native_save_assistant_tool` (types.rs:608) | `call_save_assistant` (native_registry.rs:719) | `false` (manual append only) | ✕ | ✕ | — |
| 15 | `run_python` | `native_run_python_tool` (types.rs:635) | `call_run_python` (native_registry.rs:726) | `native.run_python` | ✕ | ✕ | `runPython` |
| 16 | `memory_read` | `native_memory_read_tool` (types.rs:663) | `call_memory_read` (native_registry.rs:656) | `memory_enabled` | ✅ | ✕ | (chat memory setting) |
| 17 | `memory_modify` | `native_memory_modify_tool` (types.rs:696) | `call_memory_modify` (native_registry.rs:667) | `memory_enabled` | ✕ | ✕ | (chat memory setting) |
| 18 | `memory_search` | `native_memory_search_tool` (types.rs:743) | `call_memory_search` (native_registry.rs:676) | `memory_enabled` | ✅ | ✕ | (chat memory setting) |
| 19 | `todo_write` | `chat::todo::todo_write_tool` (todo.rs:65) | `todo::handle_conversation_tool_call` (todo.rs:282) [`Conversation`] | `false` (appended in commands.rs) | ✕ | ✕ | — |
| 20 | `todo_update` | `chat::todo::todo_update_tool` (todo.rs:95) | `todo::handle_conversation_tool_call` (todo.rs:282) [`Conversation`] | `false` (appended in commands.rs) | ✕ | ✕ | — |
| 21 | `ask_user` | `chat::ask_user::ask_user_tool` | intercepted in `execute.rs::execute_ask_user_call` [`HostMediated`] | `false` (appended in commands.rs) | ✕ | ✕ | — |
| 22 | `agent` | `chat::sub_agent::agent_tool` | `sub_agent::dispatch_agent_spawn` [`SubAgent`] | `false` (appended in commands.rs) | ✕ | ✕ | — |

`NativeToolCall` variants (dispatch mode) at `native_registry.rs:69-94`: `SyncText`, `SyncResult`, `BlockingText`, `BlockingMutation`, `Async`, `Conversation`, `HostMediated`, `SubAgent`.

### Skill tools — NOT in the static registry

Skill tools have `source: "skill"` and are assembled by a separate path:
- Defs: `native_skill_activate_tool` (types.rs:270, id `skill__activate`, name `skill_activate`), `native_skill_read_file_tool` (types.rs:294, name `skill_read_file`), `native_skill_run_script_tool` (types.rs:322, name `skill_run_script`). Bundled by `native_skill_tools()` (types.rs:359).
- Exposure gate: `list_skill_tool_defs` (registry.rs:617) returns them iff `skill_runtime_tools_enabled` (registry.rs:613) → `settings.chat_tools.native_tools.skill_runtime` (FE toggle `skillRuntime`, tauri.ts:402).
- Dispatch: `call_skill_tool` (registry.rs:523-611), a `match tool.name` over `skill_activate` / `skill_read_file` / `skill_run_script`.
- All three are not `sensitive` (types.rs test `builtin_skill_tools_are_not_marked_sensitive`, types.rs:950) and bypass approval via `builtin_tool_bypasses_approval` (prepare.rs:182-189, special-cases `tool.source == "skill" && is_native_skill_tool_name`).

### mixer_generate_image

`mixer_generate_image_tool` (types.rs:776), `source: "mixer"`. Treated as a Kivio built-in by `is_kivio_builtin_tool` (prepare.rs:176) and listed in `EXTRA_BUILTIN_NAMES` for disabled-tool feedback (prepare.rs:155).

### is_read_only_tool

`ChatToolDefinition::is_read_only_tool` (types.rs:84-95): for `source == "native"` it defers to `native_registry::find_entry(&self.name).read_only`. MCP uses annotation hints.

### Frozen classification-set tests (must stay in sync)

In `native_registry.rs` tests:
- `EXPECTED_ORDER` (native_registry.rs:762-785) — full ordered name list incl. `ls`, `find`, `list_background`, `todo_update`.
- `session_consent_set_is_exactly_the_seven_file_shell_tools` (native_registry.rs:788) — hardcodes `[read, ls, grep, find, write, edit, bash, bash_output, list_background, kill_background]`.
- `registry_order_and_names_match_legacy_exposure_order` (native_registry.rs:820).
- `registry_defs_match_entry_names` (native_registry.rs:827).
- `parallel_safe_set_is_exactly_the_narrow_read_whitelist` (native_registry.rs:836) — incl. `ls`, `find`, `list_background`.
- `read_only_set_matches_legacy_is_read_only_tool_arm` (native_registry.rs:887) — incl. `ls`, `find`, `list_background`.
- `approval_bypass_set_matches_legacy_list` (native_registry.rs:866) — incl. `todo_update`.
- `builtin_exposure_snapshot_per_settings_combination` (native_registry.rs:947) — asserts `["read","ls","grep","find"]` for the read gate, and full ordered surface.

In `mcp/types.rs` tests:
- `file_tool_path_descriptions_are_scope_specific` (types.rs:969) — reads `native_list_dir_tool` (`ls`) + `native_glob_files_tool` (`find`).
- `default_native_config_exposes_file_and_command_tools` (types.rs:1067) — asserts `["read","ls","grep","find","write","edit","bash"]`.

## Config toggles → tools (backend `ChatNativeToolsConfig`)

`ChatNativeToolsConfig` fields seen in tests (native_registry.rs:914-925, 962-973): `web_search, web_fetch, skill_runtime, read_file, write_file, edit_file, run_command, run_python, knowledge_search, workspace_roots`.
- `read_file` → gates **read, ls, grep, find** (one toggle, four tools).
- `run_command` → gates **bash, bash_output, list_background, kill_background** (one toggle, four tools).
- `skill_runtime` → gates **skill_activate, skill_read_file, skill_run_script** (via `list_skill_tool_defs`).

Frontend mirror `ChatNativeToolsConfig` (tauri.ts:399-409) + `defaultNativeTools()` (tauri.ts:435-450): `webSearch, webFetch, skillRuntime, readFile, writeFile, editFile, runCommand, runPython, workspaceRoots`. **Note: FE type omits `knowledgeSearch`** — the backend gate exists but the FE type doesn't declare it.

## Caveats / Not Found

- `MAX_READ_FILE_BYTES` and the sensitive-path write guards (`resolve_tool_write_path`, `.ssh`/`.gnupg`/Keychains blocking) live in `src-tauri/src/native_tools/mod.rs` (imported at files.rs:15-18); exact lines not opened this pass.
- Did not enumerate a Settings-panel toggle UI mapping; the native-tools toggles surface through `ChatNativeToolsConfig` but the specific `SettingsShell.tsx`/panel components were not opened (see risks doc open question).
