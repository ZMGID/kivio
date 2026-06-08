# Parallel Tool Execution PRD

## Goal

Improve Kivio Chat agent runtime so multiple independent tool calls emitted by one model planning step can run concurrently, matching standard AI SDK behavior while preserving Kivio's safety, approval, transcript, cancellation, and UI semantics.

## What I Already Know

- Kivio's chat agent runtime is implemented in Rust under `src-tauri/src/chat/agent/`.
- The model abstraction already supports multiple tool calls in one output through `GenerateOutput.tool_calls: Vec<PendingToolCall>`.
- OpenAI-compatible and Anthropic adapters both parse multiple provider tool calls into that shared vector.
- The current bottleneck is local execution: `run_agent_loop` iterates `for tool_call in tool_calls` and awaits `execute_tool_call` one call at a time.
- Tool results are appended back into `runtime_messages`, `generated_api_messages`, and per-step response metadata as OpenAI-compatible `role: tool` messages.
- `ToolExecutor::call` currently receives `&mut SkillRunCache`, so skill tools cannot be naively executed in parallel without changing cache ownership.
- Kivio has multiple tool families with different risk profiles: native built-ins, skill runtime tools, Mixer image generation, external MCP stdio/HTTP tools, memory tools, filesystem tools, shell command execution, and Pyodide-backed Python execution.
- Existing settings include `max_tool_rounds`, `tool_timeout_ms`, `max_tool_output_chars`, and `approval_policy`, but no parallel tool policy or concurrency limit.

## Assumptions

- The first implementation should optimize latency for independent read-only/network tools without changing the public chat UX.
- Tool results must be fed back to the model in the same order as the assistant's original tool calls, even if execution completes out of order.
- Tools with side effects, user approval, shared process state, or shared caches should remain serial until explicitly proven safe.
- The runtime should not require adopting the JavaScript AI SDK; this is a Rust-native agent loop compatibility improvement.

## Requirements

- Execute eligible tool calls from the same agent round concurrently.
- Keep provider-side multi-tool-call support and local execution concurrency conceptually separate.
- Preserve current behavior for provider planning, step limits, fallback to plain chat when providers reject tools, streaming text buffering, and final synthesis.
- Preserve transcript compatibility:
  - assistant message with all requested `tool_calls` must be recorded before tool results.
  - generated tool result messages must correspond one-to-one with each original `tool_call_id`.
  - persisted `api_messages` / `model_messages` replay must remain provider-compatible.
- Preserve UI event semantics:
  - every tool emits pending/running/final records.
  - records may appear running concurrently.
  - final persisted `tool_calls` should be deterministic.
- Preserve cancellation:
  - cancelling the generation should cancel or stop awaiting all active tool work.
  - cancelled tools should emit cancelled records where possible.
- Preserve approval policy:
  - tools that require approval must not start execution before approval.
  - first implementation may serialize approval-gated calls to avoid simultaneous approval prompts.
- Preserve timeout policy per tool, including special cases for `skill_run_script`, `run_command`, `run_python`, and `mixer_generate_image`.
- Add a conservative eligibility policy:
  - parallel by default only for explicitly safe tool categories.
  - serial by default for mutation, command execution, skill cache, memory write, Python sandbox, image generation, and unknown MCP tools unless classified safe.
- Use an MVP concurrency cap for eligible tools, recommended default: 4 concurrent tool executions per round.
- Add tests that prove:
  - independent eligible tools start before each other completes.
  - result messages are appended in model call order.
  - serial-only tools keep old behavior.
  - unknown/invalid tool calls preserve existing error handling.
  - cancellation and timeout behavior do not regress.

## Non-Goals

- Do not replace the Rust runtime with Vercel AI SDK.
- Do not change the chat UI layout in this task unless required for concurrent running states.
- Do not enable parallel mutation tools by default.
- Do not add speculative model planning or run multiple agent rounds in parallel.
- Do not change user-facing assistant behavior beyond faster multi-tool responses.

## Acceptance Criteria

- [ ] In one model planning round, two or more safe independent tool calls can run concurrently.
- [ ] Persisted replay messages keep the same ordering and shape expected by OpenAI-compatible and Anthropic provider adapters.
- [ ] Unsafe/side-effecting tools remain serial.
- [ ] Tool records show concurrent running states without losing final success/error/skipped/cancelled outcomes.
- [ ] Existing unit tests pass.
- [ ] New unit tests cover parallel scheduling, serial fallback, ordering, and invalid/unknown calls.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` passes.

## Research References

- [`research/current-agent-runtime.md`](research/current-agent-runtime.md) - Kivio runtime analysis and current serial execution point.
- [`research/standard-sdk-tool-patterns.md`](research/standard-sdk-tool-patterns.md) - Tool-calling patterns from AI SDK and adjacent agent frameworks.
- [`research/implementation-options.md`](research/implementation-options.md) - Candidate approaches mapped onto Kivio constraints.

## Recommended MVP

Implement a Rust-native per-round tool scheduler:

1. Convert the current inline per-call loop into a helper that plans a batch of tool executions.
2. Classify each call as parallel-eligible or serial-only.
3. Execute contiguous groups:
   - eligible read-only/network calls concurrently with a small limit, initially 4.
   - serial-only calls one by one in original order.
4. Collect outputs by original call index and append tool messages in original order.
5. Keep `SkillRunCache` mutable and serial for all `skill_*` tools in the MVP.

## Open Questions

- Should external MCP tools be serial by default, or should HTTP MCP tools be parallel when marked non-sensitive?

Recommended answer: serial by default for arbitrary MCP tools in the first implementation; later add explicit per-tool/server parallel policy after collecting real usage.

## Technical Notes

- Main execution loop: `src-tauri/src/chat/agent/loop_.rs`.
- Single-tool execution helper: `src-tauri/src/chat/agent/execute.rs`.
- Agent host abstraction and approval/cancellation hooks: `src-tauri/src/chat/agent/host.rs`.
- Tool executor trait currently uses `&mut SkillRunCache`, constraining naive concurrency.
- Model tool call abstraction: `src-tauri/src/chat/model/types.rs`.
- OpenAI adapter: `src-tauri/src/chat/model/openai.rs`.
- Anthropic adapter: `src-tauri/src/chat/model/anthropic.rs`.
- MCP/native/skill/mixer dispatch: `src-tauri/src/mcp/registry.rs`.

## Definition of Done

- Tests added/updated where appropriate.
- Lint/typecheck/test commands run where practical.
- Behavior documented in code or task notes.
- Rollback path is straightforward: scheduler can be disabled or eligibility can be set to all-serial.
