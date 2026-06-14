# Agent File Editing Reliability PRD

## Goal

Redesign Kivio Chat's agent file editing and file writing workflow so large file creation, existing-file edits, and provider interruptions are handled reliably. The agent should not lose an entire large file because the provider stream disconnects once, and a failed file write should not crash or poison the main conversation.

The target behavior is closer to coding-focused agents such as Codex, OpenCode, Aider, and Hermes: existing code edits prefer patch/search-replace operations, long file creation uses an explicit resumable draft transaction, file mutations return inspectable diffs and metadata, and provider/tool failures are represented as scoped tool errors instead of one red failure that makes the user unsure what happened.

## Problem

Kivio currently exposes `write_file(path, content)` as a native Chat tool. The model must generate the complete file body inside one tool-call JSON argument before the backend can execute the tool.

This creates several failure modes:

- For long files, the risky part happens before the write tool starts. If the provider stream times out while generating `content`, the tool never executes and the entire generated file is lost.
- The app has a global HTTP client timeout of 60 seconds. This is reasonable for normal HTTP calls, but unreasonable for long SSE model streams where content is actively arriving.
- Existing-file edits may cause the model to regenerate a whole file even when only a small patch is needed.
- A post-tool provider failure can visually dominate the turn, hiding that file mutations may have succeeded.
- A naive chunk tool that writes directly to the target file would leave partial/corrupted target files if the model disconnects mid-write.
- The frontend can show "waiting" or "writing" without reflecting the true backend phase: generating tool arguments, executing the write, verifying, or failing.

## Current Evidence

- Current Kivio `write_file` requires `path` and full `content`.
- Current backend writes with `fs::write` only after complete arguments are decoded.
- Current `reqwest::Client` is built with a global `.timeout(Duration::from_secs(60))`.
- The latest observed error happened during `Chat tools planning` stream decoding, before a durable file mutation could finish.
- Kivio already has partial improvements for tool-call draft visibility, but not a durable/resumable write protocol.
- A half-built `write_file_chunk(start/append/finish)` exists in the working tree, but it writes/appends directly to the target file and should not be shipped as-is.

## External Reference Summary

- Codex / OpenAI `apply_patch`: code edits should be structured diffs that the runtime applies and reports back on, not giant generated files pasted into a tool.
- OpenCode: keep `edit`, `write`, and `apply_patch` separate under the same edit permission. `write` creates/overwrites; patch text embeds project-relative paths in marker lines.
- Aider: whole-file editing is simple but slow/costly; diff/search-replace/unified diff formats are better for normal code changes.
- Local Hermes: copy its practical safety ideas: `read_file(offset, limit)`, same-directory temp + rename atomic writes, line-ending/BOM preservation, path locks, stale-read warnings, diff output, and diagnostics.
- Claude-Code-style tool contracts: keep `Read`, `Glob`, `Grep`, `Edit`, and `Write` roles distinct; require current context before mutating existing files; report tool-level errors so the model can recover instead of guessing.

See: `research/agent-file-editing-patterns.md`.

## Reference Hierarchy

Kivio should not copy one tool wholesale. Use each reference for the part it does best:

1. Codex / OpenAI `apply_patch`
   - Primary reference for code edits.
   - Copy structured add/update/delete diffs, runtime parsing, path validation, and explicit success/failure tool outputs.

2. OpenCode
   - Primary reference for tool boundaries.
   - Copy the separation between `read`, `glob`, `grep/search`, `edit`, `write`, and `apply_patch`; keep file mutations under one permission category.

3. Aider
   - Primary reference for why whole-file edits should not be the normal coding path.
   - Copy the principle that diff/search-replace formats are cheaper and safer than returning full files for small changes.

4. Hermes Agent
   - Primary reference for local filesystem safety details.
   - Copy offset reads, same-directory temp writes, BOM/line-ending preservation, stale-read warnings, per-path locks, diagnostics, and recovery hints.

5. Kivio-specific extension
   - Add durable draft write sessions for long generated files. This is the missing layer in the references: it addresses Kivio's provider-stream interruption and current full-argument write problem.

## Adopted Design Stance

This PRD intentionally combines the references instead of copying any one agent:

- Copy Codex/OpenAI for the normal code-edit model: the model proposes structured add/update/delete edits, and the host runtime validates, applies, and returns an explicit tool result.
- Copy OpenCode for tool boundaries: `read`, `glob`, `grep/search`, `edit`, `write`, and `apply_patch` stay separate even when they share one edit permission group.
- Copy Aider for edit-format discipline: whole-file edits are allowed, but they are not the normal path because they are expensive and encourage omitted or placeholder code.
- Copy Claude text editor style for exact replacements: a precise replacement must be based on current viewed context and must match exactly one location unless the tool explicitly requests replace-all.
- Copy Hermes for local filesystem correctness: offset reads, stale-read warnings, per-path locks, same-directory temp writes, line-ending/BOM preservation, post-write verification, diffs, and diagnostics.
- Add Kivio's own draft-session layer for long generated files because the observed failure happens while the provider is streaming a huge tool argument, before a normal `write_file` call can execute.

The practical rule is simple:

```text
discover/read -> edit_file or patch for existing code
small create/overwrite -> write_file
large generated content -> begin_file_write -> append_file_write* -> finish_file_write
```

If a future implementation makes the model generate a large source file inside one `write_file.content` argument for normal coding work, that implementation has regressed from this PRD.

## Design Decision Snapshot

The final behavior should be:

```text
find/read context -> produce the smallest safe edit -> backend validates -> backend applies -> UI shows the exact phase/result
```

Kivio should stop treating "write a file" as one generic capability. Coding agents are safer when file tools have narrow roles:

- Read and discovery tools locate context.
- `edit_file` changes one exact known block.
- `patch` applies normal source-code changes.
- `write_file` creates or explicitly replaces a small whole file.
- Draft-session tools handle long generated content.

This follows OpenCode's tool separation, Codex/OpenAI's patch-first runtime, Aider's warning that whole-file edits are costly, Anthropic's exact replacement contract, and Hermes' local filesystem safety layer.

The draft-session layer is Kivio-specific. Patch/edit tools reduce the need for huge writes, but they do not solve the observed Kivio failure where a provider disconnects while streaming one very large JSON tool argument. Draft sessions move long content into backend-owned durable state before final commit.

## Normative Reference Matrix

