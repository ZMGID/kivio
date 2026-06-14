# Fix file write tools and wire chunked large-file writes

## Goal

The uncommitted file mutation tool work (`write_file` / `edit_file` / `patch`) has a batch of
real defects found in review, and large-file writes have a fundamental UX/reliability flaw:
the model must generate the entire file content inside one `write_file` tool-call argument.
Nothing lands on disk until argument generation finishes, so a mid-stream failure wastes
everything generated so far. Fix all review findings and wire the already-implemented (but
dead) `write_file_chunk` tool so long files are written incrementally with durable progress.

## What I already know (from review, 2026-06-10)

* `write_file_chunk` (files.rs:160) is fully implemented (start/append/finish + locks + diff)
  but referenced nowhere: no tool definition, no dispatch, no filter-list entries. Dead code.
* Patch parser rejects completely empty hunk lines (files.rs:610) — models frequently emit
  empty context lines without the leading space; `git apply` tolerates this. High-frequency
  failure source.
* `patch_added_content` joins with `\n` and never emits a trailing newline (files.rs:688);
  the Add File grammar cannot express one at all.
* `write_file` overwrite reads old content via `fs::read_to_string` only for diff purposes and
  hard-fails on non-UTF-8 existing files (files.rs:134).
* `delete_path` / `move_path` / `copy_path` never acquire the per-path mutation locks that
  write/edit/patch use (spec agent-runtime.md:590 requires locks for mutation tools).
* `acquire_file_mutation_locks` blocks a tokio worker thread via `Condvar::wait`; all file
  mutation work runs synchronously on async threads (no `spawn_blocking`).
* `unified_diff` (files.rs:800) strips only common prefix/suffix → one giant hunk; scattered
  edits report wildly inflated +/- stats that the frontend renders.
* `move_path` resolves its source with `resolve_tool_write_path` (follows final symlink),
  inconsistent with `delete_path` and spec agent-runtime.md:442.
* In non-project conversations, relative patch paths silently resolve under `~` — surprising
  write locations with no warning.
* Multi-file patch apply-phase IO failure leaves earlier files written with an error message
  that does not say which files were already applied.

## Requirements

1. **Wire `write_file_chunk` end-to-end** as the incremental large-file write path:
   * `native_write_file_chunk_tool()` definition in `mcp/types.rs` (sensitive, source native),
     schema `{ path, mode: start|append|finish, content? }` (`content` required for
     start/append, ignored for finish).
   * Register in `list_native_builtin_tool_defs` under the same settings gate as `write_file`.
   * Dispatch in `call_native_tool` through `file_mutation_tool_result`.
   * Add to `disabled_builtin_tool_feedback` BUILTIN_NAMES and to the inline-code request
     filter (`write_file | patch | write_file_chunk`).
   * Stays serial (not in parallel whitelist), blocked in Plan mode (not read-only) — both
     already hold by default; add tests proving it.
   * Tool descriptions teach the model: for long content (roughly > 200 lines or > 8 KB),
     call `write_file_chunk` with mode=start for the first portion, then append in chunks of
     a few hundred lines, then finish; each chunk persists immediately so an interrupted
     stream only loses the unfinished chunk.
   * `write_file` description points long-content cases at `write_file_chunk`.
2. **Patch parser tolerance**: a completely empty line inside an Update File hunk is treated
   as an empty context line (equivalent to `" "`), not an error.
3. **Add File trailing newline**: `patch_added_content` emits a trailing `\n` when the file
   has any lines.
4. **write_file non-UTF-8 overwrite**: read of existing content degrades gracefully
   (`.ok()`); operation still reports `overwrite`, diff is empty, and a warning explains why.
5. **Lock coverage**: `delete_path`, `move_path` (source + destination), `copy_path`
   (destination) acquire `acquire_file_mutation_locks` before touching the filesystem.
6. **Async hygiene**: file mutation tool calls (write/edit/patch/chunk/delete/move/copy) run
   inside `tokio::task::spawn_blocking` in `call_native_tool` so lock waits and large IO do
   not stall runtime workers.
