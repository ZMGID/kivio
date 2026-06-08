# Current Agent Runtime Analysis

## Summary

Kivio's current chat agent runtime already supports receiving multiple tool calls from one model response, but executes them serially in the local Rust agent loop. The architecture is close to standard multi-step tool-calling runtimes, but lacks a per-step concurrent tool scheduler.

## Relevant Flow

1. `chat_send_message` / regeneration command prepares conversation context and tool definitions.
2. `run_agent_loop` performs up to `settings.chat_tools.max_tool_rounds` planning rounds.
3. Each planning round sends runtime messages and available tools to a provider.
4. The provider returns an assistant message that may include multiple tool calls.
5. `extract_tool_calls` converts native provider tool calls or DSML fallback markup into `Vec<PendingToolCall>`.
6. The assistant tool-call message is appended to the runtime transcript.
7. Current code iterates the vector and awaits each `execute_tool_call` before moving to the next.
8. Tool result messages are appended and the next planning round or final synthesis runs.

## Serial Execution Point

`src-tauri/src/chat/agent/loop_.rs` currently does:

```rust
for tool_call in tool_calls {
    ...
    let (record, tool_content) = execute_tool_call(...).await;
    ...
}
```

This means a model response containing independent calls such as `web_search(A)` and `web_search(B)` cannot overlap; total latency is roughly the sum of tool latencies.

## Model Layer Readiness

`src-tauri/src/chat/model/types.rs` defines:

- `PendingToolCall`
- `GenerateOutput { tool_calls: Vec<PendingToolCall>, ... }`
- `MessagePart::ToolCall`
- `MessagePart::ToolResult`

This is already compatible with multiple calls per model step.

`openai.rs` and `anthropic.rs` both gather tool calls into vectors. Anthropic replay maps assistant tool calls to `tool_use` content blocks and tool results to `tool_result` content blocks. OpenAI replay maps them to assistant `tool_calls` and `role: tool` messages.

## Execution Constraints

- `execute_tool_call` owns per-call lifecycle emission: pending, approval, running, timeout/cancel, final.
- It receives `&mut SkillRunCache`; this prevents naive parallel borrowing.
- Approval is asynchronous through `AgentHost::request_tool_approval`.
- Cancellation is checked with `wait_for_generation_inactive`.
- Timeout is calculated per tool and includes special cases.
- Result truncation and artifact handling happen inside `execute_tool_call`.

## Tool Families

Likely safe to parallelize in MVP:

- `native:web_search`
- `native:web_fetch`
- `native:read_file`

Require serial handling in MVP:

- `native:write_file`
- `native:edit_file`
- `native:run_command`
- `native:run_python`
- `native:memory_modify`
- all `skill:*`
- `mixer:mixer_generate_image`
- arbitrary MCP tools unless explicitly classified safe

Potentially parallel but should be reviewed:

- `native:memory_read`
- external HTTP MCP tools
- external stdio MCP tools

## Data Ordering Requirement

Even if tools execute concurrently, result messages should be appended in the original model-requested order. Provider replay and model reasoning tend to assume the transcript pairs assistant tool calls with deterministic tool result order.

## Implementation Implication

The cleanest local change is to move the per-call loop into a scheduler helper that returns ordered `(ToolCallRecord, tool_message)` outputs for a whole planning round. The main loop can then append them exactly as before.
