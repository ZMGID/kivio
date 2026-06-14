# Agent File Editing Patterns Research

## Purpose

Summarize how established coding agents handle file creation and code edits, then map those patterns to Kivio's current Chat native tools.

## Sources Reviewed

- OpenAI apply_patch tool guide: https://developers.openai.com/api/docs/guides/tools-apply-patch
- OpenAI Codex CLI overview: https://developers.openai.com/codex/cli
- OpenCode tools docs: https://opencode.ai/docs/tools/
- Aider edit formats docs: https://aider.chat/docs/more/edit-formats.html
- Aider unified diff notes: https://aider.chat/docs/unified-diffs.html
- Anthropic text editor tool docs: https://docs.anthropic.com/en/docs/agents-and-tools/tool-use/text-editor-tool
- OpenCode Go write tool: https://github.com/opencode-ai/opencode/blob/73ee4932/internal/llm/tools/write.go
- OpenCode TypeScript edit tool: https://github.com/anomalyco/opencode/blob/5c5069b6/packages/opencode/src/tool/edit.ts
- OpenAI Codex apply_patch runtime: https://github.com/openai/codex/blob/35aaa5d9/codex-rs/apply-patch/src/lib.rs
- Aider edit formats documentation: https://github.com/Aider-AI/aider/blob/main/aider/website/docs/more/edit-formats.md
- Local Hermes Agent:
  - `/Users/zmair/.hermes/hermes-agent/tools/file_tools.py`
  - `/Users/zmair/.hermes/hermes-agent/tools/file_operations.py`
  - `/Users/zmair/.hermes/hermes-agent/tools/file_state.py`
  - `/Users/zmair/.hermes/hermes-agent/tools/patch_parser.py`
- Current Kivio:
  - `src-tauri/src/mcp/types.rs`
  - `src-tauri/src/native_tools/files.rs`
  - `src-tauri/src/chat/agent/prepare.rs`

## 2026-06-10 Evidence Refresh

Re-checked with live docs/source references on 2026-06-10:

- `smart-search fetch https://developers.openai.com/api/docs/guides/tools-apply-patch --format json`
- `smart-search fetch https://opencode.ai/docs/tools/ --format json`
- `smart-search fetch https://aider.chat/docs/more/edit-formats.html --format json`
- `smart-search fetch https://docs.anthropic.com/en/docs/agents-and-tools/tool-use/text-editor-tool --format json`
- Fallback live checks with `curl` against the same public pages confirmed:
  - OpenAI page metadata describes `apply_patch` as structured diffs applied by the integration.
  - OpenCode docs list separate built-ins for `edit`, `write`, `read`, `grep`, `glob`, and `apply_patch`; `read` supports line ranges and `apply_patch` embeds project-relative paths.
  - Aider docs describe `whole`, `diff`, and `udiff`; `whole` is simple but slow/costly because the model returns the whole file.
  - Anthropic's public page is a rendered Next app, but page metadata confirms the text editor tool page; existing documented commands remain `view`, `create`, `str_replace`, and `insert`.
- Local Hermes inspection with `rg` and targeted reads over:
  - `/Users/zmair/.hermes/hermes-agent/tools/file_state.py`
  - `/Users/zmair/.hermes/hermes-agent/tools/file_operations.py`
  - `/Users/zmair/.hermes/hermes-agent/tools/patch_parser.py`

## 2026-06-10 Second-Pass Source Check

The second pass focused on whether the PRD should be stricter than "add chunked writes".

Commands used:

- `smart-search fetch "https://developers.openai.com/api/docs/guides/tools-apply-patch" --format json`
- `smart-search fetch "https://opencode.ai/docs/tools/" --format json`
- `smart-search fetch "https://aider.chat/docs/more/edit-formats.html" --format json`
- `smart-search fetch "https://docs.anthropic.com/en/docs/agents-and-tools/tool-use/text-editor-tool" --format json`
- Local source reads from Hermes `file_tools.py`, `file_operations.py`, `file_state.py`, and `patch_parser.py`.

Second-pass conclusions:

- The core bug is not "no progress bar". The core bug is that Kivio lets large generated file content live inside one fragile model/tool argument until the provider finishes streaming.
- `write_file_chunk` is not an established coding-agent pattern. Direct target chunk append is worse than one-shot `write_file` because an interruption can leave a corrupted target. It is acceptable only as a compatibility wrapper over draft sessions.
- OpenAI/Codex and OpenCode both support the stronger conclusion that normal code edits should be patch/edit operations with host-side application and recoverable tool results.
- Aider supports the same conclusion from the token/edit-format side: whole-file output is allowed, but it is not a good default for ordinary code edits.
- Anthropic text editor docs support narrow tool roles: view current content, create new files, exact string replacement, and insert at a line. Kivio should map this to `read_file`, `write_file`, `edit_file`, and future insert/patch behavior rather than one broad write tool.
- Hermes is the best local implementation reference for filesystem correctness, but Kivio should copy the semantics rather than the transport. In Rust/Tauri that means native path validation, path locks, read-state registry, same-directory temp writes, BOM/line-ending preservation, and structured results.
- The draft-session protocol is Kivio-specific. None of the reviewed tools fully addresses Kivio's observed failure mode where the provider disconnects while streaming one giant JSON `content` argument before the backend can write anything.

Implementation consequence:

```text
Do not optimize the unsafe flow.
Replace it:
  normal code edits -> patch/edit_file
  small explicit files -> write_file
  large full-file generation -> begin/append/finish draft session
```

The PRD and backend spec now treat this as a normative rule, not a UI preference.

## 2026-06-10 Third-Pass Specification Refresh

This pass focused on making the PRD and backend spec precise enough to drive implementation review and tests, not just general direction.

Commands used:

- `smart-search fetch "https://developers.openai.com/api/docs/guides/tools-apply-patch" --format json`
- `smart-search fetch "https://opencode.ai/docs/tools/" --format json`
- `smart-search fetch "https://aider.chat/docs/more/edit-formats.html" --format json`
- `smart-search fetch "https://docs.anthropic.com/en/docs/agents-and-tools/tool-use/text-editor-tool" --format json`
- Local Hermes inspection with `rg` over `file_tools.py`, `file_operations.py`, `file_state.py`, and `patch_parser.py`.

Confirmed points:

- OpenAI's apply-patch guide treats file changes as structured operations that the host integration applies and reports back as completed or failed. That supports Kivio making `patch` a host-validated runtime operation, not model prose.
- OpenCode keeps `read`, `grep`, `glob`, `edit`, `write`, and `apply_patch` as separate built-ins. That supports Kivio keeping narrow tool ownership instead of expanding `write_file` into a mega-tool.
- Aider explicitly calls whole-file editing simple but slow/costly because the model returns the entire file. That supports keeping `write_file` for small whole-file create/replace, not normal code edits.
- Anthropic's text editor tool separates `view`, `create`, `str_replace`, and `insert`, and uses line ranges for view. That supports Kivio's `read_file` range contract and exact `edit_file` replacement contract.
- Hermes local code has the concrete filesystem safety behavior Kivio should copy semantically: read-state registry, per-path locks, stale warnings, same-directory temp writes, mode/line-ending/BOM preservation, post-write verification, and recovery hints.

Spec consequence:

```text
Tool docs and prompts choose the route.
Backend contracts enforce the route.
UI only renders backend truth; it does not infer mutation state.
```

The stricter backend spec now requires:

- structured `ReadFileResult`;
- structured `FileMutationResult`;
- read-state tracking;
- a shared atomic write contract;
- draft storage as feature state under app data;
- prompt/schema review rules;
- `write_file_chunk` hidden or implemented only as a draft-session compatibility wrapper.

## 2026-06-10 Fourth-Pass Contract Refresh

This pass was triggered by the question: "The direction looks okay, but are the rules complete enough?" The answer is that the PRD must be judged tool-by-tool, not only by the high-level idea of "chunked writes".

Commands used:

- `smart-search fetch "https://developers.openai.com/api/docs/guides/tools-apply-patch" --format json`
- `smart-search fetch "https://opencode.ai/docs/tools/" --format json`
- `smart-search fetch "https://aider.chat/docs/more/edit-formats.html" --format json`
- `smart-search fetch "https://docs.anthropic.com/en/docs/agents-and-tools/tool-use/text-editor-tool" --format json`
- Local Hermes inspection with `rg` over `/Users/zmair/.hermes/hermes-agent/tools/file_tools.py`, `file_operations.py`, `file_state.py`, `patch_parser.py`, and `fuzzy_match.py`.

Confirmed source-backed rules:

- OpenAI apply-patch: the model proposes structured create/update/delete operations; the host applies them and sends back `completed` or `failed` tool output. Kivio should copy this host-owned application model for `patch`.
- OpenCode: `edit`, `write`, `read`, `grep`, `glob`, and `apply_patch` are distinct built-ins, even though file mutations share one edit permission. Kivio should keep narrow tool ownership instead of making `write_file` smarter and broader.
- Aider: the `whole` edit format is simple but slow/costly because the model returns the whole file; `diff` and `udiff` return only changed regions. Kivio should treat whole-file writing as an exception for small/new/full-rewrite cases.
- Anthropic text editor: `view`, `create`, `str_replace`, and `insert` are separate commands; `view` supports ranges and `str_replace` is precise replacement. Kivio should keep `read_file` and `edit_file` strict instead of fuzzy/direct overwrite behavior.
- Hermes: read state, per-path locks, stale-write warnings, line-ending/BOM preservation, same-directory temp replacement, verification, and recovery hints are the practical local filesystem layer Kivio needs in Rust/Tauri.

Normative split after the fourth pass:

```text
Find context:
  list_dir / glob_files / search_files / stat_path / read_file

Modify existing code:
  edit_file for one exact block
  patch for normal source changes, multi-region changes, and multi-file changes

Create or explicitly replace a small whole file:
  write_file

Generate or replace a long whole file:
  begin_file_write -> append_file_write* -> finish_file_write
```

The key correction is that `write_file_chunk` is not the feature. A direct target chunk writer is the broken shape. The feature is a backend-owned draft session whose final commit is atomic and whose pre-finish failures always report `target_touched=false`.

Tool-by-tool adoption:

| Kivio tool | Reference behavior copied | Required Kivio rule |
|---|---|---|
| `read_file` | OpenCode range read, Anthropic `view`, Hermes offset/limit/read-state | Return structured range metadata and record full vs partial context. |
| `edit_file` | OpenCode `edit`, Anthropic `str_replace`, Hermes replace mode | Exact unique replacement only; no silent fuzzy mutation; failure tells model to re-read/search. |
| `patch` | OpenAI/Codex apply_patch, OpenCode apply_patch, Aider diff/udiff | Default coding edit path; validate paths/hunks before touching targets. |
| `write_file` | OpenCode write, Anthropic create, Hermes atomic write | Small create/explicit full replace only; atomic temp+rename; reject placeholders. |
| Draft session tools | Kivio-specific, with Hermes filesystem safety | Durable draft storage under app data; target untouched until verified finish. |
| `write_file_chunk` | No good external reference for direct target chunks | Hide/deprecate or wrap over draft sessions; never append to target directly. |

Implementation review should therefore ask one blunt question for every change: "Did this reduce the mutation surface, or did it recreate one huge fragile write under another name?"

### OpenAI / Codex

- Official OpenAI `apply_patch` docs describe a model emitting structured file operations and the host application applying them, then returning one `apply_patch_call_output` with `completed` or `failed`.
- Supported patch operations are create, update, and delete files. The host harness is responsible for path validation, interpreting V4A diffs, applying changes, and returning recoverable failure messages.
- The docs explicitly recommend clear file context, small focused diffs, and returning helpful failed output so the model can re-read or simplify the change.
- Codex runtime sources reinforce the same pattern: parse patch, validate hunk shape, resolve affected paths, run under the tool runtime, and surface patch approval/result metadata.

Kivio adoption:

- `patch` should be the default code edit primitive.
- `patch` results must be structured and recoverable, not just a generic red provider error.
- Failed patch application should be a tool result that the model can react to in the next step.

### OpenCode

- OpenCode public docs list distinct built-in tools: `read`, `grep`, `glob`, `edit`, `write`, and `apply_patch`.
- `edit` is described as the primary way to modify code through exact replacements.
- `write` creates or overwrites files, and is controlled by the same `edit` permission as `edit` and `apply_patch`.
- `read` supports line ranges for large files, while `grep`/`glob` are separate discovery tools.
- `apply_patch` embeds project-relative paths inside marker lines such as `*** Add File: ...`, `*** Update File: ...`, and `*** Delete File: ...`.
- OpenCode implementation examples add practical details: per-file locks, BOM/line-ending handling, diff metadata, LSP diagnostics, and file watcher events.

Kivio adoption:

- Keep discovery, read, targeted edit, whole write, and patch as separate tool roles.
- Keep file mutation UI and permission grouping unified, but do not collapse every operation into `write_file`.
- Return operation metadata that can drive UI labels and diff rendering.

### Aider

- Aider documents multiple edit formats. Its `whole` format is simple because the model returns the whole updated file, but it is slow/costly because even small edits require returning the entire file.
- Aider's `diff` and `udiff` formats are more efficient because the model only returns changed parts of a file.
- The Aider docs call out placeholder/lazy-code risks in whole-file style workflows.

Kivio adoption:

- Whole-file `write_file` should stay available, but it should not be the normal coding path.
- Existing source-code edits should prefer `edit_file` or `patch`.
- Tool descriptions should explicitly discourage placeholder content in generated files or patches.

### Hermes Agent

Local code inspection confirms Hermes has several safety patterns worth copying:

- `read_file(path, offset, limit)` returns paginated content and hints for continuing large reads.
- Read wrappers block obvious device/binary/credential paths and prevent repeated identical reads from wasting context.
- Writes use a temp file in the same target directory and rename it over the target, so failed writes leave the original target intact.
- Writes preserve existing file mode, CRLF line endings, and UTF-8 BOM when practical.
- Patch/write wrappers use per-path locks, stale-read warnings, wrong-cwd warnings, and absolute resolved-path reporting.
- Patch failures include recovery hints such as re-reading current file content or using more unique context.

Kivio adoption:

- Use Hermes as the practical local-filesystem model: atomic writes, preservation, read-state warnings, path locks, diagnostics, and recovery hints.
- Do not copy Hermes' terminal-shell implementation directly; implement equivalent behavior in Rust/Tauri native tools.
- Copy the concept of a process-wide file state registry: record reads, note writes, check stale state, and lock by resolved path.
- Copy the atomic-write shape, not the exact shell implementation: write temp beside target, preserve existing mode, stream/write content safely, rename over target, and clean temp on failure.
- Copy the line-ending/BOM behavior: read/display may strip BOM for the model, but writes/patches should restore it when the original file had it.

### Claude Code Style

- Anthropic's public text editor tool is not the same thing as Claude Code's private internal implementation, but it gives a concrete official file-editing contract.
- The tool supports distinct commands: `view`, `create`, `str_replace`, and `insert`.
- `view` can inspect files/directories and supports line ranges.
- `str_replace` is for precise replacement and expects exact old/new text.
- The implementation guidance calls out path validation, permission checks, backups/recovery, unique matching, error handling, and verifying changes.

Kivio adoption:

- Treat Claude's text editor contract as secondary support for separate read/create/replace/insert roles.
- Copy the unique-match and verification posture into `edit_file`.
- Do not block implementation on matching Claude Code internals exactly.

## Updated Reference Matrix

| Reference | What It Proves | Kivio Rule |
|---|---|---|
| OpenAI apply_patch | Model emits structured diffs; host validates/applies and returns tool output. | `patch` is the normal multi-file/larger edit primitive. |
| OpenCode tools | `read`, `grep`, `glob`, `edit`, `write`, and `apply_patch` are separate roles under edit permissions. | Do not collapse discovery/read/edit/write into one mega-tool. |
| Aider edit formats | Whole-file edits are simple but token-heavy; diff/udiff edits are more efficient and reduce lazy placeholders. | Whole-file `write_file` is not the default code-edit path. |
| Anthropic text editor | Official contract separates `view`, `create`, `str_replace`, and `insert`; exact replacements need unique matching and verification. | `edit_file` must validate uniqueness and tell the model to re-read on mismatch. |
| Hermes local code | Production local-file safety: offset reads, locks, stale warnings, temp writes, line-ending/BOM preservation, diagnostics. | Kivio's Rust tools must implement these filesystem safety mechanics. |
| Kivio-specific failure | Provider can fail while streaming one huge `write_file.content` argument. | Add backend-owned draft write sessions for large files. |

## Current Kivio Behavior

- `write_file` requires `path` and full `content`.
- Backend writes with a single `fs::write(&full, content)`.
- `edit_file` exists, but it is a minimal exact `old_string` -> `new_string` replacement.
- Tool UI currently shows a tool block lifecycle, but does not expose diff, per-file verification, or a separate "generating file content" state.
- If provider synthesis/planning fails after the tool ran, the user may see a final red provider error even though file mutation may already have landed.