| Area | Primary Reference | Kivio Decision |
|---|---|---|
| Normal code edits | Codex/OpenAI `apply_patch`, OpenCode `apply_patch`, Aider `udiff` | Use `patch` as the default multi-file/larger edit path. The model emits structured operations; Kivio validates and applies them. |
| One-region replacement | OpenCode `edit`, Claude text editor `str_replace`, Hermes `patch_replace` | Use `edit_file` with exact old/new strings, uniqueness checks, and recovery hints. |
| File discovery | OpenCode `glob`/`grep`/`read`, Claude text editor `view`, Hermes `read_file(offset, limit)` | Keep `list_dir`, `glob_files`, `search_files`, `stat_path`, and `read_file` separate. Use partial reads for large files. |
| Small whole-file creation | OpenCode `write`, Claude text editor `create`, Hermes `write_file` | Keep `write_file`, but make it atomic and steer it to small new files or explicit full overwrites only. |
| Local filesystem safety | Hermes Agent | Copy same-directory temp writes, line-ending/BOM preservation, path locks, stale-read warnings, resolved-path reporting, and diagnostics. |
| Large generated files | Kivio-specific | Use draft write sessions because provider stream interruptions can happen while tool arguments are still being generated. |

This matrix is the implementation priority. If two references disagree, prefer the safer coding-agent behavior over a simpler text-generation behavior.

## Implementation Priority Rules

Use these as tie-breakers when implementation choices are ambiguous:

1. Existing source code changes prefer `patch`; `edit_file` is for one exact local replacement.
2. `write_file` is not a code-edit default. It is for small new files, small explicit deliverables, or user-requested full replacement.
3. Large generated files must not depend on one huge JSON argument. They must move content into backend-owned draft storage in chunks.
4. The real target file must remain untouched until a verified final commit step.
5. A failed tool must return a recoverable tool result with `target_touched=false` when no target mutation happened.
6. A successful tool mutation must remain visible even if the final assistant answer fails afterward.
7. Frontend state must reflect backend state; it must not infer that a file was written just because model arguments are being generated.
8. Compatibility tools may exist for old callers, but model-facing prompts and schemas must steer away from unsafe protocols.

## Normative Operating Rules

These are product decisions, not suggestions:

- "Current context" means the agent has recently used `read_file`, `search_files`, or equivalent tool output for the exact file/region being changed. Prose from the model's memory is not current context.
- `read_file(offset, limit)` is line-based. `append_file_write(offset)` is byte-based and must use UTF-8 byte length of the draft.
- A partial read is enough for a targeted `edit_file`/`patch` in that region, but not enough for a full overwrite.
- `write_file` on an existing file must be treated as a full replacement and should warn or block if the file was never read, was only partially read, or changed after the last read.
- `edit_file` is exact-string replacement, not fuzzy editing. If the string does not match, the model should re-read/search rather than retrying variants blindly.
- `patch` is the normal code-edit path. A multi-file patch must validate every path and hunk before the first target file is touched.
- `write_file` is for small explicit files. Model-facing guidance should route files expected to exceed about 300 lines or 20 KiB to draft sessions. Backend enforcement may use a configurable warning/reject threshold; the default should prefer draft sessions once content approaches 64 KiB.
- Draft-session chunks are stored only in app-managed draft storage. They are never appended to the target path, never stored in browser/http cache, and never treated as packaged runtime assets.
- `finish_file_write` is the commit point. Before it succeeds, the correct UI phrase is "target untouched", not "file written".
- A failed provider stream after a tool-call draft starts is a scoped tool/planning failure. It must not erase successful earlier file mutations from the turn.
- `write_file_chunk` is not a product feature. If retained, it is a compatibility shim over draft sessions and should stay absent from normal prompts.

### Size And Routing Defaults

The model needs a concrete rule so it does not guess:

| Situation | Tool route |
|---|---|
| Existing source file, one local block | `read_file` -> `edit_file` |
| Existing source file, several blocks or files | `search_files` / `read_file` -> `patch` |
| New small config/doc/source file | `write_file` |
| Existing small file explicitly requested as a full rewrite | full `read_file` -> `write_file` |
| New or replacement file likely over 300 lines / 20 KiB | `begin_file_write` -> `append_file_write` -> `finish_file_write` |
| Any long generated artifact where provider interruption would waste work | draft-session tools |

The exact byte thresholds can be tuned, but the direction cannot: large generated content must stop depending on a single `write_file.content` argument.

## Product Principles

- Targeted edits first; whole-file writes only when appropriate.
- Long file writes must be durable, resumable, and atomic.
- Never write partial generated content directly into the target file.
- A provider disconnect should not erase visible progress or invalidate the whole chat turn.
- File mutation state must come from backend runtime events, not frontend guesses.
- Tool success, tool failure, provider stream failure, and final answer failure must be visually separate.
- Prefer small, local Rust/React changes over heavy new dependencies.

## Users And Use Cases

### Primary User

A user asks Kivio Chat to modify a bound local project, create a website/demo, generate source files, update configuration, or produce durable local deliverables.

### Key Use Cases

- Modify a few lines in an existing file.
- Apply coordinated changes across multiple files.
- Create a small new file.
- Generate a large new file without losing all progress on one disconnect.
- Resume or retry a failed large-file write.
- Understand whether failure happened during model generation, tool execution, verification, or final assistant response.

## Proposed Direction

Use four clear file mutation paths:

1. `edit_file`
   - For small existing-file edits.
   - Exact old/new replacement with uniqueness enforcement.

2. `patch`
   - For multi-file or larger code edits.
   - V4A/Codex-style add/update/delete patch format.

3. `file_write_session`
   - For long new files, long explicit full-file replacement, or generated deliverables.
   - Draft-based, chunked, resumable, and atomically committed.

4. `write_file`
   - For small new files only, or explicit whole-file overwrite when the model already has the full intended content.
   - Atomic write implementation, but not the default coding edit primitive.

Keep `write_file` as a compatibility/simple-path tool for small new files and explicit full overwrites, but make prompt guidance and tool descriptions push coding work toward `edit_file`, `patch`, or `file_write_session`.

## Tool Selection Rules

The model and tool descriptions must steer to this order:

1. Need to find files: use `glob_files` or `search_files`.
2. Need code context: use `read_file`, preferably with a small range when the target is known.
3. Need to change an existing file:
   - one precise region: `edit_file`;
   - multiple regions/files or add/delete/update: `patch`;
   - avoid `write_file` unless the user asked for a full rewrite or the file is tiny and simple.
4. Need to create a small file: `write_file`.
5. Need to create a large file or replace a large file: `begin_file_write` -> `append_file_write` -> `finish_file_write`.
6. Need to recover after a failed edit:
   - no match / multiple match: re-read or search, then retry with better context;
   - append offset mismatch: inspect session progress, then resume at the reported offset;
   - provider stream interrupted before finish: report that the target is untouched and offer retry/resume.

Hard rule: no tool description should recommend direct target-file chunk appends.

### Tool Selection Decision Tree

