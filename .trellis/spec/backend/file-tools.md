# File Tools

> Contracts for native file discovery, reading, editing, and writing in Kivio Chat.

## Design Stance (2026-06-11)

The model-facing tool surface is intentionally minimal, mirroring Claude Code and pi agent:

- **Read side**: `read_file`, `list_dir`, `search_files`, `glob_files`, `stat_path`.
- **Write side**: `write_file` (whole file), `edit_file` (exact replacement), plus path management `create_dir`, `delete_path`, `move_path`, `copy_path`.
- **No model-visible protection protocols.** The chunked draft-write session tools (`begin_file_write` / `append_file_write` / `finish_file_write` / `abort_file_write` / `write_file_chunk`) and the V4A `patch` tool were removed in 2.6.9. Their original motivation — provider streams dying mid-argument under the global 60s HTTP timeout — was fixed at the HTTP layer (commit `88f76ae`, see [HTTP Timeouts](./http-timeouts.md)). Do not reintroduce multi-step write protocols or hash/byte-count verification parameters into tool schemas.
- **Protection lives in the runtime**, invisible to the model: atomic writes, per-path locks, BOM/CRLF preservation, path-boundary validation, placeholder rejection.
- Tool descriptions are 1-2 plain sentences. Selection guidance lives in one line of the system prompt (small edits → `edit_file`; new files or whole-file overwrites → `write_file`), not in per-tool essays.

History: the removed draft-session design is documented in `.trellis/tasks/06-10-agent-file-editing-workflow/`; the removal rationale and mature-agent evidence in `.trellis/tasks/06-11-simplify-native-file-tools/` (PRD + `research/mature-agent-tool-surfaces.md`).

## Scenario: Agent File Operations

### 1. Scope / Trigger

- Trigger: changing native file tools (`src-tauri/src/native_tools/files.rs`), tool schemas (`src-tauri/src/mcp/types.rs`), file mutation dispatch (`src-tauri/src/mcp/registry.rs`), the built-in tools prompt (`src-tauri/src/chat/agent/prepare.rs`), or Chat UI rendering of file tool results (`src/chat/ToolCallBlock.tsx`).

### 2. Signatures

Discovery/read tools (gated by `native_tools.read_file`):

- `list_dir(path?) -> directory entries`
- `glob_files(pattern, path?) -> paths`
- `search_files(query, path?, max_results?) -> path/line/context matches`
- `stat_path(path) -> type/size/mtime metadata`
- `read_file(path, offset?, limit?) -> ReadFileResult`

Mutation tools (`write_file` + path tools gated by `native_tools.write_file`; `edit_file` by `native_tools.edit_file`):

- `write_file(path, content) -> FileMutationResult`
- `edit_file(path, old_string, new_string, replace_all?) -> FileMutationResult`
- `create_dir(path)` / `delete_path(path, recursive?)` / `move_path(from, to)` / `copy_path(from, to)`

### 3. Contracts

#### Path Resolution

- Project conversations: paths resolve relative to the bound project root and can never escape it (`resolve_project_path`); `..` is rejected; absolute paths must stay inside the root. Unbound legacy projects return a bind-first error.
- Non-project conversations: reads resolve anywhere readable (`resolve_read_path`); writes are restricted to `$HOME`, honor `workspace_roots` when configured, and `assert_writable_path` blocks `.ssh`, `.gnupg`, and Keychain directories.

#### Read

- `read_file` without `offset`/`limit` reads whole files up to `MAX_READ_FILE_BYTES` (2 MiB); larger files return an error that tells the model to pass `offset`/`limit`.
- With `offset`/`limit`, files of any size are read as a line window via streaming (`read_file_window_streaming`); the window itself is capped at 2 MiB and reports a warning plus `next_offset` when capped. Oversized reads with `offset` but no `limit` default to 2000 lines.
- Results report `total_lines`, `start_line`, `end_line`, `truncated`, `next_offset`, and `read_state.scope` (`full`/`partial`) so the model can continue reading.

#### Edit

