# Project Workspace Filesystem

## Goal

Upgrade the existing Chat project system from a conversation folder concept into a local workspace concept, so a selected project can bind to a local directory and Chat tools can read, write, search, and run commands relative to that project root in a Codex-like workflow.

## What I Already Know

* The current project UI already exists in the left sidebar with "new project" and a project list. We should reuse that instead of adding a bottom project selector.
* Current `ChatProject` only stores project metadata such as id, name, description, color, and timestamps.
* Current conversations store project membership as `folder` = project name. This is enough for grouping but fragile for a real workspace because rename/duplicates/migration can break root lookup.
* Current native tools already include `read_file`, `write_file`, `edit_file`, and `run_command`.
* Current file/write boundaries are based on global native tool `workspaceRoots`; the project system does not yet drive tool execution.
* The intended product direction is closer to Codex local projects: select a project directory, then file operations and commands naturally run within that workspace.
* Codex, Claude Code, and OpenCode all center local agent work around a project/working directory boundary, with explicit handling for directories outside that boundary.
* Their permission designs separate hard filesystem/command boundaries from approval prompts. Kivio should not rely on prompt text alone for path safety.

## Requirements (Evolving)

* Reuse the existing sidebar project management entry points.
* Add local directory binding to projects, likely as `rootPath`.
* Add a durable `projectId` link on conversations while preserving `folder` for compatibility/display migration.
* Create projects from a selected folder, using the folder name as a sensible default project name.
* Support editing/rebinding a project's folder.
* When a conversation is in a selected project, file tools should resolve relative paths against that project's root.
* When a conversation is in a selected project, `run_command` should default cwd to that project's root.
* Add project-scoped filesystem tools beyond single-file reads/edits: list directory, search/glob, file stat, mkdir, delete, move/rename, copy.
* Preserve existing tool approval behavior for sensitive actions.
* Do not add the bottom project selector shown in the Codex screenshot.

## Acceptance Criteria (Evolving)

* [ ] A project can be created by choosing a local directory from the existing "new project" flow.
* [ ] Existing project list behavior continues to work for old projects without a bound directory.
* [ ] Conversations can be associated with projects by stable project id, and legacy `folder`-name conversations migrate or resolve without data loss.
* [ ] A selected project with `rootPath` causes file tools to operate relative to that directory.
* [ ] Writes, edits, deletes, moves, and copies cannot escape the project root through absolute paths, symlinks, or `..`.
* [ ] `run_command` defaults to project root cwd when a project is active.
* [ ] In project mode, explicit command cwd outside the project root is rejected.
* [ ] The model receives clear prompt/tool descriptions explaining project-relative paths.
* [ ] Lint, typecheck, and Rust tests pass where practical.

## Out of Scope (Draft)

* New bottom composer project selector.
* Full Codex permission profile UI.
* Git worktree mode.
* Commit, push, PR, and full diff management.
* Cloud project execution.

## Open Questions

* What should happen to old/name-only projects: allow them to remain chat-only, or force a folder binding when selected?
* Should the MVP include a read/list/search-only mode for no-root legacy projects, or should all project filesystem tools require a bound `rootPath`?

## Technical Notes

* Existing project storage: `src-tauri/src/chat/storage.rs`.
* Existing frontend project type/UI: `src/chat/types.ts`, `src/chat/Sidebar.tsx`, `src/chat/ProjectDialog.tsx`.
* Existing native tools: `src-tauri/src/native_tools/files.rs`, `src-tauri/src/native_tools/shell.rs`.
* Existing tool definitions/execution: `src-tauri/src/mcp/types.rs`, `src-tauri/src/mcp/registry.rs`.
* Existing approval flow: `src-tauri/src/chat/agent/execute.rs`, `src-tauri/src/chat/commands.rs`.
* Research notes: `.trellis/tasks/06-09-project-workspace-filesystem/research/agent-workspace-filesystem-patterns.md`.
* Implementation risk: current `run_command` accepts arbitrary existing `cwd`; project mode needs a project-aware resolver so shell defaults and explicit cwd cannot silently leave the active project.
* Implementation risk: path validation must canonicalize existing files and parents for non-existing files, and account for symlink escapes.