Use this decision tree in prompts, tool descriptions, tests, and review:

```text
Need to know where something is?
  -> glob_files / search_files

Need current content?
  -> read_file
  -> use offset/limit for large or focused context

Need to modify existing source code?
  -> one unique exact block: edit_file
  -> multiple blocks/files, add/update/delete, larger change: patch
  -> never default to write_file

Need to create a small explicit file?
  -> write_file

Need to generate or replace a long full file?
  -> begin_file_write
  -> append_file_write until complete
  -> finish_file_write after byte/hash verification

Need to stop a large write?
  -> abort_file_write
```

Ambiguous cases should choose the tool with the smallest mutation surface. If the agent only read one section of an existing file, it must not overwrite the whole file from that partial view.

### Non-Negotiable Invariants

- `write_file_chunk` must never direct-append to the real target in shipped behavior.
- The real target file can be touched only by `edit_file`, `patch`, `write_file`, or a successful `finish_file_write`.
- `begin_file_write`, `append_file_write`, `abort_file_write`, and failed `finish_file_write` always mean `target_touched=false`.
- Large generated content must not depend on one huge `write_file.content` JSON argument.
- Existing-code edits must prefer `patch` or `edit_file`; whole-file replacement is an explicit exception, not the normal path.
- Every mutation result must be inspectable after provider failure: changed paths, touched/not touched, diff stats, warnings, diagnostics, and session state when relevant.
- The frontend must distinguish model argument generation from backend mutation execution.

## Tool Ownership Rules

Each tool has a narrow job. This avoids the current ambiguity where every file operation collapses into `write_file`.

| Tool | Owns | Must not own |
|---|---|---|
| `list_dir` | Directory shape and entry types. | File body reads or search. |
| `glob_files` | Filename/path discovery by pattern. | Reading matched file contents. |
| `search_files` | Locating text/symbols and returning line/context hits. | Dumping full files. |
| `stat_path` | Size/type/mtime checks before deciding read/write strategy. | Content mutation. |
| `read_file` | Text content, whole file when small, ranged slices when large. | Shell-style `cat` replacement for binary/device/secret paths. |
| `edit_file` | One exact replacement in one existing file. | Large multi-file edits or whole-file rewrites. |
| `patch` | Structured add/update/delete code edits, especially multi-file work. | Direct shell patching without path/hunk validation. |
| `write_file` | Small new files and explicit small whole-file overwrites. | Normal source-code modifications or long generated files. |
| `begin_file_write` | Creating a durable draft session and capturing pre-state. | Writing target content. |
| `append_file_write` | Appending bytes to backend draft storage with offset validation. | Touching the target file. |
| `finish_file_write` | Verifying and atomically committing the completed draft. | Accepting incomplete or unverifiable draft content. |
| `abort_file_write` | Marking a draft stopped and preserving target safety. | Deleting or modifying the target. |

The agent prompt, tool schemas, backend dispatch, tests, and UI labels should all use this ownership table as the source of truth.

## Canonical Workflows

These are the workflows the prompt, tool descriptions, backend tests, and UI labels should all agree on.

### Read a file

```text
stat_path(path) when size/type is unknown
read_file(path) for small text files
read_file(path, offset, limit) for large files or focused context
```

Rules:

- Do not use terminal `cat`, `sed`, or `head` as the normal read path when native tools are available.
- `read_file` output must include enough range metadata for the next read: total lines, returned range, truncated flag, byte size, and next offset hint.
- Repeated identical reads of unchanged content should return a lightweight "unchanged/already read" result or warning so the agent does not burn context in a loop.

### Grab a relevant file section

```text
search_files(query, path)
read_file(path, offset=<match line - context>, limit=<small window>)
```

Rules:

- `search_files` is for locating lines/symbols; it should not dump whole files.
- The model should widen the read window only when the first slice is insufficient.
- Partial reads mark the file state as partial; full-file overwrites after partial reads must warn or require a fresh whole-file read.
- The follow-up edit should be `edit_file` or `patch`; the agent must not treat one partial slice as enough evidence for a full-file rewrite.

### Modify existing code

```text
search_files / read_file
edit_file for one unique replacement
patch for multi-file, add/update/delete, or larger code edits
```

Rules:

- `patch` is the default coding edit path.
- `edit_file` rejects no-match, multiple-match, and no-op edits.
- `patch` validates every path and hunk before touching any file.
- Both tools acquire per-path locks, return diffs, and refresh read-state after success.
- On failure, the next model step should read/search again instead of retrying the same stale text.
- The model should not rewrite the full file unless the user explicitly asked for a full rewrite or the file is tiny and self-contained.
- Patch is preferred even for a single file when the edit spans multiple regions.
- Multi-file patch is all-before-write for MVP: path and hunk validation must finish before any target is modified.

### Write a small new file

```text
write_file(path, content)
```

Rules:

- Use only when the full intended content is already small and explicit.
- The backend writes to a temp file beside the target, preserves existing mode/line endings/BOM where practical, then renames over the target.
- Existing target overwrites return stale-read warnings when the file was not read or changed since the last read.
- If the content contains placeholder text such as "rest of original file here", the write must fail before touching the target.
- `write_file` may still create durable user deliverables, but it must not be used to smuggle a long coding edit into one fragile argument.

### Write a large generated file

```text
begin_file_write(path, mode, expected_bytes?, expected_sha256?)
append_file_write(session_id, offset, content, chunk_sha256?) repeated
finish_file_write(session_id, expected_bytes?, expected_sha256?)
```

Rules:

- Chunks append to draft storage only.
- The target file remains unchanged until `finish_file_write`.
- Each append returns the next byte offset and chunk count.
- `finish_file_write` verifies byte/hash expectations, writes atomically, returns diff metadata, and deletes the successful draft.
- Provider interruption before finish is a draft/tool failure with `target_touched=false`, not a corrupted target file.
- Draft sessions are a backend feature, not a frontend progress illusion. The backend owns the session id, metadata, offsets, and draft file.

### Abort or recover a large write

```text
abort_file_write(session_id, reason)
```

Rules:

- Abort never touches the target.
- Failed/aborted drafts remain in app support data for the retention window.
- Offset mismatch reports the current offset so the model can continue from the durable draft position.

## Tool-By-Tool Workflow

### Read / Discover

1. `list_dir`
   - Use when the agent needs to inspect a directory shape.
   - Returns names and basic entry type only; it must not read file bodies.

2. `glob_files`
   - Use when the agent knows a filename pattern, such as `src/**/*.tsx` or `src-tauri/src/**/*.rs`.
   - Returns candidate paths only.
   - Patterns must stay relative to the requested search path.

