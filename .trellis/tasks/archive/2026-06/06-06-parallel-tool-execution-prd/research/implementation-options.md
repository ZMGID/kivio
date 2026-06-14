# Implementation Options

## Option A: Conservative Batch Scheduler

Add a scheduler helper in `src-tauri/src/chat/agent/loop_.rs` or a new `scheduler.rs`.

Behavior:

- Classify calls into parallel-eligible or serial-only.
- Execute contiguous eligible groups concurrently.
- Execute serial-only calls one at a time.
- Store results by original index.
- Append transcript messages in original order.

Pros:

- Smallest change to the existing runtime.
- Keeps `SkillRunCache` unchanged.
- Easy rollback by classifying everything serial.
- Lowest risk for approval, mutation, and shared state.

Cons:

- Does not parallelize arbitrary MCP or skill tools initially.
- Requires careful tests around ordering and event emission.

Recommendation: choose this for MVP.

This mirrors the OpenAI Agents SDK split between allowing the model to emit multiple tool calls and controlling local tool execution concurrency. Kivio can keep provider behavior stable while introducing a local execution limit for eligible calls.

## Option B: Fully Shared Cache and Parallel Everything Safe-by-Policy

Change `ToolExecutor::call` to accept a shared cache such as `Arc<tokio::sync::Mutex<SkillRunCache>>`, then allow more tool families to run concurrently.

Pros:

- More complete concurrency model.
- Skill read/activate calls could eventually overlap safely.

Cons:

- Larger trait and call-site change.
- Locking around cache may serialize meaningful parts anyway.
- Harder to reason about skill scripts and shared resources.
- Higher regression risk.

Recommendation: defer until after MVP telemetry/manual confidence.

## Option C: Adopt JavaScript AI SDK for Chat Runtime

Replace or wrap the Rust agent loop with Vercel AI SDK semantics.

Pros:

- Rich established agent/tool abstractions.
- Built-in step callbacks and tool execution patterns.

Cons:

- Kivio backend is Rust/Tauri; moving core chat runtime to JS would cross process/language boundaries.
- Existing native tools, approval, cancellation, MCP dispatch, and settings live in Rust.
- Packaging and desktop runtime constraints make this much larger than the feature requires.

Recommendation: do not pursue for this task.

## MVP Eligibility Policy

Parallel-eligible:

- `native:web_search`
- `native:web_fetch`
- `native:read_file`

Serial-only:

- `native:write_file`
- `native:edit_file`
- `native:run_command`
- `native:run_python`
- `native:memory_modify`
- `skill:*`
- `mixer:*`
- unknown tool calls
- invalid tool arguments
- external MCP tools

Open for implementation judgment:

- `native:memory_read` can be parallel if the memory store is read-only and thread-safe; otherwise keep serial in MVP.

Suggested default concurrency limit:

- Start with 4 concurrent eligible calls per agent round.
- Do not expose a UI setting in the MVP unless manual testing shows a need.
- Keep an internal constant or hidden config so the behavior can be tuned without reshaping the scheduler.

## Suggested Rust Shape

Introduce:

```rust
struct PlannedToolCall<'a> {
    index: usize,
    call: PendingToolCall,
    tool: Option<&'a ChatToolDefinition>,
    mode: ToolExecutionMode,
}

enum ToolExecutionMode {
    ParallelEligible,
    SerialOnly,
}
```

The helper should return ordered outputs:

```rust
struct ToolRoundOutput {
    response_messages: Vec<Value>,
    records: Vec<ToolCallRecord>,
}
```

For parallel execution, avoid borrowing `&mut SkillRunCache`. Only schedule calls whose execution does not need the mutable skill cache. For the current trait, this may require either:

- a separate executor path for non-skill calls, or
- a small trait refactor where skill cache is optional and only borrowed for serial skill calls.

## Test Strategy

Unit-level:

- fake host records lifecycle events.
- fake executor delays by tool name.
- assert two eligible calls both enter running state before either completes.
- assert result messages are ordered by original call index.
- assert serial-only calls do not overlap.
- assert invalid and unknown calls stay ordered and emit expected records.

Integration-level:

- one provider fixture returning two tool calls in a single planning response.
- one tool delayed longer than the other.
- final assistant synthesis receives both tool results in original order.

Manual smoke:

- Ask for two independent web lookups in one prompt.
- Confirm both tool cards show running at the same time.
- Cancel while tools are running.
- Confirm final answer remains coherent and transcript replay still works.
