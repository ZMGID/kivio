# Implement Chat Agent Runtime PRD

## Source PRD

Implement Phase B from [`docs/CHAT_AGENT_RUNTIME_PRD.md`](../../../docs/CHAT_AGENT_RUNTIME_PRD.md).

## Scope

- Extract the cloud chat agent orchestration loop out of `src-tauri/src/chat/commands.rs` into `src-tauri/src/chat/agent/`.
- Preserve user-visible behavior for send/regenerate, streaming, tool cards, approvals, DSML tool calls, and persisted transcript shape.
- Keep Apple local provider short-circuited in `commands.rs`.
- Add Rust characterization tests for the new agent runtime pieces where practical.

## Acceptance Criteria

- [ ] `chat/agent/` contains the runtime types, host boundary, stop/prepare/execute/stream helpers, and `run_agent_loop`.
- [ ] `commands.rs` delegates send/regenerate assistant completion through the new runtime and has no `chat/agent -> commands.rs` dependency cycle.
- [ ] Planning with no tool calls does not issue a second synthesis request.
- [ ] Step limit injects the existing max-tool-rounds system message before synthesis.
- [ ] Tool filtering and fallback behavior remain compatible with the current settings/skill/assistant preset behavior.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`, `npm run typecheck`, and `npm run lint` pass or any pre-existing failures are clearly documented.