3. `search_files`
   - Use when the agent knows text/symbols but not exact locations.
   - Returns path, line number, and short matching context.
   - This is the first step for "grab the relevant part of a file".

4. `stat_path`
   - Use before reading or writing when size/type matters.
   - Helps decide whether `read_file` can read the whole file or must use `offset`/`limit`.

5. `read_file`
   - Small text file: read whole file when the size limit allows.
   - Large file or targeted edit: read by line range with `offset` and `limit`.
   - Normal partial context flow: `search_files` -> pick line number -> `read_file(path, offset, limit)`.
   - It should strip/normalize only for display when needed, but mutation tools must preserve original line endings/BOM where practical.
   - Return enough metadata for follow-up reads: total lines, returned range, truncated flag, byte size, and next offset hint.
   - Record read state so mutation tools can warn when an existing file changed after the last read.
   - Repeated identical reads of unchanged regions should return a compact already-read warning after the first repeat, so the agent stops looping and either widens the range or edits.

### Grab Partial Context

Use partial context for targeted work, not whole-file reconstruction:

```text
search_files("symbol or text", path)
read_file(path, offset=<near match>, limit=<focused window>)
edit_file(...) or patch(...)
```

Rules:

- `search_files` returns locations and short context; it does not become a hidden full-file read.
- `read_file(offset, limit)` marks that path as partially read.
- A later full overwrite after only partial read state must warn or block depending on approval policy.
- The model should widen the read range only when necessary; repeated identical reads should be discouraged.
- The mutation result should update read state so a successful write becomes the agent's current known file state.

### Modify Existing Files

6. `edit_file`
   - Use for one precise replacement in one existing file.
   - Requires `old_string` and `new_string`.
   - Reject if `old_string` is missing, duplicated, or identical to `new_string`, unless `replace_all` is explicitly true.
   - On failure, the agent must re-read relevant context instead of guessing.
   - Return recovery hints that tell the model whether to use a larger read range, `search_files`, or `patch`.

7. `patch`
   - Default coding edit tool.
   - Use for multi-file changes, add/update/delete operations, or larger existing-file edits.
   - Runtime validates paths and hunks before touching files.
   - Runtime returns affected files, diff, additions/removals, warnings, and diagnostics.
   - This is the main reference path from Codex/OpenCode/Aider/Hermes.
   - Multi-file patch MVP should validate all hunks before mutating any file. If future behavior allows partial apply, it must report exact per-file success/failure.
   - For add/update plans, writes still go through the same atomic write path as `write_file`.
   - Patch paths are project-relative and must reject absolute paths, `~`, backslashes, and `..`.
   - Patch results must be replay-safe: even if final answer synthesis fails, the tool record still says exactly what changed.

### Write New Or Whole Files

8. `write_file`
   - Use only for small new files or explicit full-file overwrite.
   - Not the default for code edits.
   - Must use atomic write: create parent dirs, write temp beside target, preserve mode/line endings/BOM where applicable, rename over target.
   - Must return diff metadata.
   - Must warn when overwriting an existing file that was not read in this conversation or whose mtime/hash changed after last read.
   - Must reject internal read/status text such as "file unchanged" stubs being written as file content.
   - Must reject lazy placeholders such as "rest unchanged", "original code here", or "same as before".
   - Should be treated as a full replacement operation in UI and approval copy.

9. `write_file_chunk`
   - Do not ship direct target-file `start/append/finish`.
   - Replace with draft-session tools below.
   - If retained for compatibility, implement it as a wrapper over draft sessions and mark it deprecated in schema output.

10. `begin_file_write`
    - Creates a draft session for large new file/full overwrite.
    - Validates target path and captures pre-state.
    - Does not mutate target.

11. `append_file_write`
    - Appends chunk content to the draft only.
    - Requires exact byte offset match.
    - Optional chunk hash validation.
    - Does not mutate target.
    - Returns current draft offset after every append so the model can resume safely.

12. `finish_file_write`
    - Verifies expected bytes/hash when provided.
    - Generates diff metadata against target pre-state.
    - Atomically replaces target only after the draft is complete.
    - On verification mismatch, keep the draft resumable and return `target_touched=false`.

13. `abort_file_write`
    - Marks session aborted.
    - Target remains untouched.
    - Draft may be retained for a short recovery window.

## MVP vs Hardening

MVP must include:

- prompt/schema guidance that steers existing code changes to `patch`/`edit_file`;
- atomic `write_file` and `patch` writes through one shared helper;
- draft session tools with target untouched until verified finish;
- `target_touched` semantics in structured mutation results;
- frontend display for draft bytes, chunk count, target path, and target untouched/modified state;
- streaming timeout separation so active SSE streams are not killed by a 60-second total timeout;
- tests for offset mismatch, finish verification mismatch, abort, placeholder rejection, and `write_file_chunk` compatibility safety.

Hardening can follow after MVP:

- stronger cross-run stale-read registry;
- cleanup UI for abandoned drafts;
- optional diff approval before commit;
- diagnostics/LSP integration;
- true Windows `ReplaceFileW` final replacement semantics where practical;
- richer read deduplication and partial-read policy controls.

Do not defer the draft-session target-safety invariant to hardening. That invariant is MVP.

## Implementation Contract By Tool

This section turns the reference research into product-level delivery rules. A feature is not done if the UI looks correct but the backend still uses an unsafe file mutation path.

### Discovery tools

`list_dir`, `glob_files`, `search_files`, and `stat_path` are context tools. They should help the model choose the smallest safe next action.

Required outcome:

- The model can find files and line numbers without dumping whole file bodies.
- Search/list results are bounded and skip heavy ignored directories by default.
- Results include enough path/type/line metadata for a follow-up `read_file`.
- These tools never imply mutation and never set file-write UI states.

Reference basis:

- OpenCode separates `glob`, `grep`, and `read`.
- Hermes uses search/read wrappers so the model does not burn context with repeated shell `cat/head/tail` patterns.

### `read_file`

`read_file` is the source of current file context. It must be good enough for targeted edits and honest enough to block unsafe whole-file replacement from partial context.

Required outcome:

- Reads are line-based: `offset=1, limit=N` returns lines 1 through N.
- The response includes `path`, `resolved_path`, `content`, `start_line`, `end_line`, `total_lines`, `truncated`, `file_size`, and `next_offset` when truncated.
- The backend records whether the read was full or partial.
- Repeated identical reads of unchanged content produce a compact warning/stub instead of resending the same region indefinitely.
- Device, binary, image, and obvious secret paths are rejected or routed away from text read.

Reference basis:

- OpenCode `read` supports line ranges.
- Anthropic text editor `view` supports whole-file and range inspection.
- Hermes tracks `(path, offset, limit)` reads and stale/partial state.

