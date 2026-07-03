# Research: Risks & open questions

- **Query**: Risks of the native-tools MVP consolidation + open questions the PRD/design must resolve.
- **Scope**: internal
- **Date**: 2026-07-03

## Risks

### R1 — Stored allow-list breakage on rename/removal (HIGH)
Sub-agent tool allow-lists are enforced by NAME. `filter_tools_for_agent` (filter.rs:16) → `tool_matches_recommended_name` (prepare.rs:713) matches a stored name against `tool.name / tool.id / openai_tool_name() / server_id:name`. If `find`→`glob` and `ls` is removed, personas that list `find`/`ls` (built-in agents/types.rs:46-80; user `<app_data>/agents/*.md` and project `.kivio/agents/*.md` via parse.rs:48) will have those tools **silently stripped** from the sub-agent — a research/reviewer sub-agent could lose file-listing entirely. Same for skill `allowed_tools`/`recommended_tools` referencing `skill_read_file`/`skill_run_script` in `retain_tools_for_allowed` (prepare.rs:73). Mitigation: one legacy-name canonicalization table consulted in both `match_tool_call` (execute.rs:56) and `tool_matches_recommended_name` (prepare.rs:713).

### R2 — Model tool-selection accuracy for merged tools (MEDIUM)
- `read` doing double duty (file + dir listing) diverges from the schema's `offset`/`limit` semantics; models may pass `offset` to a dir. The tool description (types.rs:371) and `native_tools_prompt` hints (prepare.rs:453) must teach the dir behavior or models will still reach for a removed `ls`.
- `skill` (single tool) loses the explicit `skill_run_script` affordance. The system prompt currently steers scripts to `skill_run_script` ("Skill 脚本走 skill_run_script", prepare.rs:815, 841); after the merge that instruction must flip to "run skill scripts via bash/run_python", which is a behavioral regression risk (see R4).
- Removing `list_background` relies on the model discovering jobs another way; if `bash_output` doesn't accept a no-`job_id` list mode, the model has no enumeration path.

