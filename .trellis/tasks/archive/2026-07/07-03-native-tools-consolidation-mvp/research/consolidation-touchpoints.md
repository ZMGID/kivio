# Research: Consolidation touchpoints (the 5 MVP changes)

- **Query**: For each of the 5 MVP tool-consolidation changes, list exact files+functions to edit, the alias mechanism to reuse, backward-compat concerns, and tests to update.
- **Scope**: internal
- **Date**: 2026-07-03

## Shared machinery you will reuse

### Name → handler dispatch
`match_tool_call(tools, function_name)` (`chat/agent/execute.rs:56-79`): matches a model tool call by `tool.openai_tool_name() == function_name || tool.name == function_name`, then a **case-insensitive unique** fallback (execute.rs:69-78). Native dispatch then goes `registry::call_native_tool` → `find_entry(&tool.name)` (registry.rs:634) → `entry.call`. Skill dispatch goes `call_skill_tool` `match tool.name` (registry.rs:566-601).

### The wire-alias mechanism (the `web_search`→`search_web` precedent)
- Table: `RESERVED_WIRE_ALIASES: &[(&str,&str)]` (`mcp/types.rs:11`).
- `apply_reserved_wire_alias(internal) -> wire` (types.rs:14); `resolve_reserved_wire_alias(wire) -> internal` (types.rs:24).
- Applied **only** in `ChatToolDefinition::openai_tool_name()` for sources `native|skill|mixer` (types.rs:50-59). So: tools declaration + system-prompt rendering show the wire name; `match_tool_call` maps the wire name back because it compares against `openai_tool_name()`; **all internal logic keeps using `tool.name`** (`find_entry`, `is_read_only_tool`, `parallel_safe`, `filter_tools_for_agent`, DSML). The only place that receives a wire name and reverses it is `disabled_builtin_tool_feedback` (prepare.rs:157). Spec: `.trellis/spec/chat/request-shape-contracts.md` C3 (lines 31-56).

**Important distinction for `find`→`glob`:** the wire-alias table changes the *model-facing* name while keeping `tool.name` internal. The MVP wants the opposite direction — make the new model-facing name the real `tool.name` (`glob`) and keep the *old* name (`find`) resolvable for stored allow-lists. That is an **allow-list/name-canonicalization** concern, not a wire concern (see change 5).

### System-prompt rendering of tool names
`native_tools_prompt` (prepare.rs:728-851) renders the enabled tool list via `apply_reserved_wire_alias` (prepare.rs:743-747) and has per-tool hint branches keyed on names: `read/ls/grep/find` (prepare.rs:453), `bash/run_python` (prepare.rs:459), `web_search/web_fetch` (prepare.rs:465), `skill_run_script` mentions (prepare.rs:815, 841), background tools mention `list_background`/`bash_output`/`kill_background` (prepare.rs:842). `action_examples` in `build_chat_system_prompt_with_segments` also keys on `read/ls/grep/find` (prepare.rs:453).

---

## Change 1 — Fold `ls` into `read` (read lists a dir), remove `ls`

**Behavior today:** `read_file` (files.rs:108-118) errors with `不是可读取的文件` when the path is not a file (files.rs:117-119). `list_dir` (files.rs:1016-1072) returns a JSON `{path, entries, truncated}` listing. `call_read_file` (native_registry.rs:392-421) short-circuits images (→ vision/OCR) and PDF/Word/Excel (→ skill hint) before falling to `read_file`.

**Edit:**
1. `native_tools/files.rs` — teach `read_file` (or `call_read_file`) to branch on `full.is_dir()`: when a directory, produce the `list_dir` listing instead of erroring. `list_dir` already takes `include_hidden`/`max_entries`; decide whether `read`'s `offset`/`limit` map onto listing pagination or are ignored for dirs. `resolve_tool_read_path` already handles both.
2. `mcp/types.rs` — update `native_read_file_tool` description (types.rs:371) to say it lists directories; **remove** `native_list_dir_tool` (types.rs:390-410) or keep it only if still referenced.
3. `mcp/native_registry.rs` — remove the `ls` `NativeToolEntry` (native_registry.rs:159-168). `read` stays.
4. Consider whether `call_read_file` should route dirs to `list_dir` (keeps images/docs branches intact).

