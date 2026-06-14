# Normalize Kivio Chat Runtime framework

## Goal

Turn the current Chat AI implementation into a Rust-native provider-agnostic runtime. OpenAI-compatible and Anthropic/Claude should both be first-class provider backends behind a shared model contract, while existing Tauri events, conversation storage compatibility, tools, streaming, retry, and failover keep working.

## Requirements

- Add explicit model-layer contracts for messages, parts, tools, generation requests, stream parts, provider capabilities, and provider errors.
- Move OpenAI-compatible and Anthropic Messages API request/response/stream handling behind parallel provider implementations.
- Keep the current Chat UI event protocol compatible.
- Keep old provider settings and old conversation `api_messages` compatible.
- Avoid introducing a Node sidecar or Vercel AI SDK runtime dependency.

## Acceptance Criteria

- [ ] `commands.rs` calls provider-agnostic model APIs for Chat generation/streaming instead of directly branching on provider JSON formats.
- [ ] Anthropic adapter no longer depends on `chat::commands`.
- [ ] OpenAI and Anthropic provider unit tests cover text, tool calls, and stream parsing/conversion.
- [ ] TypeScript typecheck, ESLint, and Rust tests pass.

## Technical Notes

- The first implementation may retain compatibility conversion helpers at adapter boundaries.
- Runtime/storage extraction can remain incremental as long as provider concerns are separated and behavior stays compatible.
