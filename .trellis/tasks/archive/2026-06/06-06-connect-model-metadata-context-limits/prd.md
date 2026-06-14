# Connect model metadata to context limits

## Goal

Make the chat context usage panel use the configured model metadata instead of falling back to name-based guesses. A model such as `deepseek-v4-flash` should display its configured/model-database context window around 1M tokens, not the hard-coded 128K heuristic.

## What I already know

* The frontend model database lives in `src/data/modelDatabase.json` and matching logic in `src/data/modelMatching.ts`.
* Model detail settings can store `contextWindow`, `maxOutput`, capabilities, and pricing as `ModelInfo`.
* The chat context panel renders `ConversationContextState.context_window_tokens` from the backend.
* The backend currently computes context window in `src-tauri/src/chat/commands.rs` via `context_window_for_model(model)`, which only guesses from the model name and returns 128K estimated for `deepseek`, `qwen`, `gemini`, etc.

## Requirements

* When a conversation has provider/model metadata with `contextWindow`, use it for `ConversationContextState.context_window_tokens`.
* Preserve existing name-based heuristics as a fallback when no explicit metadata exists.
* Mark `context_window_estimated` as false when the value came from explicit model metadata.
* Keep the change scoped to context-window plumbing; do not redesign model database UX.

## Acceptance Criteria

* [ ] A model configured with `contextWindow: 1048576` reports `context_window_tokens: 1048576` in context stats.
* [ ] Models without explicit context metadata continue to use existing heuristics/fallbacks.
* [ ] Rust tests cover explicit metadata overriding the DeepSeek 128K heuristic.
* [ ] TypeScript/Rust checks pass where practical.

## Definition of Done

* Lint/typecheck/Cargo tests run as practical.
* No unrelated user changes reverted.

## Out of Scope

* Changing the model database contents.
* Changing pricing/capability behavior beyond existing settings storage.
* Adding a new frontend test runner.

## Technical Notes

* Likely edit: `src-tauri/src/chat/commands.rs` to pass `settings`/provider model info into context window resolution.
* Relevant specs: frontend index plus cross-layer guide because this is a data-flow issue across settings/backend/frontend.

