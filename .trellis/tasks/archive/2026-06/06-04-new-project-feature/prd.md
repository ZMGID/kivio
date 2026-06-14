# Build New Project Feature

## Goal

Make the disabled "新建项目" entry in the Chat sidebar functional by introducing lightweight local projects: persistent workspaces that can exist even when empty, appear in the sidebar, contain conversations, and let users start or move chats inside a project.

This is the foundation for richer project context later, but the MVP should stay small enough to fit Kivio's local desktop-first architecture and the current chat implementation.

## What I Already Know

- The user wants the "新建项目" feature designed based on this repo and comparable software patterns.
- The current sidebar has a disabled "新建项目" nav row in `src/chat/Sidebar.tsx`.
- Conversations already have an optional `folder` field in `src-tauri/src/chat/types.rs` and `src/chat/types.ts`.
- `chat_get_conversations` already accepts an optional `folder` filter.
- `chat_create_conversation` already accepts an optional `folder`.
- The conversation context menu already includes "添加到项目", but available projects are inferred from conversations with a non-empty folder, so empty projects cannot exist.
- Comparable products treat projects/spaces as long-running workspaces with chats, files/knowledge, instructions, and sometimes sharing.

## Research References

- [`research/project-feature-research.md`](research/project-feature-research.md) - summarized ChatGPT Projects, Claude Projects, Perplexity Spaces, and the recommended Kivio MVP.
- [`research/04-chatgpt-projects-fetch.json`](research/04-chatgpt-projects-fetch.json) - fetched OpenAI help content.
- [`research/05-claude-projects-fetch.json`](research/05-claude-projects-fetch.json) - fetched Anthropic help content.
- [`research/03-perplexity-spaces-search.json`](research/03-perplexity-spaces-search.json) - fetched/discovered Perplexity Spaces content.

## Assumptions

- MVP should ship as a local single-user project system, not a collaboration feature.
- The first version should prioritize organization and workflow continuity over RAG/project knowledge.
- Existing conversations with `folder` values should continue to work.
- The UI language can remain Chinese for the current Chat surface.

## Requirements

- Enable the "新建项目" sidebar row.
- Let users create a project with at least a required name.
- Persist project records independently from conversations so empty projects are visible.
- Show created projects in the sidebar.
- Selecting a project scopes the conversation list to that project.
- Starting a new chat while a project is selected creates the chat inside that project.
- Moving an existing conversation to a project should use persisted projects as the destination list.
- Moving a conversation out of a project should continue to be supported.
- A project should support rename and delete at minimum.
- Deleting a project should not silently delete conversations unless the UI explicitly confirms that behavior. Recommended MVP behavior: delete the project record and move its conversations out of the project.
- Search should search within the currently selected scope for MVP, with global search left as the current top-level behavior unless the UX already makes a global mode obvious.
- Empty project state should make it clear the user can start a new chat in this project.

## Technical Direction

- Add a project data type in Rust, likely `ChatProject` plus `ChatProjectIndex`.
- Add storage helpers near `src-tauri/src/chat/storage.rs`, either in the same module or a new `projects.rs` module under `src-tauri/src/chat/`.
- Suggested storage path: `{app_data_dir}/chat-projects/index.json` or `{app_data_dir}/conversations/projects.json`.
- Add Tauri commands:
  - `chat_get_projects`
  - `chat_create_project`
  - `chat_update_project`
  - `chat_delete_project`
- Add TypeScript API wrappers and mock browser implementations.
- Add TS types for `ChatProject`.
- Update `Sidebar` state to load projects alongside conversations and track selected project.
- Update route state only if necessary. Minimal route can keep selected project in React state; stronger route can use `#chat/project/<project-id>` for persistence/deep-linking.
- Update `ConversationList` and `ConversationContextMenu` to receive persisted projects instead of deriving project names from conversations.

## UX Direction

- Keep creation light:
  - Modal/dialog title: "新建项目"
  - Required field: "项目名称"
  - Primary action: "创建"
  - Cancel action
- Sidebar:
  - Keep top nav "新建项目" as the creation entry.
  - Add a "项目" section showing project names.
  - Keep "聊天" for non-project or all conversations depending on selected scope. Recommended MVP: "聊天" means all conversations; selecting a project filters to that project.
- Main pane:
  - When a project is selected and has no conversations, show an empty state that invites creating the first project chat.
  - Header can show project name near model selector or in the empty state; avoid large marketing hero copy.
- Context menu:
  - Conversation menu "添加到项目" lists persisted projects.
  - Project menu should support rename and delete.

## Acceptance Criteria

- [ ] Clicking "新建项目" opens a create-project dialog.
- [ ] Creating a project persists it and displays it in the sidebar without requiring any conversations.
- [ ] Selecting a project filters the sidebar conversation list to conversations in that project.
- [ ] Starting a chat from inside a selected project creates a conversation assigned to that project.
- [ ] Existing conversations can be moved into a persisted project through the context menu.
- [ ] Conversations can be removed from a project.
- [ ] Project rename updates the sidebar and move destinations.
- [ ] Project delete is confirmed and leaves conversations accessible.
- [ ] Browser mock mode supports the same core project flows for UI preview.
- [ ] `npm run lint` passes.
- [ ] `npm run typecheck` passes.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` passes when practical.

## Out Of Scope

- Project-level RAG / file knowledge.
- Project-specific system prompt injection.
- Cross-chat memory retrieval.
- Sharing/collaboration.
- Drag-and-drop conversations into projects.
- Project icons, full color customization, nested projects.
- Bulk project operations.

## Open Questions

- Should project membership be stored as the existing human-readable `folder` name for minimal migration, or should we introduce a stable `project_id` field and migrate/display legacy `folder` values?

## Recommendation

Use the existing `folder` field as the MVP membership key to minimize backend and migration risk, while designing the new project index so a later migration to stable `project_id` is possible. This matches the current command signatures and lets the feature ship with mostly additive changes.

## Definition Of Done

- Tests/checks pass or failures are documented.
- Existing conversation flows still work outside projects.
- No unrelated dirty worktree changes are reverted or included.
- Task research and PRD remain in `.trellis/tasks/06-04-new-project-feature/`.