### `edit_file`

`edit_file` is exact replacement, not fuzzy magic and not a mini whole-file writer.

Required outcome:

- It requires current context from `read_file`/`search_files` or emits a stale/no-read warning.
- It rejects zero matches.
- It rejects multiple matches unless `replace_all=true`.
- It rejects no-op old/new pairs.
- On failure, it tells the model to re-read/search, widen/narrow the exact string, set `replace_all=true` only when appropriate, or switch to `patch`.
- On success, it returns a `FileMutationResult` with diff stats and `target_touched=true`.

Reference basis:

- OpenCode describes `edit` as the primary precise code modification tool.
- Anthropic text editor `str_replace` is exact replacement.
- Hermes returns recovery hints for failed replacements.

### `patch`

`patch` is the normal coding edit path. It should be the default whenever a source-code change spans multiple regions, files, add/update/delete operations, or meaningful refactors.

Required outcome:

- Patch paths are project-relative and reject absolute paths, `~`, backslashes, and `..`.
- Add/update/delete operations are parsed and validated before the first target file is touched.
- Multi-file patches acquire locks in sorted resolved-path order.
- Add/update writes use the same atomic helper as `write_file`.
- Failures before mutation return `ok=false`, `target_touched=false`, the failing path/hunk, and a recovery hint.
- Success persists affected files, additions/removals, diff metadata, warnings, and diagnostics so a later final-answer failure cannot hide the landed change.

Reference basis:

- OpenAI/Codex `apply_patch` uses structured create/update/delete diffs that the runtime applies and reports back on.
- OpenCode exposes `apply_patch` separately from `write`.
- Aider documents diff/udiff formats as the safer normal edit shape compared with whole-file output.
- Hermes supports V4A patch parsing, path locks, diffs, and diagnostics.

### `write_file`

`write_file` is a small whole-file create/replace tool. It is not the default source-code editing primitive.

Required outcome:

- Use it for small new files, small explicit deliverables, or explicit small full overwrites.
- Existing target writes are treated as full replacement and warn on no-read, stale-read, or partial-read state.
- It writes atomically through same-directory temp replacement or a stronger platform equivalent.
- It preserves existing mode, CRLF line endings, and UTF-8 BOM where practical.
- It rejects placeholder/status-stub content before opening the target for mutation.
- It recommends draft sessions when content is likely over about 300 lines / 20 KiB, and should reject or warn strongly when content approaches the larger backend threshold.

Reference basis:

- OpenCode `write` creates or overwrites and is separate from `edit`.
- Aider's `whole` format is simple but slow/costly because it returns entire files.
- Hermes `write_file` uses temp + rename and preservation behavior.

### Draft write sessions

Draft sessions are Kivio's answer to the observed provider-stream failure: a long generated file should not live only inside one fragile JSON argument until the model finishes.

Required outcome:

- `begin_file_write` creates backend-owned draft metadata and captures target pre-state.
- `append_file_write` writes only to app-data draft storage, requires exact UTF-8 byte offset, and returns current offset and chunk count.
- `finish_file_write` verifies expected bytes/hash when present, then atomically commits to the real target.
- `abort_file_write` stops the draft and never touches the target.
- Every pre-finish failure returns `target_touched=false`.
- Failed drafts stay recoverable until retention cleanup; successful drafts are removed after the committed result is persisted.
- Draft cleanup can delete only known draft-session directories under the app-data draft root.

Reference basis:

- This is Kivio-specific. Codex/OpenCode/Aider reduce normal edit size with patch/edit, but they do not solve Kivio's long `write_file.content` stream interruption by themselves.
- Hermes supplies the local filesystem safety model: app-owned state, path locks, pre-state, temp writes, verification, and diagnostics.

### `write_file_chunk`

`write_file_chunk` is not a product feature.

Required outcome:

- It must not appear in normal prompts or recommended tool schemas.
- If old callers still invoke it, it must behave as a compatibility wrapper over draft sessions.
- It must never append directly to the target path.

Reference basis:

- The reviewed coding tools do not use direct target chunk append as their normal safe edit model.
- Direct append makes Kivio's current failure mode worse because the target can be corrupted halfway through provider generation.

## Tool Contract

### `edit_file`

Input:

```json
{
  "path": "src/App.tsx",
  "old_string": "old text",
  "new_string": "new text",
  "replace_all": false
}
```

Requirements:

- Reject missing file.
- Reject no-op edits where old and new strings are identical.
- Require exactly one match unless `replace_all` is true.
- Preserve project path boundaries.
- Return diff metadata.

### `patch`

Input:

```json
{
  "patch": "*** Begin Patch\n*** Update File: src/App.tsx\n@@\n-old\n+new\n*** End Patch"
}
```

Supported operations:

- `*** Add File: path`
- `*** Update File: path`
- `*** Delete File: path`

MVP excludes move/rename unless already cheap to support.

Requirements:

- Patch file paths must be project-relative.
- Reject absolute paths, `~`, backslash paths, and `..` traversal.
- Validate all hunks before writing any file.
- Acquire sorted per-path locks.
- Return all affected files, additions/removals, diff, warnings, and diagnostics.

### `file_write_session`

Use a transaction-style protocol instead of direct append to the target.

#### `begin_file_write`

Input:

```json
{
  "path": "src/generated/large.ts",
  "mode": "create_or_overwrite",
  "expected_bytes": 120000,
  "expected_sha256": "optional",
  "description": "optional user-facing purpose"
}
```

Behavior:

- Resolve and validate target path.
- Create a session id.
- Create a draft file under app-managed data, not in arbitrary cache and not directly at the target.
- Capture target pre-state metadata if it exists.
- Emit a pending tool record with session id and target path.

#### `append_file_write`

Input:

```json
{
  "session_id": "fw_...",
  "offset": 32768,
  "content": "next chunk",
  "chunk_sha256": "optional"
}
```

Behavior:

- Append only if `offset` matches current draft length.
- Optionally verify chunk hash.
- Emit real bytes/chunks progress.
- Do not mutate target file.
- Return `session_id`, `target_path`, `current_offset`, `chunk_count`, `target_touched=false`, and a retry hint on mismatch.

#### `finish_file_write`

Input:

```json
{
  "session_id": "fw_...",
  "expected_bytes": 120000,
  "expected_sha256": "optional"
}
```

Behavior:

- Verify final byte count/hash when provided.
- Generate diff metadata against target pre-state.
- Atomically replace target with draft using temp file + rename.
- Clean up session draft after success.
- Return `FileMutationResult`.
- If verification fails, keep the target untouched and keep the draft resumable until cleanup.

#### `abort_file_write`

Input:

```json
{
  "session_id": "fw_...",
  "reason": "provider stream interrupted"
}
```

