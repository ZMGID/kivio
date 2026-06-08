# Fix Chat Tools Indicator and Pyodide Packaging

## Goal

Fix two concrete integration gaps in Kivio Chat: the UI must accurately reflect all enabled tool categories, and release builds must include the local Pyodide runtime required by `run_python` and document Skills.

## What I Already Know

* Chat currently supports MCP tools, native tools, Skill runtime tools, and Pyodide-backed `run_python`.
* The Chat tool indicator only treats `nativeTools.webSearch` as native tool enablement, so `readFile`, `writeFile`, `editFile`, `runCommand`, `runPython`, and `webFetch` can be enabled without the UI noticing.
* `pyodideRunner.ts` first tries to load Pyodide from local app resources under `/pyodide/`, then falls back to CDN.
* Tauri currently bundles `resources/skills` but does not explicitly bundle local Pyodide runtime files.

## Requirements

* Chat tool status should consider every native tool switch that can register a backend tool.
* Skill runtime remains its own category because it controls `skill_activate`, `skill_read_file`, and `skill_run_script`.
* The existing `chat_mcp_list_tools` command remains the source of truth for the actual enabled tool count.
* Frontend build must copy the required Pyodide runtime files into the built frontend so `local app resources` resolves in production.
* Release packaging must include the built Pyodide assets in the app bundle.
* CDN loading may remain as fallback, but packaged resources should be the normal first path.

## Acceptance Criteria

* [ ] Enabling only `runPython`, `readFile`, `writeFile`, `editFile`, `runCommand`, or `webFetch` makes Chat request/list tools instead of showing no tools.
* [ ] Existing web search, MCP, and Skill behavior remains unchanged.
* [ ] `npm run build:ui` produces `dist/pyodide/pyodide-lock.json`, `python_stdlib.zip`, and core Pyodide runtime files.
* [ ] Tauri bundle config includes `dist/pyodide` as a bundled resource.
* [ ] `npm run lint`, `npm run typecheck`, and Rust tests pass when practical.

## Out of Scope

* Changing native tool security policy.
* Downloading or vendoring extra third-party wheels beyond what the installed Pyodide npm package already provides.
* Reworking the Chat tool execution model or Settings layout.

## Technical Notes

* Likely frontend files: `src/chat/Chat.tsx`, `src/chat/pyodideRunner.ts`.
* Likely build files: `package.json`, `scripts/*`, `src-tauri/tauri.conf.json`.
* Backend command registration was already aligned in the architecture review.