## Design Tightening After Refresh

The stronger rule after reviewing the references is:

```text
read/discover tools locate context
edit/patch tools change existing code
write tools create or explicitly replace whole files
draft-session tools handle long generated content
```

Kivio should avoid an ambiguous "write anything" tool posture. This is where the current behavior goes wrong: a long generated file and a tiny source edit both look like one huge `write_file.content` JSON argument. Established coding tools avoid that by giving the model smaller, more structured edit surfaces.

The Hermes-specific addition is backend trustworthiness. Tool schemas alone are not enough. The runtime must enforce:

- path normalization and denied write paths;
- offset/limit reads for large files;
- repeated-read/search loop warnings;
- read-state tracking for stale writes;
- sorted per-path locks;
- same-directory temp writes with atomic rename;
- line-ending and UTF-8 BOM preservation;
- post-write verification and diagnostics where cheap.

The Kivio-specific addition is draft sessions. None of the references fully solve the "provider disconnects while streaming one giant JSON tool argument" problem for large generated deliverables. Kivio needs `begin_file_write` / `append_file_write` / `finish_file_write` so partial progress is backend-owned and the target remains untouched until commit.

## Adopted Reference Split

Kivio should not copy one agent wholesale. Each reference owns a different layer of the design:

| Layer | Reference to copy | Concrete Kivio behavior |
|---|---|---|
| Public code-edit protocol | Codex/OpenAI `apply_patch` | Model proposes structured add/update/delete edits; Kivio applies them and returns explicit completed/failed tool output. |
| Tool boundaries | OpenCode | Keep discovery, read, edit, write, and patch as separate tools under one file-edit permission group. |
| Edit-format discipline | Aider | Avoid whole-file output for normal code edits; prefer diff/patch because it is smaller and less prone to placeholder code. |
| Exact replacement contract | Anthropic text editor | Require current viewed context and exact unique replacement for `edit_file`. |
| Filesystem safety | Hermes | Track reads, lock paths, warn on stale writes, preserve line endings/BOM, write temp beside target, return diffs/diagnostics. |
| Large generation recovery | Kivio-specific | Use backend-owned draft sessions so provider interruption does not corrupt or erase target-file progress. |

That means the answer to "参考谁" is:

```text
Normal code edits: Codex/OpenAI + OpenCode + Aider
Exact one-block edit: Claude text editor + Hermes
Local filesystem correctness: Hermes
Large generated files: Kivio's draft-session extension
```

## Patterns From Other Coding Agents

### Read and discovery are separate from mutation

- Coding agents generally gather context before writing: list/glob/search locate files, then file reads pull just enough context for the edit.
- Aider asks users to add relevant files to the chat, but its edit-format docs still distinguish whole-file replacement from diff/search-replace formats.
- Kivio should keep this split explicit in tools: discovery tools return paths/matches, `read_file` returns content, mutation tools mutate only after validation.

### Whole-file write exists, but is not the preferred code-edit primitive

- OpenCode has a write tool that accepts full file content. It adds safety around it: read-before-write checks, diff generation, permission request with diff metadata, history versions, and LSP diagnostics.
- Hermes also keeps `write_file`, but its schema explicitly says it completely replaces existing content and directs targeted edits to `patch`.
- Aider documents whole-file editing as the simplest approach, but slow and expensive because the model must return the entire file even for small changes.

### Patch/search-replace is the preferred targeted-edit primitive

- Codex uses an `apply_patch` flow where the model emits add/update/delete patch hunks, then the runtime parses and applies them.
- OpenAI's apply_patch guide frames this as structured diffs that the application applies and reports back on, rather than model prose that the user manually integrates.
- OpenCode groups `edit`, `write`, and `apply_patch` under one edit permission, but keeps their purposes separate: edit targeted content, write create/overwrite, apply_patch patch text with paths embedded in marker lines.
- OpenCode TypeScript uses an `edit` tool with `oldString`, `newString`, `replaceAll`, locking, diff metadata, formatting, file watcher events, and diagnostics.
- Hermes uses a `patch` tool with:
  - replace mode: `path`, `old_string`, `new_string`, `replace_all`
  - patch mode: V4A multi-file patch text
  - fuzzy matching strategies
  - unified diff return
  - lint/LSP diagnostics
  - path locks
  - stale file and wrong-cwd warnings
  - file mutation verifier when writes/patches fail
