# Technical Design

## Release Shape

- Version: `2.8.0`; Git tag: `v2.8.0`.
- Base changelog tag: `v2.7.9`.
- Stable release, not prerelease.
- Windows: GitHub Actions `windows-latest` builds NSIS.
- macOS: local Apple Silicon build produces unsigned aarch64 DMG.
- Website source version is updated in Git, but remote website deployment is not executed.

## Version Contract

The following values must agree:

- `package.json` and both root entries in `package-lock.json`
- `src-tauri/Cargo.toml` and the root `kivio` package in `src-tauri/Cargo.lock`
- `src-tauri/tauri.conf.json` top-level version and Windows WiX version
- `.github/workflows/release.yml` dispatch example/default
- English and Chinese README release headings
- `website/index.html` version labels
- `docs/releases/v2.8.0.md` title, asset names, and compare link

## Release Highlights

1. Model request controls: optional temperature, model metadata refresh, and safer provider request serialization.
2. Chat scrolling: stable bottom behavior and no replayed entrance animation when virtualized messages remount.
3. OCR memory: RapidOCR/inpainting model memory is released after use.
4. External Agents: Pi shutdown no longer produces EPIPE; OpenCode custom models are discovered from global/project config.
5. Reliability: full frontend and Rust suites pass, with regressions for each issue class.

## Ordering and Safety

1. Create and commit release metadata on the current release branch.
2. Run all quality checks and build the macOS DMG locally.
3. Inspect the mounted DMG and required resources before any Tag is pushed.
4. Fast-forward local `main` to the release branch and push `main`.
5. Create and push `v2.8.0`; this triggers the Windows workflow and creates the GitHub Release.
6. Watch Windows CI to completion, upload the already-verified DMG, replace boilerplate notes, and verify both assets.

## Secret Boundary

- Never stage `website/DEPLOY.md` or `website/deploy.sh`.
- Do not print, copy, or store the embedded credential in release notes, task artifacts, commits, or commands.
- Stage release files explicitly; never use `git add -A`.

## Rollback

- Before Tag push: amend/fix the release commit and rebuild locally.
- After Tag push but before user downloads: delete/recreate the GitHub Release and Tag only if the workflow/artifact is irreparably wrong.
- Prefer corrective asset upload or release-note edit over moving a published Tag.
