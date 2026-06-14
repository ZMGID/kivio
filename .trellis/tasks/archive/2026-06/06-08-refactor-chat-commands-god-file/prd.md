# Refactor Chat Commands God File

## Goal

Reduce the god-file pressure in `src-tauri/src/chat/commands.rs` by extracting low-risk, self-contained Chat helper logic into focused backend modules. This is an incremental step toward making Chat model metadata, attachment handling, conversation management, and generation orchestration independently testable and less conflict-prone.

## What I Already Know

- `src-tauri/src/chat/commands.rs` is about 5250 lines and mixes Tauri command wrappers, conversation CRUD, attachments, model metadata, context estimation/compression, agent orchestration, event emission, and tests.
- The active pain point called out by the user is that model database helpers, `chat_max_output_tokens_for_model`, conversation CRUD, and agent entry points are all in one file.
- Existing PRD `.trellis/tasks/06-07-use-model-database-max-output-for-chat/prd.md` explicitly notes that backend model database helpers currently live in `src-tauri/src/chat/commands.rs`.
- The repo already has better domain modules under `src-tauri/src/chat/agent/**`, `src-tauri/src/chat/model/**`, and `src-tauri/src/chat/storage.rs`.
- The lowest-risk extraction target is model metadata, because it is mostly pure functions over `ModelProvider`, `ModelInfo`, and the embedded `src/data/modelDatabase.json`.
- After model metadata, the next low-risk extraction target is attachment handling, because the helpers are clustered and already have focused tests.

## Requirements

- Add a focused `src-tauri/src/chat/model_metadata.rs` module.
- Move model metadata helpers out of `commands.rs` without changing behavior:
  - provider model override resolution,
  - built-in model database lookup,
  - context window resolution,
  - max output token resolution,
  - vision/image-generation capability resolution,
  - direct image generation capability decision.
- Keep existing function semantics:
  - provider overrides beat built-in database values,
  - built-in database values beat heuristics/fallbacks,
  - unknown/custom models retain current heuristic/fallback behavior,
  - direct image generation still requires the existing image generation route check.
- Move or preserve focused Rust tests for the extracted metadata behavior.
- Add a focused `src-tauri/src/chat/attachments.rs` module.
- Move attachment helper logic out of `commands.rs` without changing behavior:
  - pasted image/file temp-save helpers,
  - attachment path resolution and previews,
  - attachment MIME/type/name sanitization,
  - per-message attachment persistence,
  - prompt text composition for attachment hints,
  - stored image path collection for vision requests.
- Move or preserve focused Rust tests for attachment behavior.
- Keep `commands.rs` command API and frontend behavior unchanged.

## Acceptance Criteria

- [x] `commands.rs` no longer owns the model database helper implementation.
- [x] Chat generation still resolves `deepseek-v4-flash` max output from the built-in model database.
- [x] Provider `model_overrides` still take precedence over built-in metadata.
- [x] Context window and capability tests still pass.
- [x] `commands.rs` no longer owns attachment helper implementation.
- [x] Existing attachment preview, paste-save, and prompt composition behavior is unchanged.
- [x] Targeted Rust tests pass.

## Out Of Scope

- Splitting conversation CRUD into a new store/service module.
- Moving context compression/estimation.
- Reworking `complete_assistant_reply` or the agent runtime entry path.
- Changing Settings UI or persisted settings.

## Technical Notes

- Initial files inspected:
  - `src-tauri/src/chat/commands.rs`
  - `src-tauri/src/chat/mod.rs`
  - `src-tauri/src/chat/storage.rs`
  - `src-tauri/src/chat/agent/loop_.rs`
  - `src-tauri/src/chat/model/README.md`
  - `.trellis/tasks/06-07-use-model-database-max-output-for-chat/prd.md`
- This task is intentionally incremental. A larger follow-up can split attachments, context management, event host/executor, and conversation service boundaries.

## Definition Of Done

- Rust code compiles for the changed backend module.
- Relevant tests are moved/updated and pass.
- No unrelated dirty files are modified or reverted.
