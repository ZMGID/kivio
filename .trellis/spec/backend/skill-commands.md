# Skill Slash Commands & Run-Level Cache

> Skill `/`-trigger with `$ARGUMENTS` substitution, run-level registry cache, dynamic `allowed_tools` filtering, and `skill_read_file` size cap. Landed in P2-B.

## Where it lives

- `src-tauri/src/skills/types.rs` — `SkillMeta.{triggers, argument_hint, arguments}` (all `#[serde(default)]`); `SkillRegistry::find_by_trigger`; `normalize_trigger`.
- `src-tauri/src/skills/parse.rs` — frontmatter parse of `triggers` / `argument-hint` / `arguments`.
- `src-tauri/src/skills/runtime.rs` — `substitute_arguments`; `SkillRunCache.registry` + `registry_for`; `read_skill_file` size cap.
- `src-tauri/src/chat/commands.rs` — `try_apply_skill_slash_trigger` in `chat_send_message` preprocessing.
- `src-tauri/src/chat/agent/{prepare.rs,loop_.rs}` — `retain_tools_for_allowed` + per-round recompute (`RunState.base_tools`).
- Frontend: `src/chat/slashCommands.ts` (`buildSlashCommands`), `src/chat/InputBar.tsx` (popover, action vs skill `kind`), `src/chat/Chat.tsx` (enabledSkills mapper), `src/api/tauri.ts` + `src/chat/types.ts` (`SkillMeta` fields).

## Executable contracts (do not regress)

1. **Single-pass `$`-substitution.** `substitute_arguments` does ONE left-to-right scan: each `$TOKEN` is resolved exactly once against the original body; substituted values are emitted verbatim and never re-scanned. This is mandatory — the old multi-pass `String::replace` chain had two bugs: (a) prefix collision (`$A` rewriting inside `$AB`), (b) feedback (a value containing `$X` re-substituted by a later pass). Greedy identifier read `[A-Za-z0-9_]+` after `$`. Resolution order: `$ARGUMENTS` → full trimmed arg string; `$NAME` (direct declared-name lookup, uppercased); `$ARG_NAME` (strip `ARG_` prefix then lookup); unknown `$x` → left literal. Missing positional → empty string. Both `$NAME` and `$ARG_NAME` conventions are supported. Tests: `substitute_arguments_no_prefix_collision_between_a_and_ab`, `..._does_not_re_substitute_value_tokens`.
2. **Backend owns the trigger; frontend is sugar.** `try_apply_skill_slash_trigger` runs in `chat_send_message` BEFORE the round-0 toolset is built, so a `/skill args` message works even without the frontend popover (paste / API / mobile). It only fires when `active_skill_id` is empty and the first token exactly matches a trigger of an **enabled** skill; it rewrites content to `[Skill: name]\n\n{substituted body}` and sets `active_skill_id`, then the existing pin chain (`resolve_forced_skill_id` → `apply_active_skill_tool_filter`) takes over. `find_by_trigger` is exact-match (leading `/`, lowercased) against explicit `triggers` or default `/{id}` / `/{name-slug}` — no prefix matching, so it won't shadow built-in popover commands (which are intercepted in the frontend before send).
3. **Run-level registry cache.** `SkillRunCache.registry_for(app, scan_paths)` builds the `SkillRegistry` at most once per run (was a full recursive FS scan on every skill tool call). `call_skill_tool` clones the `SkillRecord` out of the cached `&registry` BEFORE the `&mut cache` dispatch (borrow discipline). Cache lives on `SkillRunCache` (per-run lifetime), not `AppState` — no eviction/staleness logic; imports are picked up by the next run's fresh cache.
4. **Dynamic `allowed_tools` recompute from base.** When the model activates a skill mid-run, its `allowed_tools` narrows subsequent rounds. The filter MUST recompute each round from the full `RunState.base_tools` (then narrow by the union of activated skills' allowed_tools, then apply the plan-mode filter) — NOT cumulatively shrink a single list. Cumulative `retain` compounds with the plan-mode filter and permanently drops tools a later step needs. `retain_tools_for_allowed` always keeps skill-source tools, native skill meta-tools, and Kivio builtins. Test: `activated_tool_filter_recomputes_from_base_not_cumulatively`.
5. **`skill_read_file` size cap.** `read_skill_file` caps at `native_tools::MAX_READ_FILE_BYTES` — truncates at a UTF-8 boundary (`from_utf8_lossy` on the head slice) + appends a marker suggesting `skill_run_script` for the full file. The run-cache stores the already-truncated content (double-truncation safe).

## Tests / verification

- Rust: the substitution tests above, `find_by_trigger_*`, `parse_skill_markdown_reads_triggers_and_arguments`, `skill_run_cache_builds_registry_once`, `slash_trigger_rewrites_body_and_pins_skill` / `_ignores_non_slash_and_unknown` / `_skips_disabled_skill`, `read_skill_file_caps_oversize` / `_returns_full_when_small`, `retain_tools_for_allowed_*`.
- No frontend test runner configured — slash logic is covered by the Rust `try_apply_skill_slash_trigger` tests; pure helpers extracted to `src/chat/slashCommands.ts` for testability.
- Manual smoke: a user skill with `triggers: [/commit]`, `arguments: [message]`, body containing `$ARGUMENTS` → `/commit fix login` shows `/commit` in the popover, Enter sends, the turn pins the skill, body renders with `fix login`; `/help` still opens the built-in command list.
