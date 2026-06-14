# Agent Workspace Filesystem Patterns

## Scope

Compare Codex, Claude Code, and OpenCode workspace/file-operation patterns, then map them onto Kivio's current Chat project and native-tool architecture.

## Sources

* Codex manual fetched from `https://developers.openai.com/codex/codex-manual.md` on 2026-06-09.
* Claude Code official docs:
  * `https://code.claude.com/docs/en/permissions`
  * `https://code.claude.com/docs/en/settings`
  * `https://code.claude.com/docs/en/permission-modes`
  * `https://www.anthropic.com/engineering/claude-code-auto-mode`
* OpenCode official docs:
  * `https://open-code.ai/en/docs/permissions`
  * `https://dev.opencode.ai/docs/tools/`
  * `https://opencode.ai/docs/config`
  * `https://opencode.ai/docs/agents/`

## Cross-Tool Patterns

### 1. Project root is the primary trust boundary

Codex treats a desktop/app project like a session started in a specific directory. Local work is scoped to the project or a worktree, and the default recommendation is to avoid roaming outside the project root.

Claude Code starts with the launch directory as the working directory. Additional directories can be granted, but they extend file access rather than becoming full project/config roots.

OpenCode treats the directory where it was started as the working directory. Paths outside that directory trigger `external_directory` permission handling.

Kivio implication: `ChatProject.rootPath` should become the primary workspace root for project conversations. Global `workspaceRoots` should become fallback/advanced settings, not the main project model.

Source details checked on 2026-06-09:

* Codex app docs describe a project as similar to starting a Codex CLI session in a specific directory, and recommend splitting monorepos into separate projects so the sandbox only includes that project.
* Claude Code docs say the default file scope is the launch directory; `additionalDirectories` extend file access but do not become full configuration roots.
* OpenCode docs define `external_directory` for tool calls that touch paths outside the startup working directory.

### 2. File tools should be project-relative by default

Codex and OpenCode both make project-relative paths feel natural. OpenCode's `apply_patch` paths are relative to the project root. Claude Code permission examples distinguish project-relative paths from absolute paths.

Kivio implication: model-facing file tools should prefer `path: "src/App.tsx"` over absolute host paths. Tool results can display both relative and absolute/canonical paths for clarity.

Source details checked on 2026-06-09:

* Claude Code permission patterns use `/path` for project-root-relative paths, `path` or `./path` for current-directory-relative paths, `~/path` for home paths, and `//path` for filesystem absolute paths.
* OpenCode `apply_patch` embeds project-root-relative paths in patch marker lines such as `*** Update File: src/existing.ts`.

### 3. Capability categories are more useful than one-off tools

OpenCode groups read/list/glob/grep/edit/bash as permission categories. Claude Code separates Read/Edit/Bash and common filesystem commands. Codex distinguishes sandbox mode, filesystem roots, network, and approval policy.

Kivio implication: model tools can remain concrete (`list_dir`, `read_file`, `write_file`, etc.), but policy should group them:

* read/search/list/stat = read category
* write/edit/mkdir/copy/move/delete = edit category
* run_command = bash category

Source details checked on 2026-06-09:

* Claude Code has `Read`, `Edit`, `Write`, and `Bash` permission rules, plus modes such as `default`, `acceptEdits`, `plan`, `auto`, `dontAsk`, and `bypassPermissions`.
* OpenCode permissions include `read`, `edit`, `glob`, `grep`, `bash`, `external_directory`, and others. `edit` covers `edit`, `write`, and `patch`.

### 4. Approval is not the same as sandboxing

Codex explicitly treats sandbox boundaries and approval policy as two layers: what is technically allowed vs when to ask. Claude Code has permission modes and also a sandbox/auto mode story. OpenCode has allow/ask/deny rules.

Kivio implication: MVP can keep current approval prompts, but project-root enforcement must be hard backend validation. Prompt instructions alone are not enough.

Source details checked on 2026-06-09:

* Codex documentation describes sandbox mode as the technical boundary and approval policy as the question of when the agent must stop and ask.
* Claude Code docs say permission rules are enforced by Claude Code rather than by prompt instructions, and recommend sandboxing for OS-level enforcement against subprocesses.
* OpenCode config supports `allow`, `ask`, and `deny` permissions globally and per-agent.

### 5. External directories need explicit treatment

OpenCode has `external_directory` rules. Claude Code has `additionalDirectories`. Codex has workspace roots/writable roots.

Kivio implication: first MVP should deny project tool calls outside `rootPath` by default. Later we can add per-project `additionalRoots` if real workflows need multi-repo access.

### 6. Sensitive files need default guardrails

Claude Code and OpenCode both document denying `.env`/secret paths. Codex protects internal metadata directories under writable roots.

Kivio implication: MVP should at least avoid listing/reading/editing common secret files by default or require confirmation for them. Stronger default: deny `.env`, `.env.*`, secrets, keychain/ssh paths for project tools unless explicitly overridden later.

Source details checked on 2026-06-09:

* Codex permission examples use deny rules such as `**/*.env`, and default writable-root protection covers `.git`, `.agents`, and `.codex`.
* Claude Code docs note symlink checks should consider both the symlink path and resolved target; deny should win if either path matches.

### 7. Commands inherit the workspace boundary

Codex sandboxing applies to spawned commands as well as built-in file operations. OpenCode applies permissions to bash commands and external-directory touches. Claude Code gates Bash through permissions/modes.

Kivio implication: `run_command` should default cwd to project root and should not accept arbitrary cwd outside root in project mode unless a future external-root policy allows it.

Source details checked on 2026-06-09:

* Codex sandbox docs state spawned commands inherit the same sandbox boundaries as built-in file operations.
* Claude Code `acceptEdits` only auto-approves file edits and common filesystem Bash commands inside the working directory or `additionalDirectories`.
* OpenCode `external_directory` applies to path-taking tools including many `bash` commands.

## Kivio Current State

Existing:

* Sidebar project management already exists.
* `ChatProject` currently behaves as a conversation folder.
* Conversations currently store `folder` as a project name, not a durable project id.
* Native tools already include `read_file`, `write_file`, `edit_file`, and `run_command`.
* Sensitive tools already go through confirmation.
* Global native `workspaceRoots` exists.

Missing:

* `ChatProject.rootPath`.
* Durable current-project resolution in tool execution. Mapping through `Conversation.folder -> ChatProject.name` is possible but fragile after duplicate names, renames, and legacy migration.
* Project-relative path resolver shared by all file/command tools.
* Directory/list/search/glob/stat/copy/move/delete/mkdir tools.
* Project-root escape prevention for all file actions.
* Secret-file guardrails.
* UI affordance to bind/change a project folder in the existing project flow.

## Recommended Direction

Build a project-root-first MVP:

1. Extend `ChatProject` with `rootPath`.
2. Reuse the existing sidebar project flow; no bottom project selector.
3. Bind a folder when creating or editing a project.
4. Add `projectId` to conversations and resolve current conversation -> project -> rootPath at tool execution time. Keep `folder` as a compatibility/display field during migration.
5. Make file tools project-relative by default and hard-deny root escapes.
6. Make `run_command` default cwd to project root and reject outside-root cwd in project mode.
7. Keep the current approval UI for sensitive actions.
8. Treat legacy no-root projects as chat-only until the user binds a folder.

## Later Extensions

* Per-project additional roots.
* Permission profiles: read-only, workspace write, full access.
* Git metadata display: branch, dirty count, diff summary.
* Worktree mode and handoff.
* LSP/tooling integration.
