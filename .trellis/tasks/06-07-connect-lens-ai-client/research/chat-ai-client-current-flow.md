# Chat / AI Client Current Flow

## Scope

This note maps the current AI client runtime relevant to connecting Lens screenshot Q&A.

## Frontend Entry Points

* `src/App.tsx` routes `#chat` to `src/chat/Chat.tsx` inside `ChatWindowHost`.
* `src/chat/api.ts` wraps conversation CRUD and `chat_send_message`.
* `src/api/tauri.ts` centralizes event listeners for both Lens and Chat:
  * Chat: `chat-stream`, `chat-context`, `chat-tool`, `chat-tool-confirm`, `chat-run-python`.
  * Lens: `lens-stream`, `lens-translate-stream`, `lens-web-search`.
* `Chat.tsx#handleSendMessage` creates/reuses a conversation, posts optimistic user UI, sends content and pending attachments, and then applies the returned conversation after streaming completes.

## Backend Entry Points

* `chat_create_conversation` creates a stored `Conversation` under the Chat data domain.
* `chat_send_message` accepts `conversation_id`, `content`, `attachments: Vec<String>`, and an optional `active_skill_id`.
* `save_message_attachments` copies source file paths into the conversation attachment directory and records relative attachment paths.
* Image attachments are converted into stored image paths through `stored_image_paths_for_attachments`.
* `complete_assistant_reply` is the main AI client send path.

## Runtime Path

1. `chat_send_message` loads the conversation and copies attachments.
2. It builds user-facing message content and API-facing content hints.
3. It computes context state and may auto-compress the conversation.
4. It calls `complete_assistant_reply`.
5. `complete_assistant_reply` resolves provider/model from the conversation.
6. If the selected model is direct image generation, it routes to image generation.
7. If images are attached and the main model needs auxiliary vision, it runs the configured auxiliary vision model and records that as a tool-like record.
8. It builds system prompt, tools, skills, memory, and runtime messages.
9. It calls `chat::agent::run_agent_loop`.
10. Streaming is emitted through `chat-stream`; context/tool progress through `chat-context` and `chat-tool`.
11. The final assistant message, reasoning, tool records, and API/model messages are persisted into the conversation.

## Existing Vision/Assistant Signals

* The mock/default assistant list includes a built-in screenshot analysis assistant (`asst_builtin_screenshot_analyst`) with category `vision`.
* `Settings::effective_chat_model()` already falls back from explicit chat default to Lens provider/model and then translator provider/model. Model configuration is partially connected already.
* `Settings::effective_vision_model()` falls back to the effective chat model when no explicit vision model is configured.

## Current Strengths

* Provides persistent conversations, titles, projects, assistants, memory, tool runtime, MCP, skills, context usage, context compression, and per-conversation cancellation.
* Attachment handling already supports local image file paths.
* Agent runtime has provider-compatible replay messages and tool scheduling contracts documented in `.trellis/spec/backend/agent-runtime.md`.

## Current Limitations For Lens Integration

* The public frontend send API only accepts an existing conversation ID. A Lens-to-Chat bridge would need to create/reuse a conversation before sending.
* Chat event payloads do not map directly to Lens local placeholder state unless Lens subscribes to `chat-stream` and tracks a conversation/run.
* Chat UI owns its own route/window. Opening the Chat window and navigating to a specific created conversation needs an event or hash/navigation convention.
* Chat attachment copying expects actual file paths; Lens only exposes `imageId` to the frontend. The backend must resolve `imageId` to a safe path before handing it to Chat storage.