Behavior:

- Mark session aborted.
- Keep draft for a short retention window if partial recovery is possible.
- Do not mutate target file.

## File Mutation Result Schema

All mutation tools should return a common envelope so the frontend and model do not have to guess:

```json
{
  "ok": true,
  "operation": "patch",
  "target_touched": true,
  "files": [
    {
      "path": "src/App.tsx",
      "action": "update",
      "bytes_written": 1234,
      "additions": 3,
      "removals": 1,
      "diff": "..."
    }
  ],
  "warnings": [],
  "diagnostics": [],
  "session": null
}
```

Required semantics:

- `target_touched=false` means no target file was modified.
- `target_touched=true` is only allowed after `edit_file`, `patch`, `write_file`, or successful `finish_file_write`.
- Failed draft appends must include `target_touched=false`.
- Warnings are non-fatal and should be visible, especially stale-read and wrong-cwd warnings.
- Diagnostics are post-mutation validation signals, not reasons to pretend the mutation failed.
- For draft-session operations before finish, `files` may be empty, but `session` must be populated so the UI can render target path, byte offset, chunk count, status, and target-touched state.
- For failed validation before mutation, the result must prefer structured `ok=false` output over throwing a generic transport error.

## Frontend State Contract

The UI must render the following states from tool events and result payloads:

| State | Source | User-facing meaning |
|---|---|---|
| `generating_arguments` | Provider stream is producing tool arguments. | No backend file write has happened yet. |
| `drafting_file` | `begin_file_write` result. | Draft exists; target untouched. |
| `writing_draft` | `append_file_write` in progress or completed. | Bytes are being stored in app draft storage; target untouched. |
| `applying_file_change` | `finish_file_write`, `patch`, `edit_file`, or `write_file` executing. | Backend is applying a real mutation. |
| `completed` | `ok=true` and final operation state. | Mutation or draft step succeeded. |
| `failed_uncommitted` | `ok=false` and `target_touched=false`. | Target file was not changed. |
| `failed_after_commit` | Tool succeeded, final synthesis or later provider step failed. | File mutation succeeded; final answer failed separately. |
| `aborted` | `abort_file_write`. | Draft stopped; target untouched. |

This distinction is mandatory. The UI must not show "已写入文件" while the model is only generating a large JSON argument.

## State Machine

Large write sessions use this lifecycle:

```text
idle
  -> begin_file_write: drafting
drafting
  -> append_file_write: drafting
  -> finish_file_write: committing
  -> abort_file_write: aborted
committing
  -> committed
  -> failed_uncommitted
```

Rules:

- `drafting`: target untouched.
- `committing`: target can change only inside the atomic commit section.
- `committed`: target changed and diff metadata must be available.
- `failed_uncommitted`: target untouched; draft can be retained for retry.
- `aborted`: target untouched; draft retention follows policy.

## Storage And Retention

Drafts should live in app-managed durable support data, not generic browser/http cache:

- macOS: Tauri app data/support directory.
- Windows: Tauri app data directory.
- Suggested subdir: `agent-file-drafts/<conversation_id>/<session_id>/`.

Retention:

- Successful sessions: delete draft immediately after commit.
- Failed/aborted sessions: retain for a short window, default 24 hours or until app cleanup.
- Include metadata JSON with target path, workspace id, offsets, hashes, created_at, updated_at, and status.
- Draft metadata must be app support data, not user-facing project files and not browser cache.
- The draft store is part of the agent file-write feature; cleanup policy must not delete active sessions.

Minimum metadata:

```json
{
  "session_id": "fw_...",
  "conversation_id": "optional",
  "workspace_root": "/abs/project",
  "target_path": "/abs/project/src/generated.ts",
  "display_path": "src/generated.ts",
  "mode": "create_or_overwrite",
  "status": "drafting",
  "created_at": "iso8601",
  "updated_at": "iso8601",
  "current_offset": 32768,
  "chunk_count": 4,
  "expected_bytes": 120000,
  "expected_sha256": null,
  "pre_state": {
    "exists": true,
    "size": 8192,
    "mtime": "iso8601",
    "sha256": "optional"
  }
}
```

The model-facing result should expose only useful fields; absolute internal draft paths do not need to be shown unless debugging is enabled.

## HTTP Streaming Timeout Policy

Replace the global 60-second total timeout for model streaming calls with role-specific timeouts.

Recommended MVP:

- Non-streaming API calls: keep a finite total timeout, configurable around 60-120 seconds.
- Streaming model calls: use connect timeout plus read-idle timeout, not total 60 seconds.
- Read-idle timeout should mean "no bytes/events received for N seconds", not "stream lasted N seconds".
- Long tool-argument streams should remain cancellable by user generation token.
- Error text should distinguish:
  - provider stream idle timeout;
  - local request total timeout;
  - user cancellation;
  - invalid provider response;
  - tool argument JSON failed before executable call.

## Frontend UX

Tool blocks should show the actual backend phase:

- `generating_arguments`: model is generating tool args; no file mutation has happened.
- `drafting_file`: draft session started, target untouched.
- `writing_draft`: chunk append in progress, show bytes/chunks.
- `applying_file_change`: final atomic commit or patch is running.
- `completed`: committed mutation with diff stats.
- `failed`: scoped failure with reason.
- `aborted`: draft retained or discarded; target untouched.

Large file write UI:

- Show target path.
- Show draft bytes received.
- Show chunk count.
- Show whether target file has been touched.
- On failure, say clearly: "目标文件未被修改" when failure happened before finish.

Patch/edit UI:

- Show operation, affected files, additions/removals.
- Show expandable diff.
- Show warnings/diagnostics.

Main conversation:

- If final synthesis fails after successful tool mutation, preserve the completed tool block and show final-response failure separately.
- Do not clear the assistant preview before persisted conversation state is applied.

## Prompting Rules

Update native tool descriptions and system prompt:

- Existing-file small edit: use `edit_file`.
- Multi-file or larger code change: use `patch`.
- New small file: `write_file` is acceptable.
- Large generated file or explicit full-file replacement: use `begin_file_write` + `append_file_write` + `finish_file_write`.
- Do not repeat full saved file content after successful mutation unless user explicitly asks.
- If a draft write fails, explain the draft/target state and retry/resume if possible.
- Before editing an existing file, use current read/search context unless the tool itself can prove the file state is unchanged.
- After a failed `edit_file`/`patch`, do not retry the same failing old text blindly; re-read or search first.
- Prefer `patch` over whole-file replacement for normal source-code work.
- Do not emit placeholders such as "original code here" inside generated patches or full file writes.
- Treat `write_file_chunk` as deprecated. The model-facing tool list should expose `begin_file_write`, `append_file_write`, `finish_file_write`, and `abort_file_write` instead.

