# Lens + AI Client Integration Options

## Option A: Send Lens Screenshot To A Chat Conversation (Recommended MVP)

Add a backend bridge command that resolves a Lens `imageId` to a local image path, creates or reuses a Chat conversation, sends the screenshot as a Chat image attachment with the user question, and opens/focuses the Chat window on that conversation.

Possible command shape:

```text
lens_send_to_chat(image_id, question, messages?, folder?) -> { conversationId }
```

Recommended MVP behavior:

* Default Lens Q&A sends the captured screenshot and question directly to the AI client.
* Open/focus the AI client and navigate to the conversation where the screenshot/question were sent.
* Follow-up conversation happens in the AI client.
* Add a setting/switch to disable this transfer mode. When disabled, Lens uses the original in-window `lens_ask` behavior.
* Backend resolves `imageId` through the existing Lens image resolver and passes the resolved file path into Chat attachment storage.
* Create the Chat conversation with normal/default Chat settings; do not select a specialized assistant for Lens handoff.
* Reuse `chat_send_message` / `complete_assistant_reply` instead of duplicating agent runtime logic.

Pros:

* Lowest risk to existing Lens overlay and translation behavior.
* Uses Chat persistence, tools, memory, assistants, context, and attachment pipeline.
* Avoids forcing Lens UI to render tool progress and context indicators immediately.
* Gives users a clear mental model: Lens is capture and dispatch; Chat is where the conversation continues.
* Keeps the old Lens quick-answer behavior available for users who prefer it.

Cons:

* Requires a navigation/event bridge to open Chat to a newly created conversation.
* If the first response is streamed in Chat while the Chat window is opening, frontend must already subscribe to `chat-stream`.
* Need a decision on whether Lens closes immediately after handoff or briefly shows a sent state.

## Option B: Run Lens Q&A Through Chat Runtime While Staying In Lens

Replace `lens_ask` internals with a Chat runtime call and adapt Chat stream/tool/context events back into Lens UI.

Pros:

* Lens immediately gains tools, memory, assistant prompts, and context handling.
* One backend runtime for visual Q&A.

Cons:

* Much bigger UI surface: Lens would need to render or suppress tool status, confirmation prompts, context warnings, Python requests, artifacts, and per-conversation cancellation.
* Requires creating hidden/ephemeral conversations or a new non-persistent agent session type.
* Risky for the fast floating overlay UX.
* Blurs product semantics: Lens history and Chat history may diverge unless migration is designed.

## Option C: Share Only Low-Level Provider/Model Helpers

Keep Lens `call_vision_api`, but extract shared provider/model resolution, image token estimation, model metadata, prompt pieces, or streaming parsers from Chat.

Pros:

* Smallest implementation footprint.
* Preserves current Lens behavior exactly.
* Good for cleanup if duplicated model/provider logic becomes painful.

Cons:

* Does not connect Lens to AI client features such as tools, memory, assistants, projects, MCP, or persistent conversations.
* Users still cannot continue a Lens screenshot naturally in Chat.

## Recommendation

Start with Option A, with transfer enabled by default and the original Lens answer mode available as a setting fallback.

The codebase already has the hard parts needed for this path:

* Lens screenshots are real local PNG paths behind `imageId`.
* Chat accepts local image file paths as attachments and persists them safely.
* The Chat runtime is already the richer AI client surface.

Do not replace `lens_ask` as the first move. It remains the fallback when transfer mode is disabled. The first integration should be a default handoff bridge, not a runtime transplant.

## Key Design Decisions Still Needed

* Whether transfer mode sends only the current screenshot/latest question or also copies any previous Lens local messages.
* Whether Lens should immediately close/focus Chat after handoff or show a short sent state.
* Whether text-only Lens mode should also support handoff to Chat without screenshot attachment.
* Exact setting label and default: likely "Send Lens questions to AI client" enabled by default.

## Likely Implementation Areas

* Backend:
  * `src-tauri/src/lens_commands.rs` for the bridge command.
  * `src-tauri/src/chat/commands.rs` for exposing a reusable internal send helper if needed.
  * `src-tauri/src/shortcuts.rs` or `src-tauri/src/windows.rs` for focusing Chat and routing to a conversation.
  * `src-tauri/src/main.rs` command registration.
* Frontend:
  * `src/api/tauri.ts` for the bridge invoke type.
  * `src/Lens.tsx` for the handoff action and UI state.
  * `src/chat/Chat.tsx` if a new `chat-open-conversation` event is needed.

## Risks

* Chat conversation creation and streaming are currently tied to Chat UI assumptions. A backend-triggered send should avoid racing with Chat frontend reloads.
* Lens image paths must stay validated through existing `resolve_explain_image_path` rules before being copied into Chat storage.
* Lens close cleanup deletes the active temp screenshot. The handoff bridge must copy the resolved Lens image to an independent temporary PNG before closing Lens or starting asynchronous Chat attachment handling.
* Do not bind the handoff conversation to a screenshot assistant unless the user explicitly asks for assistant-specific behavior.
* Do not share Lens cancellation with Chat cancellation; `AppState` intentionally separates `explain_stream_generation` from `chat_stream_generations`.
* Tool confirmation prompts are Chat-window concepts. Option A avoids exposing them in Lens.
