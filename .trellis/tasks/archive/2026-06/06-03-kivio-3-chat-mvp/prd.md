# Kivio 3.0 Phase 1 MVP Chat Completion

## Goal

Complete the existing AI Chat scaffold into a usable Phase 1 MVP: users can open Chat as the main app, create and load conversations, send messages with visible streaming, search and switch conversations, switch models persistently, and keep all chat data locally in `{app_data_dir}/conversations/`.

## Requirements

- Chat route `#chat` opens the chat shell, and `#chat/{conversation_id}` loads the referenced conversation.
- App launch and single-instance activation open Chat by default.
- Tray includes an "打开 AI 客户端" entry.
- macOS shows the Dock icon while Chat is open and returns to accessory mode when the main window is hidden or closed.
- Frontend consumes the existing backend stream payload shape: `{ kind, delta, reasoningDelta?, done?, reason?, full? }`.
- Streaming assistant text is rendered live in the message list.
- Stream failures are visible to users and re-enable the input.
- `ModelSelector` uses `enabledModels` first and falls back to `availableModels`.
- Changing a conversation model persists through `chat_update_conversation`.
- New conversations use Chat defaults, falling back to Lens and then translator settings.
- Sidebar refreshes after create, send, update, and delete operations.
- Selected conversation and URL stay in sync.
- Local search filters the loaded conversation list by title and preview.
- Storage validates conversation IDs before file access and handles pagination offsets beyond list length.

## Out of Scope

- Lens-to-Chat handoff.
- Image attachments and attachment upload.
- Conversation export, folders, regenerate/edit, and advanced message actions.
- SQLite or storage format migration beyond the existing JSON files and index.
- Multiple simultaneous Chat streams.

## Public Interfaces

- Keep existing Chat commands:
  - `chat_get_conversations(offset, limit, folder?)`
  - `chat_get_conversation(conversationId)`
  - `chat_create_conversation(providerId?, model?, folder?)`
  - `chat_send_message(conversationId, content, attachments[])`
  - `chat_delete_conversation(conversationId)`
  - `chat_update_conversation(conversationId, title?, pinned?, folder?, providerId?, model?)`
- Extend `chat_update_conversation` to accept `providerId` and `model`.
- Add minimal settings fields:
  - Rust `Settings.chat_provider_id`
  - Rust `Settings.chat_model`
  - frontend settings type mirrors both fields

## Acceptance Criteria

- [ ] Opening the app launches the Chat UI by default.
- [ ] Creating, selecting, and deep-linking conversations works.
- [ ] Sending a message shows live streamed assistant content and saves the final assistant reply.
- [ ] Model changes persist after reload.
- [ ] Conversation list refreshes after mutations and local search works.
- [ ] Invalid conversation IDs do not escape the conversations directory.
- [ ] Lint, type-check, Rust tests, and frontend build pass.

## Test Plan

- `npm run lint`
- `npm run typecheck`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `npm run build:ui`
- Manual smoke: launch Chat, create/send/reload/deep-link/switch model/search, then verify translator/settings/Lens routes still open.
