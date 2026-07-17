# Model temperature configuration — Implementation Plan

## 1. Shared metadata contract

- [x] Add optional `temperature` and `omitTemperature` fields to TypeScript `ModelInfo` in `src/api/tauri.ts`.
- [x] Add matching serde-defaulted fields to Rust `ModelInfo` in `src-tauri/src/settings.rs`.
- [x] Extend the frontend database entry type and conversion/merge logic in `src/data/modelMatching.ts`.
- [x] Keep every existing `modelDatabase.json` entry temperature-free.

## 2. Backend resolution

- [x] Add database temperature parsing and `temperature_for_model` precedence logic in `src-tauri/src/chat/model_metadata.rs`.
- [x] Reject non-finite or out-of-range persisted values defensively.
- [x] Add unit tests for override value, omit tombstone, missing value, and invalid value.

## 3. Model details UI

- [x] Add localized Temperature labels, blank-means-omit help text, and validation text in `src/components/ModelDetailDrawer.tsx`.
- [x] Add a `0..=2`, step `0.1` numeric input.
- [x] Map numeric edits to `temperature`; map clearing to `omitTemperature`.
- [x] Disable Save for invalid values while preserving existing dirty/reset behavior.
- [x] Extend frontend model matching tests for numeric and explicit-omit resolution.

## 4. Provider request serialization

- [x] Change `GenerateOptions.temperature` to `Option<f64>` with default `None`.
- [x] Update OpenAI Chat request construction to conditionally serialize the resolved value.
- [x] Update OpenAI Responses and Anthropic Messages request construction the same way.
- [x] Update Gemini `generationConfig` to conditionally serialize the resolved value.
- [x] Compile-check Anthropic Messages, OpenAI Responses, compaction, sub-agent, vision, and TUI callers after the type change.
- [x] Add request-body tests proving default omission, configured inclusion, and exact decimal serialization.

## 5. Lens raw request paths

- [x] Replace every hardcoded temperature in translator, OCR, vision, selected-text, and local-OCR raw OpenAI-compatible bodies.
- [x] Reuse one flat-body helper for conditional model resolution across those paths.
- [x] Verify request debug/usage labels and unrelated Lens behavior remain unchanged.

## 6. Validation

- [x] Run focused frontend tests for `modelMatching` and any drawer tests added.
- [x] Run focused Rust tests for `chat::model_metadata`, OpenAI Chat, Gemini, and settings serde.
- [x] Run frontend type-check/lint required by project specs.
- [x] Run Rust formatting/check and the relevant test suite required by project specs.
- [x] Re-run the Zen `kimi-k3` request shape through a local/request-body test to confirm no `temperature` appears by default; avoid live provider calls unless needed after deterministic tests.
- [x] Inspect git diff for unrelated user changes and preserve `website/DEPLOY.md` and `website/deploy.sh`.

## Risk and Rollback Points

- The main behavior change is global omission instead of `0.7`; verify no test or special feature relies on the old default.
- The explicit-omit tombstone must not leak into provider request bodies; it is settings metadata only.
- If UI tri-state behavior is confusing or fragile, rollback to numeric-only optional metadata before task completion rather than shipping an inability to clear future database defaults.
