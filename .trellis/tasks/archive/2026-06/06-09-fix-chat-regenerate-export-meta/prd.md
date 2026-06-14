# Fix chat regenerate metadata and sandbox export meta

## Goal

Fix two review findings from recent chat runtime changes: regenerated direct image replies should carry the correct `run_entry`, and repeated sandbox artifact exports for the same assistant message should preserve complete metadata instead of overwriting earlier entries.

## Requirements

- Direct image generation must store `run_entry: "regenerate"` when invoked from chat regenerate.
- Direct image generation must continue storing `run_entry: "send"` for normal sends.
- Sandbox export `meta.json` must represent all currently exported files for the run directory after multiple export calls.
- Metadata merging should avoid duplicating file entries and should preserve useful context fields.
- Menu bar and translator settings entry points must open the AI client embedded settings view instead of the legacy standalone settings window.
- Opening AI client settings should close any already-open legacy settings window.

## Acceptance Criteria

- [ ] Unit coverage proves direct image generation receives the selected run entry value.
- [ ] Unit coverage proves repeated sandbox exports merge metadata across calls.
- [ ] Menu bar "Settings" routes to `#chat/settings` in the AI client.
- [ ] `npm run lint` passes.
- [ ] `npm run typecheck` passes.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` passes.

## Out of Scope

- No UI redesign for usage stats or chat message metadata.
- No changes to artifact retention duration or storage root.
