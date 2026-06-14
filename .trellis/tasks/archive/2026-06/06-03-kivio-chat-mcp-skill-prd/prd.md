# Complete Kivio Chat MCP + Skill PRD

## Goal

Turn the existing desktop PRD, `/Users/zmair/Desktop/Kivio Chat MCP与Skill 功能 PRD.md`, into a decision-complete implementation spec for Kivio 3.1 Chat extensibility.

This task completes the PRD/spec work only. Full feature implementation is intentionally out of scope for this task because MCP + Skill runtime touches backend agent orchestration, Tauri settings, chat persistence, frontend Chat UI, and security UX.

## What I Already Know

- Kivio is a lightweight Tauri v2 desktop app with React 18 + TypeScript frontend and Rust backend.
- Chat already exists as a separate module with local conversation JSON storage and streaming via `chat-stream`.
- Lens already has a native web-search planning pattern, but Chat does not yet support OpenAI-compatible `tools` / `tool_calls`, MCP clients, or Skill loading.
- Settings persist through Tauri Store in camelCase JSON, while chat conversation JSON currently persists snake_case Rust field names.
- MCP stdio should be owned by the Rust backend, not frontend JS shell permissions.
- Three read-only explorer agents reviewed backend, frontend, and settings/storage architecture before this PRD update.

## Locked Product Decisions

- Tool call records are persisted as metadata on the related assistant message, not as separate timeline messages.
- Skill selection is conversation-pinned and user-switchable.
- Read-only tools run automatically; sensitive tools require user confirmation.
- MVP supports MCP Tools only, not MCP Resources or Prompts.
- MVP transport is stdio; remote MCP support is deferred as Streamable HTTP.
- Native `web_search` initially reuses Lens web-search settings and is exposed through the same Tool Runtime as MCP tools.
- API keys and MCP env values follow the current local plaintext settings policy, with UI redaction and explicit warnings.

## Requirements

- Update the desktop PRD to version v1.1 with a decision-complete addendum.
- Preserve the existing PRD structure and intent while resolving open questions that were already answered during planning.
- Specify backend implementation boundaries:
  - agent loop integrates at Chat assistant completion;
  - `api.rs` needs a full-message chat-completions primitive;
  - MCP stdio lives in Rust backend modules;
  - Chat gets dedicated cancellation/run tokens separate from Lens.
- Specify data contracts:
  - settings remain camelCase and add a `chatMcp` or `chatTools` block;
  - chat persistence remains snake_case and adds defaulted optional fields;
  - stream/tool events include `conversationId` and `runId`.
- Specify frontend UX:
  - Chat/Tools settings area;
  - conversation-pinned `SkillSelector`;
  - assistant-level `ToolCallBlock`;
  - provider compatibility disable state for MCP while allowing Skill-only use.
- Specify testing expectations for Rust, frontend, backend smoke tests, and no new frontend shell capability exposure.

## Acceptance Criteria

- [x] Desktop PRD is updated to v1.1 with locked decisions and implementation-ready contracts.
- [x] Trellis task contains a PRD that future implementation work can consult without relying on chat history.
- [x] The PRD explicitly distinguishes this PRD-completion task from the later feature implementation task.
- [x] No app source code is modified as part of PRD completion.

## Out of Scope

- Implementing MCP runtime code.
- Implementing Skill loader UI/code.
- Running full app verification for a feature that has not been built yet.
- Editing unrelated active Chat MVP or Lens WIP.

## Technical Notes

- Main backend integration point: `src-tauri/src/chat/commands.rs` around `complete_assistant_reply`.
- API refactor target: `src-tauri/src/api.rs`, currently stream helpers mostly return content text and ignore `tool_calls`.
- Chat types/storage: `src-tauri/src/chat/types.rs` and `src-tauri/src/chat/storage.rs`.
- Frontend Chat shell: `src/chat/Chat.tsx`, `src/chat/InputBar.tsx`, `src/chat/MessageBubble.tsx`, `src/chat/types.ts`, `src/chat/api.ts`.
- Frontend bridge: `src/api/tauri.ts`.
- Settings persistence: `src-tauri/src/settings.rs`, `src/settings/SettingsShell.tsx`.
- Tauri capabilities should not require shell permissions if stdio MCP is fully backend-owned.
