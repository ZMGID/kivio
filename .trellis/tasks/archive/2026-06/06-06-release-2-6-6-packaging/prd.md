# Release 2.6.6 Packaging

## Goal

Prepare and publish Kivio v2.6.6 with packaging fixes for document/Python workflows.

## Requirements

* Bump app versions to `2.6.6`.
* Keep GitHub Actions from rebuilding macOS Apple Silicon for this release; build Apple Silicon locally and upload that asset.
* Ensure Pyodide/Python sandbox common packages are bundled in installers per `docs/RELEASE_PACKAGING.md`.
* Relax `run_python` file input reading so Windows users can analyze PDF/Word/document files without unnecessary safe-copy/temp path failures.
* Preserve write/edit restrictions.
* Build and verify the local macOS Apple Silicon installer, then push release changes/tag and upload the local DMG.

## Acceptance Criteria

* [ ] Version metadata is `2.6.6`.
* [ ] Release workflow matrix omits macOS Apple Silicon and still builds Windows plus macOS Intel.
* [ ] `run_python` accepts readable local file paths and mounts them into Pyodide.
* [ ] Pyodide package bundle contains required runtime and common wheels.
* [ ] Lint, typecheck, Rust tests, and build pass.
* [ ] Final macOS artifact resources include document Skills and Pyodide runtime/package files.

## Technical Notes

* Required packaging flow: `docs/RELEASE_PACKAGING.md`.
* Pyodide cache script: `scripts/prepare-pyodide-assets.mjs`.
* Runtime loader: `src/chat/pyodideRunner.ts`.
* Backend file bridge: `src-tauri/src/mcp/registry.rs`.