## Reference-Aligned Tool Protocol

This is the concise implementation contract copied from the reviewed coding tools, adjusted for Kivio's failure mode.

| Work type | Reference to follow | Kivio tool flow | Backend invariant |
|---|---|---|---|
| Find files | OpenCode `glob` / `grep` | `glob_files` / `search_files` | Return bounded paths or path/line/context only; never dump full file bodies. |
| Read current context | OpenCode `read`, Anthropic `view`, Hermes `read_file(offset, limit)` | `stat_path` when needed, then `read_file(path, offset?, limit?)` | Return structured range metadata and record full/partial read state. |
| One local replacement | OpenCode `edit`, Anthropic `str_replace`, Hermes replace mode | `read_file` -> `edit_file` | Exact unique match unless `replace_all=true`; no-op/zero/multiple match fails before write. |
| Normal source-code edit | OpenAI/Codex `apply_patch`, OpenCode `apply_patch`, Aider `diff`/`udiff` | `search_files` / `read_file` -> `patch` | Validate every path/hunk before any target mutation; return landed diff metadata. |
| Small whole-file create/replace | OpenCode `write`, Anthropic `create`, Hermes `write_file` | `write_file` | Atomic temp+rename; reject placeholders/status stubs; warn on stale or partial-read overwrite. |
| Long generated full file | Kivio-specific draft protocol plus Hermes filesystem safety | `begin_file_write` -> `append_file_write`* -> `finish_file_write` | Append writes only to app-data draft; target remains untouched until verified finish. |
| Legacy chunk writer | No safe reference for direct target chunks | hidden `write_file_chunk` compatibility only | Must wrap draft sessions; direct target append is forbidden. |

The important distinction is that "chunked write" is not the product requirement. The product requirement is "durable draft first, verified atomic commit last". If a chunk API writes to the real target before final verification, it fails this PRD.

### Tool Contract In Plain Flow

`read_file`:

- Reads text through the native file tool, not shell `cat`.
- Uses `offset`/`limit` for large or focused reads.
- Returns content plus `resolved_path`, `total_lines`, `start_line`, `end_line`, `truncated`, `next_offset`, `file_size`, and read-state.
- Marks the file as fully read or partially read for later stale/full-overwrite warnings.

`edit_file`:

- Changes one exact known region in one existing file.
- Fails if the old text is missing, duplicated, or equal to the new text.
- On failure, tells the model to re-read/search or use `patch`, not retry blindly.

`patch`:

- Is the default for coding changes.
- Handles add/update/delete and multi-file work.
- Validates all paths and hunks first; only then enters the write phase.
- Uses atomic writes for updated/new files and returns affected files, additions/removals, diff, warnings, diagnostics, and `target_touched`.

`write_file`:

- Creates or explicitly replaces a small whole file.
- Is not the default existing-code edit path.
- Treats existing files as full replacement, with stale/partial-read warnings.
- Rejects placeholder content before opening a target for mutation.

`begin_file_write`:

- Creates a backend-owned draft session under app data.
- Captures target pre-state.
- Returns `target_touched=false`.

`append_file_write`:

- Appends UTF-8 bytes to the draft file only.
- Requires exact byte offset and optional chunk hash.
- Returns current offset and chunk count.
- Returns `target_touched=false`.

`finish_file_write`:

- Verifies expected bytes/hash when provided.
- Commits with the same atomic-write semantics as `write_file`.
- Returns `target_touched=true` only after success.
- On mismatch, keeps the draft recoverable and returns `target_touched=false`.

`abort_file_write`:

- Stops the draft.
- Never touches the target.
- Keeps or removes draft data according to retention policy only.

`write_file_chunk`:

- Is not a normal model-facing tool.
- If old callers use it, `start/append/finish` must map to the draft-session machinery.
- Any implementation that appends directly to the target path is a release blocker.

## Failure Recovery Policy

Failures should be scoped to the exact phase that failed:

| Failure | Target touched? | Required response |
|---|---:|---|
| Provider stream fails before a tool call starts | No | Preserve conversation turn and show provider stream error. |
| Provider stream fails after a tool-call draft starts but before arguments complete | No | Mark that tool draft error; do not pretend a write executed. |
| `edit_file` no match / duplicate match | No | Return tool error with re-read/search guidance. |
| `patch` parse/path/hunk validation fails | No | Reject before writing any file and return exact failing path/hunk. |
| `write_file` atomic temp write fails before rename | No | Return tool error and remove temp file. |
| `append_file_write` offset/hash mismatch | No | Return current offset/chunk count and keep draft. |
| `finish_file_write` byte/hash verification fails | No | Keep draft and report verification mismatch. |
| `finish_file_write` succeeds but final assistant synthesis fails | Yes | Keep completed tool block visible and show final-response failure separately. |

The frontend must not collapse these into one generic red "chat failed" state.

## Recovery Playbook

The model should recover like a coding agent, not like a text generator:

- `edit_file` no match: run `search_files` or `read_file` on the current target and retry with current text.
- `edit_file` multiple matches: use a larger unique `old_string`, set `replace_all=true` only when every match should change, or use `patch`.
- `patch` invalid context: re-read the specific file section and produce a smaller patch.
- `write_file` placeholder rejection: regenerate the complete small file or switch to `patch`/draft session.
- `append_file_write` offset mismatch: continue from the returned `current_offset`; do not restart by overwriting the target.
- `finish_file_write` hash/byte mismatch: append missing draft content or abort; never commit unknown content.
- Provider stream interruption during argument generation: mark the current tool draft failed and keep the conversation inspectable.

## Backend Requirements

- Implement structured `FileMutationResult` for all mutation tools.
- Implement V4A-style `patch` with add/update/delete.
- Implement file mutation path locks.
- Implement draft write sessions with metadata and atomic finish.
- Implement stale/pre-state checks for existing target overwrite.
- Implement read-state tracking for existing-file mutation warnings.
- Make `write_file` atomic using same-directory temp + rename and preserve mode/line endings/BOM where practical.
- Keep target file untouched until `finish_file_write`.
- Implement same-process per-path locks for `edit_file`, `patch`, `write_file`, and `finish_file_write`; multi-file patches acquire sorted locks.
- Implement sensitive/system/credential path deny checks before write operations.
- Implement post-write verification for patch/write where cheap: re-read changed text or stat/hash to confirm the intended content landed.
- Persist tool records even when provider stream fails during argument generation.
- Convert provider stream interruption during started tool arg generation into a scoped draft/tool error, not a whole-turn invoke crash.
- Split HTTP timeout behavior for streaming vs non-streaming calls.
- Ensure cancellation marks active draft sessions as aborted or resumable.
- Remove or hide unsafe direct-target `write_file_chunk` schema; any compatibility path must use draft sessions internally.
- Treat draft storage under app data as feature state, not disposable HTTP/browser cache.
- Add tests that assert `target_touched=false` for every pre-commit draft failure.