**Backward-compat:** `ls` is referenced by name in built-in personas (agents/types.rs:46,62,78) and models trained on an `ls` tool. Option: add `ls` to a name-canonicalization/alias so a model call to `ls` still routes to `read` (reuse the case-insensitive fallback won't help — different name). Cleanest: add `("ls" -> "read")` handling in `match_tool_call` or a dedicated legacy-alias table (see change 5 discussion — one shared alias mechanism can serve both renames/removals).

**Tests to update:** `native_registry.rs` `EXPECTED_ORDER` (762), `session_consent_set...` (788), `parallel_safe_set...` (836), `read_only_set...` (887), `builtin_exposure_snapshot...` (947, asserts `["read","ls","grep","find"]`), `default_native_config_exposes...` (types.rs:1067), `file_tool_path_descriptions_are_scope_specific` (types.rs:969, reads `native_list_dir_tool`), `simulated_agent_session...` (files.rs:1587-1594 asserts "exactly Pi's 7" incl. `ls`). Persona test `researcher_is_read_only_set` (agents/types.rs:110) references `ls` indirectly. prepare.rs `action_examples` (prepare.rs:453).

---

## Change 2 — Remove `list_background` (keep `bash_output`, `kill_background`)

**Behavior today:** three entries all gated by `native.run_command` + session consent (native_registry.rs:224-253). `list_background` handler `call_list_background` (native_registry.rs:705) → `native_tools::list_background` (shell.rs).

**Edit:**
1. `mcp/native_registry.rs` — remove the `list_background` entry (native_registry.rs:234-243).
2. `mcp/types.rs` — remove `native_list_background_tool` (types.rs:569-585).
3. `native_tools/shell.rs` — `list_background` fn can be removed OR retained internally. **Design choice noted in task:** `bash_output` with no `job_id` could return the job list. Its schema requires `job_id` (types.rs:561), so relaxing that + branching in `native_tools::bash_output` would preserve discoverability.
4. Prompt text (prepare.rs:842) names `list_background` — update.

**Backward-compat:** low. Not referenced in personas. Model may still emit `list_background` → falls to unknown-tool self-heal (`unknown_tool_record`, execute.rs:81) + `disabled_builtin_tool_feedback` returns None (not a builtin anymore) → generic unknown feedback.

**Tests to update:** `EXPECTED_ORDER` (762), `parallel_safe_set...` (836, incl. list_background), `read_only_set...` (887, incl. list_background), `session_consent_set...` (788, incl. list_background), `builtin_exposure_snapshot...` full surface (947).

---

## Change 3 — Collapse skill trio into a single `skill` tool

**Behavior today:** three `source:"skill"` tools, exposed via `list_skill_tool_defs` (registry.rs:617) under one toggle `skill_runtime`, dispatched by `call_skill_tool` `match tool.name` (registry.rs:566-601). Handlers:
- `skill_activate` → `SkillRunCache::activate_with_cache` / `activate_skill` (runtime.rs:88, 238); records `allowed_tools` for T3 tool-narrowing (registry.rs:570, runtime.rs:76).
- `skill_read_file` → `read_file_with_cache` / `read_skill_file` (runtime.rs:101, 263). Resolves paths via `resolve_skill_path` (runtime.rs:117) which enforces skill-root containment (rejects `..`, canonicalizes, `starts_with(base)` — runtime.rs:152). Caps at `MAX_READ_FILE_BYTES` and appends a "use skill_run_script" marker (runtime.rs:269-285).
- `skill_run_script` → `run_skill_script` (runtime.rs:288-349): enforces `scripts/` prefix (runtime.rs:296), interpreter allowlist `skill_script_allowlist` (runtime.rs:377, passed at registry.rs:596), runs with `current_dir(base_dir)`, custom timeout (`effective_skill_script_timeout_ms`, referenced execute.rs:563-568).

**Edit (keep `skill_activate`, rename/alias to `skill`; drop the other two):**
1. `mcp/types.rs` — keep `native_skill_activate_tool` but rename model-facing name to `skill` (types.rs:270-292); remove `native_skill_read_file_tool` (types.rs:294) and `native_skill_run_script_tool` (types.rs:322); trim `native_skill_tools()` (types.rs:359).
2. `mcp/registry.rs` — `call_skill_tool` (registry.rs:566) drop the `skill_read_file`/`skill_run_script` arms; `effective_skill_script_timeout_ms` (registry.rs:587) becomes dead if run_script goes.
3. `chat/agent/prepare.rs` — `is_native_skill_tool_name` (prepare.rs:169-174) currently matches all three names; update to `skill` (this fn is load-bearing: used by `filter_tools_for_agent` filter.rs:36, `retain_tools_for_allowed` prepare.rs:79, `is_kivio_builtin_tool` prepare.rs:178, `builtin_tool_bypasses_approval` prepare.rs:183).
4. `chat/agent/execute.rs` — `effective_tool_timeout_ms` special-cases `skill_run_script` (execute.rs:563); drop.
5. `skills/runtime.rs` — `read_skill_file`, `run_skill_script`, `read_file_with_cache`, `resolve_skill_path`, `run_skill_script`, `build_script_command` become unused unless retained for another caller; the "Use skill_run_script to process the full file" marker (runtime.rs:282) becomes stale.
6. `chat/agent/stop.rs` — DSML extraction tests reference `skill_run_script` (stop.rs:267,280) and `skill_activate` (stop.rs:202).

**What breaks if `skill_read_file` / `skill_run_script` are removed (must be documented in PRD):**
- **Skill-root-relative path resolution** (runtime.rs:117-156): generic `read` resolves against project root/workspace, NOT the skill dir. Reading `references/guide.md` by a skill-relative path stops working; the model must be handed an absolute path (skill activation output already prints `Skill directory:` + absolute base — activate_skill runtime.rs:245, and `<skill_resources>` file list runtime.rs:248-258 — so absolute reads via generic `read` are viable).
- **Script allowlist enforcement** (`skill_script_allowlist`, runtime.rs:377): `bash`/`run_python` have their own gates (session consent + host-shell rules) but do NOT enforce the interpreter allowlist or the `scripts/`-prefix restriction. Running skill scripts via `bash` also loses the auto `current_dir(base_dir)` and the skill-specific timeout.
- **secrets/ dir access**: `skill_read_file`'s description advertises reading `references/, secrets/, etc.` (types.rs:298). Generic `read` under a project workspace enforces its own path guards; whether the skill `secrets/` dir is reachable depends on where the skill lives relative to the workspace (app-data skill dirs vs project). Verify a skill's `secrets/` is still readable via absolute path + `read`.

**Backward-compat / naming:** if the surviving tool is renamed `skill`, `is_native_skill_tool_name` and any persona/allow-list referencing `skill_activate` (none in built-in personas) must map. `native_skill_activate_tool().openai_tool_name()` asserted `== "skill_activate"` (types.rs:936).

---

## Change 4 — Merge `todo_update` into `todo_write`

**Behavior today:** both in `chat/todo.rs`. `apply_todo_write` (todo.rs:166) does a **full-list replace** (parses `todos[]`, normalizes single-in-progress invariant + dependency edges). `apply_todo_update` (todo.rs:176) mutates ONE item by `id` (content/description/status/blocks/blocked_by/owner) or deletes it. `apply_tool` (todo.rs:154) routes by name. Both appended via `todo::append_tool_definitions` (todo.rs:50) / `tool_definitions` (todo.rs:61).

**Does `todo_update` do anything `todo_write` can't?** Functionally no for state — `todo_write` replaces the whole list so any single-item change is expressible as a full rewrite. `todo_update` conveniences: (a) `delete` a single item with automatic reverse-edge cleanup (todo.rs:188-203, but `todo_write` + `normalized_state`/`sync_dependency_edges` also cleans dangling edges, todo.rs:381), (b) the `changed` receipt names the specific fields changed (todo.rs:219) vs `todo_write` always reports `["todos"]` (todo.rs:172), (c) `preferred_in_progress` disambiguation prefers the just-updated item (todo.rs:249). None are strictly unique to update.

**Edit:**
1. `chat/todo.rs` — remove `todo_update_tool` (todo.rs:95), `TodoUpdateArgs` (todo.rs:20), `apply_todo_update` (todo.rs:176), `TODO_UPDATE_TOOL_NAME` (todo.rs:12); simplify `is_agent_todo_tool_name` (todo.rs:46), `apply_tool` (todo.rs:154), `tool_definitions` (todo.rs:61).
2. `mcp/native_registry.rs` — remove the `todo_update` entry (native_registry.rs:323-332).
3. Prompt text: `format_prompt` (todo.rs:255-276) mentions `todo_write and todo_update` (todo.rs:260, 268) — update.

**Tests to update:** `EXPECTED_ORDER` (762), `approval_bypass_set_matches_legacy_list` (866, incl. todo_update); todo.rs unit tests that call `apply_todo_update` (todo.rs:572, 606, 656) must be rewritten against `todo_write`.

---

## Change 5 — Rename `find` → `glob` (keep backward-compat alias for `find`)

**Behavior today:** tool name is `find` (`native_glob_files_tool`, types.rs:442-464, note id is already `native__glob_files`). Handler `native_tools::glob_files` (files.rs:1074). Registry entry name `"find"` (native_registry.rs:180).

**Edit:**
1. `mcp/types.rs` — change `native_glob_files_tool` name from `"find"` to `"glob"` (types.rs:445). (id `native__glob_files` can stay.)
2. `mcp/native_registry.rs` — change entry name `"find"` → `"glob"` (native_registry.rs:181). All classification tests that list `find` (762, 788, 836, 887, 947, types.rs:1078, files.rs:1591) update to `glob`.
3. `chat/agent/prepare.rs` — `action_examples` match `"read"|"ls"|"grep"|"find"` (prepare.rs:453) → `glob`.
4. `agents/types.rs` — built-in personas list `"find"` (types.rs:48, 64, 80) → `glob` (or rely on alias).

**Backward-compat alias — the real design decision:** old stored references to `find` exist in:
- Built-in personas (agents/types.rs) — editable in-code, but user/project persona `.md` files (`<app_data>/agents/*.md`, `.kivio/agents/*.md`) parsed by `agents/parse.rs` (tools frontmatter, parse.rs:48) are user data and may say `find`.
- Sub-agent allow-list enforcement `filter_tools_for_agent` (filter.rs:16) → `tool_matches_recommended_name(tool, name)` (prepare.rs:713-726) compares `tool.name / tool.id / openai_tool_name() / server_id:name` against the stored allow name. If a persona says `find` but the tool is now `glob`, the match fails and the tool is stripped from that sub-agent.
- Skill `recommended_tools`/`allowed_tools` (used by `retain_tools_for_allowed`, prepare.rs:73) may list `find`.
- Model calls: `match_tool_call` (execute.rs:56) would miss a `find` call once the name is `glob`.

**Two viable alias approaches (pick one, note in PRD):**
- (a) A **legacy-name canonicalization table** (`find` → `glob`, and also `ls` → `read`, `skill_*` → `skill`) consulted in BOTH `match_tool_call` (execute.rs:56, for model calls) and `tool_matches_recommended_name` (prepare.rs:713, for stored allow-lists). This is the minimal, reuse-one-mechanism path.
- (b) Reuse `RESERVED_WIRE_ALIASES` — NOT suitable: that table changes the model-facing name away from `tool.name`, the opposite of what a rename needs.

**Tests:** `match_tool_call` tests (execute.rs:867, note `match_tool_call(&tools, "Glob").is_none()` at execute.rs:874 asserts no Glob match today — will change), persona tests (agents/types.rs:110), all registry classification-set tests.

## Cross-cutting: the per-round base_tools assembly

`chat/agent/loop_.rs` `base_tools` (loop_.rs:48-51, 143) is the FULL native+skill+mcp+conversation tool list; the effective per-round `tools` is recomputed from it (`recompute...`, loop_.rs:97-110) so skill activation can monotonically narrow. Assembled from `list_native_builtin_tool_defs` (types.rs:879) + `list_skill_tool_defs` (registry.rs:617) + `todo::append_tool_definitions` + ask_user + `agent` + MCP. Removing/renaming entries in the registry + `list_skill_tool_defs` propagates automatically to base_tools; the frozen tests are the guardrail.

## Caveats / Not Found

- Conversation-stored tool lists: personas store allow-lists; conversations themselves don't appear to persist a per-conversation tool-name allow-list (tool state is recomputed each run from settings + assistant snapshot). Not exhaustively verified — flagged as an open question in risks doc.
- `native_tools/shell.rs` internals (background job registry) not opened this pass; `list_background`/`bash_output`/`kill_background` fn signatures inferred from `native_registry.rs` call sites.
