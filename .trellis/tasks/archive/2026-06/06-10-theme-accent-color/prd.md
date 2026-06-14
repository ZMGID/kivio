# Add Theme Accent Color

## Goal

Add a theme color option to Kivio's appearance settings so the app can use a warmer custom surface color in addition to the existing light/dark/system appearance mode.

## What I Already Know

* The user wants to extend the current theme functionality, which currently only supports light/dark behavior.
* The provided reference image shows a very light warm color with approximate RGB values `250, 249, 245`, which maps to `#FAF9F5`.
* Current frontend settings expose `settings.theme` as `system | light | dark`.
* `App.tsx` applies dark mode by toggling the root `.dark` class.
* `SettingsShell.tsx` renders the Appearance section and the theme segmented control.
* Backend settings store `theme` as a string in `src-tauri/src/settings.rs`, defaulting to `system`.

## Assumptions (Temporary)

* The requested theme color should be persisted in settings and applied across app windows, not only previewed in the settings page.
* The new color should coexist with light/dark/system mode instead of replacing them entirely.
* The first target color is `#FAF9F5`, derived from the reference image.

## Open Questions

* None currently blocking.

## Requirements (Evolving)

* Add settings UI for choosing a theme color.
* Use fixed preset colors, not an arbitrary custom color picker.
* Preserve the current neutral look as the default preset for existing users.
* Include the reference warm white color `#FAF9F5` as a selectable preset.
* Persist the selected theme color through the existing Tauri settings system.
* Apply the selected theme color consistently after loading settings and when settings change.
* Keep existing light/dark/system behavior working.

## Acceptance Criteria (Evolving)

* [x] Appearance settings include a clear control for theme color.
* [x] The reference color `#FAF9F5` can be selected via the warm preset.
* [x] Existing users keep the current neutral theme color unless they choose a different preset.
* [x] Theme color persists across app restart/settings reload.
* [x] Existing `system`, `light`, and `dark` theme modes continue to work.
* [x] `npm run lint`, `npm run typecheck`, and relevant Rust tests pass when practical.

## Definition of Done

* Tests added/updated where appropriate.
* Lint/typecheck pass.
* Cross-layer settings persistence is verified.
* Any new durable convention is considered for spec updates.

## Out of Scope

* Full theme marketplace or downloadable themes.
* Reworking every hard-coded component color unless needed for the selected MVP behavior.
* Changing provider/model settings behavior.

## Technical Notes

* Likely frontend files: `src/App.tsx`, `src/api/tauri.ts`, `src/settings/SettingsShell.tsx`, and CSS under `src/index.css` / `src/App.css`.
* Likely backend file: `src-tauri/src/settings.rs`.
