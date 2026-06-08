# Improve Python Sandbox CJK Font Support

## Goal

Python sandbox image generation should render Chinese text reliably by default. Users and agents should not need to download fonts, switch to English, or manually discover CJK-capable fonts when using matplotlib or Pillow.

## What I Already Know

- The current Pyodide sandbox has common Python packages, including matplotlib and pillow.
- Generated charts with Chinese labels currently show missing-glyph boxes.
- The frontend already bundles `@fontsource-variable/noto-serif-sc` for app UI text.
- `run_python` executes inside the frontend Pyodide WebView sandbox, so host OS fonts are not reliably available.

## Requirements

- Provide a CJK-capable font inside the Pyodide sandbox runtime.
- Configure matplotlib automatically so Chinese text in titles, labels, legends, and ticks uses the bundled font.
- Make Pillow code able to discover and use the bundled font without network access.
- Preserve existing Pyodide local/CDN package loading behavior.
- Keep release packaging aware that the Python/Pyodide bundle includes the font assets.

## Acceptance Criteria

- [ ] A Python snippet using matplotlib with Chinese text can save a PNG without missing-glyph boxes.
- [ ] A Python snippet using Pillow can load the exposed font path and draw Chinese text.
- [ ] The font resource is served by Vite in dev and emitted into `dist/pyodide/` for app packaging.
- [ ] Type-check and lint pass.

## Out of Scope

- Full emoji color-font rendering.
- Host OS font scanning.
- Runtime network font downloads.

## Technical Notes

- Likely files: `src/chat/pyodideRunner.ts`, `vite.config.ts`, `scripts/prepare-pyodide-assets.mjs`, `docs/RELEASE_PACKAGING.md`.
- Preferred path: reuse bundled Noto Serif SC assets if Pyodide/Pillow/matplotlib can load them.