- Aider supports multiple edit formats: whole file, search/replace diff blocks, unified diff, and architect/editor separation.
- Aider's docs call whole-file editing simple but slow/costly, and its unified diff notes are a useful warning that large files and lazy placeholder edits need structured, verifiable edit formats.

### Hermes details worth copying

- `read_file` supports `offset` and `limit`, so large files can be sampled by line range instead of loaded wholesale.
- `write_file` uses temp-file plus same-directory rename for atomic replacement.
- Writes preserve line endings and UTF-8 BOM when the original file had them.
- `patch` aggregates per-file LSP diagnostics and returns diffs.
- Tool wrappers track stale reads and warn when a file changed since the agent last read it.
- File operations use locks/registries so concurrent edits to the same path are serialized.
- A path lock wraps the whole read/modify/write section for a resolved path.
- A write is treated as an implicit fresh read for the writer, so future stale checks know the agent has current content.
- Partial reads are tracked separately; overwriting a file after seeing only a slice should warn.
- Atomic writes stream content through stdin/temp storage instead of putting large content in a shell command string. Kivio should mirror this idea in Rust rather than via shell.

### Hermes implementation details from local code

The local Hermes code gives the strongest concrete implementation reference:

- `file_tools.py` schema tells the model to use `read_file` instead of terminal `cat/head/tail`, and says reads over the character limit must use `offset`/`limit`.
- `file_tools.py` tracks repeated reads by resolved path plus offset/limit and escalates repeated unchanged reads into compact guidance instead of resending the same content forever.
- `file_state.py` records per-agent reads as `(mtime, read_ts, partial)`, notes writes as implicit fresh reads, and exposes `check_stale` plus `lock_path`.
- `file_operations.py` detects CRLF and UTF-8 BOM, strips BOM on read/display, restores BOM on write when the original had one, and normalizes line endings before write.
- `file_operations.py` writes atomically by creating a temp file in the target directory, preserving mode best-effort, writing content through stdin/temp storage, and renaming over the target.
- `file_tools.py` wraps write/patch operations in per-path locks, checks stale state before writing, and adds warnings rather than silently overwriting sibling/external changes.

Kivio should copy the semantics, not Hermes' shell transport. In Kivio those semantics belong in Rust native tools and Tauri app-data storage.

## Updated 2026-06-10 Design Tightening

After the extra review, the recommended split is stricter:

```text
normal code edits -> patch/edit_file
small explicit create/overwrite -> write_file
large generated full file -> begin_file_write/append_file_write/finish_file_write
```

`write_file_chunk` is not a reference pattern from the reviewed tools. It is only acceptable as a backward-compatible wrapper over draft sessions. A direct target append design is worse than the current `write_file` design because it can corrupt the target file when the provider disconnects halfway through generation.

The strongest norm to preserve is "smallest safe mutation surface":

- If the agent only needs to change one block, use `edit_file`.
- If the change spans regions or files, use `patch`.
- If the agent only read a partial slice, do not rewrite the full file.
- If the content is large and new, stream it into backend-owned draft storage.
- If a failure happens before verified finish, target file remains untouched and the tool result says so.

### Common safety and UX practices

- Generate a diff before applying or immediately after applying.
- Ask for approval on sensitive file mutations, ideally showing the diff.
- Preserve line endings and BOM where practical.
- Prevent stale writes by tracking whether the file has changed since the agent read it.
- Lock per path for concurrent edits.
- Run syntax/lint/LSP diagnostics after writes/patches, and surface only relevant new errors where possible.
- Report the actual path modified.
- Do not let a post-mutation provider error erase or obscure the fact that a file operation succeeded.
- Do not represent "tool arguments are still streaming" as a real file write. It is only a pending tool-call draft until backend dispatch begins.

## Recommended Direction For Kivio

- Keep `write_file` for small new files and explicit full-file generation, but make it atomic.
- Make targeted edits first-class:
  - either add a `patch` tool or significantly enhance `edit_file`;
  - support exact replace first, then V4A patch mode;
  - return unified diff and metadata to the UI.
- Make read/discovery first-class:
  - use `glob_files` and `search_files` to find candidates;
  - use `read_file(offset, limit)` for partial context;
  - reject huge whole-file reads where partial reads would be safer.