### R3 — Case-insensitive fallback interaction (LOW/MEDIUM)
`match_tool_call` has a case-insensitive UNIQUE fallback (execute.rs:69-78) and today asserts `Glob` does NOT match (execute.rs:874). After renaming to `glob`, `Glob`/`GLOB` calls resolve via that fallback — good — but a legacy `find` alias must be added separately (different string, fallback won't help). Watch for ambiguity if a legacy alias collides with an MCP tool of the same name (the fallback already refuses ambiguous hits).

### R4 — Skill regressions from dropping skill_read_file / skill_run_script (HIGH — needs explicit PRD decision)
- **Path resolution**: `resolve_skill_path` (runtime.rs:117) resolves relative to the skill root and enforces containment. Generic `read` resolves relative to project root/workspace. Reading `references/x.md` by skill-relative path stops working; activation output does print the absolute `Skill directory:` (runtime.rs:245) + `<skill_resources>` list (runtime.rs:248), so absolute-path reads are feasible but the model must use them.
- **Allowlist + scripts/ restriction**: `run_skill_script` enforces `scripts/` prefix (runtime.rs:296) and the interpreter `skill_script_allowlist` (runtime.rs:377). `bash`/`run_python` enforce neither — a shift from a narrow, allowlisted execution surface to the full host shell (behind session consent). Also loses auto `current_dir(base_dir)` (runtime.rs:309) and the skill-specific timeout (`effective_skill_script_timeout_ms`, execute.rs:563).
- **secrets/ access**: `skill_read_file` advertised `secrets/` reads (types.rs:298). Whether generic `read` can reach a skill's `secrets/` depends on the skill's location vs the active workspace and the file-tool path guards.
- **Oversize-file marker**: `read_skill_file` truncation marker points at `skill_run_script` (runtime.rs:282) — stale after removal.

### R5 — Frozen classification-set tests will fail loudly (LOW risk, HIGH churn)
Many hardcoded name lists must be edited in lockstep or the suite fails: native_registry.rs `EXPECTED_ORDER` (762), `session_consent_set...` (788), `parallel_safe_set...` (836), `read_only_set...` (887), `approval_bypass_set...` (866), `builtin_exposure_snapshot...` (947); types.rs `default_native_config_exposes...` (1067), `file_tool_path_descriptions...` (969); files.rs `simulated_agent_session...` "exactly Pi's 7" (1591-1594); todo.rs update tests (572, 606, 656); execute.rs `match_tool_call` tests (867-884); agents/types.rs persona tests (104-116). This is a guardrail, not a defect — but it defines the true blast radius.

### R6 — Prompt/spec/doc drift (MEDIUM)
Text that names the tools and must change with the rename/removals:
- `chat/agent/prepare.rs::native_tools_prompt` — `read/ls/grep/find` (453), background tools incl. `list_background` (842), `skill_run_script` (815, 841), `bash_output`/`kill_background` (842).
- `chat/todo.rs::format_prompt` — "todo_write and todo_update" (260, 268).
- `.trellis/spec/chat/request-shape-contracts.md` — documents the wire-alias precedent (C3, lines 31-73); if the new legacy-alias table is added, this spec (or backend/file-tools.md / agent-runtime.md referenced in native_registry.rs:12-13) should record it.
- `CLAUDE.md` — the "Native tools" bullet names `read_file/write_file/edit_file/glob_files/search_files/list_dir/...` and `bash_output/list_background/kill_background`; update after the change.

### R7 — Frontend config mismatch (LOW)
FE `ChatNativeToolsConfig` (tauri.ts:399) already omits `knowledgeSearch`; the merges don't add/remove toggles (ls/find/list_background share existing toggles `readFile`/`runCommand`; skill trio shares `skillRuntime`; todo tools have no toggle). So **no FE toggle changes are required** for these 5 changes — the toggle→tool mapping is many-to-one and unaffected. Verify no Settings UI hardcodes the individual tool names.

## Open questions for the PRD/design

1. **`read` on a directory** — does it reuse `list_dir`'s JSON shape (`{path,entries,truncated}`) or a new text format? Do `offset`/`limit` paginate the listing or are they ignored for dirs? Should `call_read_file`'s image/doc short-circuits (native_registry.rs:401-416) run before the dir branch (they only fire on file extensions, so likely safe)?
2. **`bash_output` no-`job_id` list mode** — adopt it (relax the required `job_id`, types.rs:561) to preserve job enumeration after `list_background` is dropped, or accept that jobs are only discoverable by remembered `job_id`?
3. **Single alias mechanism** — confirm one legacy-name table (`find`→`glob`, `ls`→`read`, `skill_read_file`/`skill_run_script`/`skill_activate`→`skill`) consulted in `match_tool_call` (execute.rs:56) AND `tool_matches_recommended_name` (prepare.rs:713). Should it also rewrite persona `.md`/skill allow-lists at load, or resolve at match time only?
4. **Skill scripts after dropping `skill_run_script`** — is losing the `scripts/` restriction + interpreter allowlist + skill-root cwd acceptable, or should `bash`/`run_python` gain a skill-aware path? Confirm `secrets/` remains readable via absolute-path `read`.
5. **Surviving skill tool name** — keep `skill_activate` or rename to `skill`? If renamed, `is_native_skill_tool_name` (prepare.rs:169) and `openai_tool_name` assertions (types.rs:936) change; a rename plus alias adds surface.
6. **Persona rename policy** — auto-migrate built-in personas (agents/types.rs) to `glob`/drop `ls` now, and leave user/project `.md` personas to the alias? Or migrate stored files on load?
7. **Conversation-persisted tool state** — confirmed personas store allow-lists; not confirmed whether any conversation persists a tool-name list that would carry stale `find`/`ls`. Needs a check of `chat/storage.rs` conversation schema before finalizing the alias scope.

## Related specs
- `.trellis/spec/chat/request-shape-contracts.md` — wire-alias contract (the naming precedent).
- `native_registry.rs:12-13` references `.trellis/spec/backend/agent-runtime.md` and `.trellis/spec/backend/file-tools.md` as the binding contracts for the parallel-safe/read-only/consent sets (not opened this pass — should be read before editing those sets).