- `edit_file` requires `old_string` to match current content exactly and uniquely; multiple matches error with the count unless `replace_all=true`. `old_string == new_string` returns an ok no-op result with a warning, `target_touched=false`.
- **Line endings are normalized for matching, not fuzzy-matched.** Matching/counting/replacing happen in an LF-normalized space (`normalize_line_endings(_, "\n")` on the file content, `old_string`, and `new_string`), so an LF `old_string` (what models emit) matches a CRLF file (the common Windows 0-match bug). The write goes through `atomic_write_text` with the original content, which re-applies CRLF and the UTF-8 BOM, so the on-disk line-ending style is preserved. The diff is computed in LF space so a CRLF↔LF difference is not reported as a whole-file change. A change that differs only in line-ending notation normalizes equal and returns the no-op result.
- No other fuzzy matching (no whitespace/indentation tolerance). Error messages must tell the model what to do next: re-read and copy an exact contiguous snippet with its indentation (`old_string not found`), or extend `old_string` with surrounding context / use `replace_all` (multiple matches). Do not blame CRLF in the not-found message — it is normalized away.

#### Write

- `write_file` creates or overwrites the whole file. No size-based routing: large content is one call.
- Placeholder rejection (`looks_like_placeholder_content`) applies **only when overwriting an existing code-like file** (`is_code_like_path` extension allowlist). New files and prose/document files may legitimately contain phrases like "省略" or "rest of file unchanged" and must not be blocked.
- All mutations (write/edit/delete/move/copy) run on the blocking thread pool (`run_blocking_file_mutation`) and take per-path locks (`acquire_file_mutation_locks`).
- Writes are atomic: temp file beside the target, then rename (`atomic_write_bytes`). Existing CRLF line endings and UTF-8 BOM are preserved (`atomic_write_text`). Non-UTF-8 existing targets may be overwritten with the diff omitted plus a warning.
- `delete_path` refuses to delete the workspace/project root; symlinks are removed without following the target.

#### Results

`FileMutationResult` carries `ok`, `operation`, `target_touched`, `resolved_path`, per-file `files[]` (path/operation/bytes/additions/removals/diff), aggregate diff stats, `warnings`, `diagnostics`. It is serialized into `structured_content` for the UI and summarized into one line for the model (`summary()`).

### 4. Approval & Scheduling

- `write_file`, `edit_file`, `create_dir`, `delete_path`, `move_path`, `copy_path` are `sensitive: true` → approval-gated under confirm policies and always serial.
- Read-side tools are parallel-eligible (`native:read_file` is in the narrow parallel whitelist; see [Agent Runtime](./agent-runtime.md)).

### 5. Frontend

- `ToolCallBlock.tsx` renders live tools (`write_file`, `edit_file`, read/path tools) and keeps **legacy-only** branches for `write_file_chunk`, `begin/append/finish/abort_file_write`, and `patch` so persisted conversations from ≤2.6.8 still render. Do not delete the legacy branches; do not add new active-path references to those names.

### 6. Tests Required

- `read_file`: whole-file read, partial window metadata, oversized-file rejection without window, oversized-file windowed read.
- `edit_file`: unique-match enforcement, `replace_all` stats, no-op warning, and line-ending normalization (LF `old_string` matches a CRLF file and the write keeps CRLF; a CRLF/LF-only change is a no-op; an LF file stays LF).
- `write_file`: structured diff metadata, non-UTF-8 overwrite warning, placeholder rejection on existing code files, placeholder acceptance for new files and prose files.
- Tool exposure: `list_native_builtin_tool_defs` write gate exposes exactly `write_file` + path tools; edit gate exposes exactly `edit_file`; none of the removed names appear.
- Scheduling: write tools stay outside the parallel whitelist even with `approval_policy = "auto"`.

### 7. Wrong vs Correct

| Wrong | Correct |
|---|---|
| Reintroducing a model-visible chunked/draft write protocol for large files | One `write_file` call; reliability problems belong to the HTTP/stream layer |
| Adding `expected_sha256`-style verification parameters to tool schemas | Runtime-internal verification only; never ask the model to precompute hashes |
| Teaching tool selection rules in a 2000-character prompt paragraph | One priority line in the prompt; the rest in 1-2 sentence tool descriptions |
| Rejecting any content containing "省略" | Reject only when overwriting an existing code-like file |
| Hard-failing `read_file` on >2 MiB files even with `offset`/`limit` | Stream the requested line window for files of any size |
| Removing legacy tool-name render branches from ToolCallBlock | Keep them for old conversations; backend no longer dispatches those names |
