# Relax Python Sandbox Package Downloads

## Goal

Make Kivio Chat's `run_python` sandbox less restrictive by allowing Pyodide-side network package installs while keeping the important safety boundary: sandboxed Python must not operate on the host filesystem or modify the host Python environment.

## What I Already Know

* `run_python` is registered by Rust as a native Chat tool, but actual execution happens in the frontend WebView through Pyodide.
* The current runner auto-loads a fixed set of Pyodide packages and blocks imports for common network/API clients before execution.
* Current prompts tell the model that `run_python` has no network access and must not use packages like `requests`, `httpx`, or `aiohttp`.
* The user's desired sandbox definition is: do not touch host files; downloading packages inside the sandbox is acceptable.

## Requirements

* Allow Python code in the Pyodide sandbox to install additional compatible packages via `micropip`.
* Keep using bundled/local Pyodide resources first for known packages, with CDN/network fallback when needed.
* Remove the broad Python import block for networking/API packages so the sandbox can attempt normal Pyodide-compatible imports and installs.
* Continue warning against host `pip`, host Python package installs, and writes to host paths.
* Update tool descriptions and agent prompts so models understand the new boundary: Pyodide sandbox may download packages, but cannot access host files.
* Preserve generated image artifact capture behavior.

## Acceptance Criteria

* [x] `run_python` no longer rejects code solely because it imports `requests`, `httpx`, `urllib3`, `aiohttp`, or `tavily`.
* [x] If an import fails for a package that is not pre-bundled, the runner attempts a sandbox-local `micropip.install(...)` before returning an import error.
* [x] Existing auto-loading for bundled packages still works.
* [x] Prompts/tool descriptions no longer describe `run_python` as fully networkless.
* [x] Prompts still forbid using `run_command`/host `pip` to work around sandbox package issues unless the user explicitly requests host environment changes.
* [x] `npm run lint`, `npm run typecheck`, and Rust tests pass when practical.

## Out of Scope

* Giving Pyodide access to the host filesystem.
* Persisting installed packages across app restarts.
* Building a package management UI.
* Guaranteeing every PyPI package works in Pyodide; packages with native wheels or unsupported browser networking may still fail.

## Technical Notes

* Main runner: `src/chat/pyodideRunner.ts`.
* Frontend bridge: `src/chat/Chat.tsx`, `src/api/tauri.ts`.
* Tool definition and prompt text: `src-tauri/src/mcp/types.rs`, `src-tauri/src/chat/agent/prepare.rs`.
* Package bundling remains handled by `scripts/prepare-pyodide-assets.mjs` and `vite.config.ts`.
