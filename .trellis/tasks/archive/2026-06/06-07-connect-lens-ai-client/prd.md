# brainstorm: connect lens with ai client

## Goal

Connect Lens Q&A with the newer AI client/chat runtime so Lens can capture a screenshot and question, send them directly into the AI client, open/focus the AI client conversation, and let follow-up discussion continue there. Preserve the original in-Lens answer behavior behind a setting.

## What I Already Know

* The user wants to first research how Lens and AI client features should connect.
* Existing Lens supports screenshot capture, screenshot translation, AI vision Q&A, follow-up questions, streaming, and local history.
* Existing AI client/chat work is active in the repo and likely has separate runtime, message, model, MCP, tool, and storage abstractions.
* Lens and Chat share the same React app bundle but render different routes/views: `#lens` and `#chat`.
* Lens currently uses a lightweight `imageId + ExplainMessage[] + lens-stream` runtime.
* Chat currently uses persistent `Conversation` records, attachments, `chat-stream`, context/tool events, and the Rust `chat::agent::run_agent_loop`.
* Chat already accepts local image paths as attachments, while Lens screenshots are stored as local PNG paths behind `imageId`.
* Product decision: the first connected feature should make Lens send its message automatically to the AI client by default, then jump to the AI client conversation with the screenshot visible there.
* The original Lens in-window Q&A behavior must remain available through a switch; default is transfer to AI client, switch off means keep old Lens behavior.

## Assumptions (Temporary)

* "AI client" refers to the newer chat/client runtime under `src/chat` and `src-tauri/src/chat`.
* Screenshot translation remains separate unless explicitly changed later.
* When transferring to the AI client, the Lens overlay should not also answer the same question in-place.

## Open Questions

* None for MVP.

## Requirements (Evolving)

* Map the current Lens frontend/backend flow.
* Map the current AI client/chat frontend/backend flow.
* Identify overlapping concepts and mismatches: message shape, image attachments, model/provider selection, streaming events, cancellation, storage/history, and tool/runtime support.
* Propose feasible integration approaches with trade-offs.
* Default Lens Q&A mode sends the screenshot and question to the AI client as a Chat message.
* After transfer, open/focus the AI client and navigate to the target conversation.
* In the AI client, the user message must show the captured screenshot/image attachment and the question.
* The handoff must not select or invoke a specialized assistant; it should send into normal Chat with the current/default Chat model and behavior.
* Follow-up conversation happens in the AI client, not in the Lens overlay, when transfer mode is enabled.
* Add a setting/switch to disable transfer mode and restore the original Lens in-window Q&A behavior.
* Preserve Lens screenshot translation and text translation behavior.

## Acceptance Criteria (Evolving)

* [x] Research note documents Lens data flow and extension points.
* [x] Research note documents AI client/chat runtime data flow and extension points.
* [x] Research note proposes 2-3 integration approaches and recommends an MVP path.
* [x] MVP behavior is chosen: default transfer to AI client; setting off restores original Lens Q&A.
* [x] Remaining UX behavior is clarified: close Lens immediately and focus the AI client.

## Definition of Done (Team Quality Bar)

* Tests added/updated if implementation follows.
* Lint / typecheck / CI green if implementation follows.
* Docs/notes updated if behavior changes.
* Rollout/rollback considered if risky.

## Out of Scope (Explicit)

* No release packaging changes.
* No provider/API key migration unless research shows it is required for the Lens/client bridge.
* Do not remove the existing Lens Q&A runtime; it remains the fallback behavior behind the switch.

## Technical Notes

* Initial research task: `.trellis/tasks/06-07-connect-lens-ai-client/`.

## Research References

* [`research/lens-current-flow.md`](research/lens-current-flow.md) — Lens is a standalone quick overlay runtime using local state, `imageId`, and Lens-specific stream/history events.
* [`research/chat-ai-client-current-flow.md`](research/chat-ai-client-current-flow.md) — Chat is the richer persistent AI client runtime with conversations, attachments, tools, memory, and `run_agent_loop`.
* [`research/integration-options.md`](research/integration-options.md) — Recommended MVP is a Lens-to-Chat handoff bridge instead of replacing `lens_ask`.

## Research Notes

* Recommended MVP: when the setting is enabled by default, Lens `doSend` resolves the current screenshot into a Chat image attachment, creates or reuses a Chat conversation, sends the user question, and opens Chat to that conversation.
* Fallback behavior: when the setting is disabled, Lens keeps using the existing `lens_ask` path and renders the answer inside the floating Lens overlay.
* Defer replacing Lens UI with Chat runtime rendering. The transfer mode lets Chat own tool progress, confirmations, context warnings, and conversation persistence.
* Transfer UX: after the Lens question is handed off successfully, close Lens immediately and focus/navigate the AI client conversation.