7. **Accurate diff stats**: replace the single-hunk prefix/suffix diff with an LCS-based
   line diff over the changed middle region (no new crate). Guard: if the middle region
   exceeds 500×500 lines, fall back to the current coarse single-hunk behavior. Multiple
   hunks separated by unchanged runs ≥ 7 lines; additions/removals must equal the +/- lines
   shown.
8. **move_path symlink semantics**: source resolves via `resolve_tool_write_entry_path`
   (do not follow the final symlink), matching `delete_path` and the spec.
9. **Non-project patch warning**: when `!workspace.has_project()`, `patch` succeeds but the
   result carries a warning that paths resolved under the home/workspace root (with the
   resolved base), since spec requires global fallback behavior rather than rejection.
10. **Partial-apply error clarity**: if the patch apply phase fails mid-way, the error lists
    which files were already written and which were not.

## Acceptance Criteria

* [ ] `write_file_chunk` appears in the model tool list when native write tools are enabled,
      executes start/append/finish against a project workspace, and returns structured
      `FileMutationResult` content.
* [ ] Plan mode blocks `write_file_chunk`; inline-code requests hide it alongside
      `write_file`/`patch` (unit tests).
* [ ] A patch containing an empty context line applies successfully (unit test).
* [ ] Add File produces content ending in `\n` (updated existing test).
* [ ] Overwriting a non-UTF-8 file via `write_file` succeeds with a warning (unit test).
* [ ] `delete_path`/`move_path`/`copy_path` hold path locks (test via lock-held observation
      or code inspection + existing lock tests still pass).
* [ ] Scattered two-location edit reports exact +/- counts and two hunks (unit test).
* [ ] `move_path` moves a project-internal symlink that targets outside the project without
      following it (unit test, unix).
* [ ] Non-project `patch` result includes the resolved-base warning (unit test).
* [ ] `cargo test --manifest-path src-tauri/Cargo.toml native_tools` green; full
      `cargo test` green; `npm run typecheck` green (frontend untouched or updated).

## Definition of Done

* Unit tests added/updated for every behavior change listed above.
* `cargo test` (full) + `npm run lint` + `npm run typecheck` pass.
* `.trellis/spec/backend/agent-runtime.md` updated: write_file_chunk contract added to the
  Agent File Mutation Tools scenario (signatures, serial/approval rules, chunk protocol,
  error matrix rows).

## Decision (ADR-lite)

**Context**: Large-file writes need durable incremental progress; a chunk protocol already
exists as dead code. Per-chunk approval prompts are annoying under confirm policies.
**Decision**: Reuse the existing start/append/finish protocol unchanged and keep
`sensitive: true` (every chunk approval-gated under always_confirm, same as write_file).
No approval special-casing in MVP — safety and consistency over convenience.
**Consequences**: Users on confirm policies see one prompt per chunk; acceptable for MVP,
revisit with a per-run "approve this path once" mechanism later. Diff algorithm is
hand-rolled LCS with a size guard instead of adding a crate — bounded memory, no new deps.

## Out of Scope

* Streaming tool-argument execution (executing write_file while arguments still stream).
* Per-path "approve once per run" approval batching.
* `bytes_written` semantics change for `edit_file` (it accurately reports bytes written to
  disk; documented, not changed).
* Frontend redesign of ToolCallBlock beyond what chunk results already render generically.

## Technical Notes

* Key files: `src-tauri/src/native_tools/files.rs`, `src-tauri/src/native_tools/mod.rs`,
  `src-tauri/src/mcp/types.rs`, `src-tauri/src/mcp/registry.rs`,
  `src-tauri/src/chat/agent/prepare.rs`, `src-tauri/src/chat/commands.rs`,
  `src/chat/ToolCallBlock.tsx` (verify chunk operations render; labels exist already).
* `mutation_operation_label` already maps `append`/`finish`; frontend
  `structuredFileMutation` parses generic operation/files shapes.
* No diff crate in Cargo.toml; hand-roll LCS with size guard.
* Spec contracts live in `.trellis/spec/backend/agent-runtime.md` (Agent File Mutation
  Tools + Per-Round Tool Scheduling scenarios).