## Frontend Requirements

- Render `FileMutationResult` metadata in `ToolCallBlock`.
- Render draft-session progress from backend events.
- Avoid marking a draft-generation failure as an applied file change.
- Show target untouched/modified state explicitly for failed large writes.
- Keep completed file mutation blocks visible when final model synthesis fails.

## Verification Requirements

### Rust Tests

- `read_file` returns range metadata and large-file hints.
- `edit_file` exact replacement success.
- `edit_file` rejects duplicate match unless `replace_all`.
- `patch` add/update/delete success.
- `patch` rejects path traversal and absolute paths.
- `patch` failure validates before write and does not partially modify files.
- `write_file` uses atomic temp+rename semantics and preserves existing mode where supported.
- `write_file` preserves CRLF and UTF-8 BOM where practical.
- `begin_file_write` creates draft but does not touch target.
- `append_file_write` enforces offset.
- `finish_file_write` atomically commits and returns diff stats.
- failed/aborted draft leaves target unchanged.
- deprecated `write_file_chunk` is not exposed or is backed by draft sessions.
- stream interruption after tool-call draft start preserves tool record as error.
- streaming timeout helper does not enforce 60-second total timeout on active SSE streams.

### Frontend Checks

- Typecheck for new structured metadata.
- Tool block renders patch/edit/write-session states.
- Failed draft write displays target untouched.
- Completed tool block survives final provider failure.

### Manual Smoke Tests

- Ask agent to edit one line in an existing file.
- Ask agent to apply a multi-file patch.
- Ask agent to create a small file.
- Ask agent to generate a large file, then simulate provider interruption before finish; target must remain unchanged and UI must show draft failure.
- Ask agent to generate a large file successfully; final target must match expected size/hash and UI must show completed mutation.

## Phased Delivery

### Phase 1: Stop Misleading Failures

- Split streaming timeout from global HTTP timeout.
- Keep provider-stream interruptions scoped when tool-call draft already started.
- Ensure frontend does not label generated tool arguments as completed file writes.

### Phase 2: Patch/Edit As Default Coding Path

- Finalize structured mutation metadata.
- Ship `patch` add/update/delete.
- Update prompts and tool descriptions.
- Update `ToolCallBlock` diff rendering.
- Add read-state stale warnings and exact failure recovery hints.

### Phase 3: Durable Large File Write Sessions

- Add draft session storage.
- Add begin/append/finish/abort tools.
- Add atomic finish and offset/hash validation.
- Add UI progress and failure state.
- Hide or reimplement `write_file_chunk` so no direct target chunk write remains exposed.
- Update model prompt guidance so long file generation uses session tools, while normal code changes stay on `patch`.

### Phase 4: Hardening

- Add stale target warnings.
- Add cleanup job for abandoned drafts.
- Add optional pre-apply diff approval.
- Add lightweight diagnostics where cheap.
- Add better Windows atomic replacement semantics where available, while preserving the "no direct chunk writes to target" guarantee.

## Acceptance Criteria

- [ ] Existing-file code edits prefer `edit_file` or `patch`, not whole-file `write_file`.
- [ ] Large generated files use draft write sessions instead of one huge `write_file(content)` call.
- [ ] Model-facing prompt and schema text include `begin_file_write`, `append_file_write`, and `finish_file_write` for long writes.
- [ ] Model-facing prompt and schema text do not recommend `write_file_chunk`.
- [ ] If provider stream interrupts before `finish_file_write`, target file is unchanged.
- [ ] `write_file_chunk` no longer writes directly to target files.
- [ ] If append offset is wrong, write session fails safely without target mutation.
- [ ] `read_file` returns path, resolved path, content range, total lines, truncation, and next-offset metadata.
- [ ] Full overwrite after only partial read state warns or blocks and recommends `patch`/`edit_file` or a full read.
- [ ] `patch` validates every path/hunk before writing and rejects absolute, `~`, backslash, and `..` paths.
- [ ] `write_file`, `edit_file`, `patch`, and successful `finish_file_write` use atomic same-directory temp replacement or equivalent semantics.
- [ ] Line endings, UTF-8 BOM, and existing file mode are preserved where practical.
- [ ] Same-process path locks serialize concurrent writes to the same resolved path.
- [ ] If final synthesis fails after a successful mutation, the mutation remains visible as completed.
- [ ] Streaming requests are not killed by a 60-second total timeout while bytes/events are still arriving.
- [ ] Tool UI distinguishes argument generation, draft writing, final apply, completed, failed, and aborted.
- [ ] Existing-file mutation results include stale/wrong-cwd warnings when applicable.
- [ ] Rust tests for patch/edit/write-session safety pass.
- [ ] `npm run typecheck` passes.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml chat::agent:: -- --nocapture` passes for agent-loop changes.

## Out Of Scope

- Full IDE editor experience.
- Binary file editing.
- LSP-level semantic edit tools.
- Cross-agent collaborative merge UI.
- Git checkpoint/rollback UI.
- Cloud sync of draft sessions.

## Open Questions

- Should the public tool names be separate (`begin_file_write`, `append_file_write`, `finish_file_write`, `abort_file_write`) or one tool with `mode`? Recommended: separate tools for clearer schemas and better UI labels.
- Should failed draft sessions be resumable by the model in the same conversation only, or across app restart? Recommended MVP: across app restart within retention window.
- Should `write_file` remain enabled for project coding prompts once write sessions exist? Recommended: keep it, but add size/context guidance and prefer write sessions for large content.

## Implementation Notes

Likely backend files:

- `src-tauri/src/api.rs`
- `src-tauri/src/chat/model/openai.rs`
- `src-tauri/src/chat/model/anthropic.rs`
- `src-tauri/src/chat/agent/loop_.rs`
- `src-tauri/src/chat/agent/stream.rs`
- `src-tauri/src/chat/agent/prepare.rs`
- `src-tauri/src/mcp/types.rs`
- `src-tauri/src/mcp/registry.rs`
- `src-tauri/src/native_tools/files.rs`
- `src-tauri/src/native_tools/mod.rs`

Likely frontend files:

- `src/chat/ToolCallBlock.tsx`
- `src/chat/types.ts`
- `src/api/tauri.ts`

Important caution:

- Do not ship a direct target-file `write_file_chunk(start/append/finish)` implementation. It solves progress display but creates a corruption risk. The correct implementation must write to a draft and commit atomically only at finish.