- Replace direct target-file chunk writing with draft sessions:
  - chunks append to a draft under app data;
  - `finish` verifies byte/hash expectations;
  - only `finish` mutates the target via atomic replace.
- Hide or deprecate `write_file_chunk` unless it is reimplemented on top of draft sessions.
- Update prompts so coding edits prefer patch/edit over whole-file write.
- Update frontend tool UI so file mutations show file path, operation type, status, diff summary, and diagnostics.
- Treat post-tool synthesis/provider failures as final-response failures, not as if the file mutation itself failed.

## Concrete Tool Flow Recommendation

### Reading

Use native file tools, not shell commands, for agent-visible file reads:

```text
list_dir / glob_files / search_files
stat_path when size/type matters
read_file(path) for small files
read_file(path, offset, limit) for large or targeted context
```

Required backend behavior:

- Return range metadata so the model can continue reading without guessing.
- Track whether the read was full or partial.
- Reject device/binary/secret paths.
- Warn or suppress repeated identical reads of unchanged content.

### Existing-file modification

Use `edit_file` only for one exact block:

```text
read_file current context
edit_file(path, old_string, new_string, replace_all=false)
```

Use `patch` for normal coding edits:

```text
search_files/read_file
patch("*** Begin Patch ...")
```

Required backend behavior:

- Validate every path/hunk before mutation.
- Acquire sorted per-path locks.
- Use atomic writes for add/update operations.
- Return diff, additions/removals, warnings, diagnostics, and structured failure messages.

### Small file creation

Use `write_file` only for small whole-file content:

```text
write_file(path, content)
```

Required backend behavior:

- Write atomically.
- Preserve mode/line endings/BOM where practical.
- Reject placeholder/status-stub content.
- Warn on stale existing targets.

### Large generated files

Use draft sessions:

```text
begin_file_write(path, mode)
append_file_write(session_id, offset, content)*
finish_file_write(session_id, expected_bytes?, expected_sha256?)
```

Required backend behavior:

- Store chunks under app support data, not browser/http cache and not beside the target.
- Enforce exact offsets.
- Keep target untouched until finish.
- Commit atomically only after final verification.
- Return `target_touched=false` for every pre-commit failure.

### Failure and UI

The UI should separate:

```text
model generating tool args
tool executing
draft updated
target committed
final assistant answer failed
```

This is important because the user's observed failure happened during tool planning/response decoding, not necessarily during a backend write.

## Tool-Level Recommendation

| User Intent | First Tool Choice | Why |
|---|---|---|
| "Find where X is used" | `search_files` | Cheap path/line/context discovery without loading whole files. |
| "Show me this file" | `read_file` | Canonical text read with ranges and metadata. |
| "Change this one exact block" | `edit_file` | Exact replacement with uniqueness checks. |
| "Modify several files" | `patch` | Structured add/update/delete operations. |
| "Create a small config/file" | `write_file` | Simple whole-file creation, with atomic backend write. |
| "Generate a long file/demo/data artifact" | write session tools | Avoids losing all generated content on one provider disconnect. |
| "Append chunks directly to target" | none | Forbidden; route through draft storage only. |

## Final Design Stance

Kivio should mainly copy Codex/OpenAI and OpenCode for the public tool model:

```text
discover -> read targeted context -> patch/edit -> report structured result
```

Kivio should copy Hermes for local filesystem correctness:

```text
track reads -> lock paths -> write temp beside target -> rename atomically -> preserve mode/line endings/BOM -> return diff/warnings
```

Kivio should add its own draft-session protocol for long generated files:

```text
begin_file_write -> append_file_write* -> finish_file_write
```

That last piece is necessary because Kivio's current failure happens while a provider is generating a huge `write_file.content` argument. Patch tools solve normal code editing, but long new file generation still needs a resumable backend-owned draft flow.

## What Not To Copy

- Do not copy a direct target `start/append/finish` chunk protocol. It is not the pattern the reviewed coding tools use for safe code edits.
- Do not copy Hermes' shell-based implementation details. Kivio should implement equivalent safety in Rust native tools.
- Do not make `write_file` smarter until it becomes a mega-tool. The references point the other way: separate read/search/edit/write/patch roles.
- Do not rely on frontend state to infer that file writing happened. Backend tool results must own `target_touched`, offsets, session status, and diffs.
- Do not treat app-data draft storage as disposable cache. Drafts are feature state for resumable writes and must be cleaned only by draft-retention logic.
