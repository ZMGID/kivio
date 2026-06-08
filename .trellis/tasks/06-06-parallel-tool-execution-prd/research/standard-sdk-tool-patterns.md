# Standard SDK Tool-Calling Patterns

## Sources

- Vercel AI SDK docs: https://ai-sdk.dev/docs/ai-sdk-core/tools-and-tool-calling
- Vercel AI SDK cookbook, Call Tools in Parallel: https://ai-sdk.dev/cookbook/node/call-tools-in-parallel
- OpenAI Agents SDK docs: https://openai.github.io/openai-agents-python/
- LangChain tool-calling docs: https://python.langchain.com/docs/how_to/tool_calling/

## Vercel AI SDK

The AI SDK models an agent run as one or more steps. In a step, the model can produce tool calls; tools with `execute` functions can be run by the framework and fed back into the next step until `maxSteps` is reached.

Important conventions:

- Tool definitions include schemas and optional execution functions.
- The framework can execute multiple independent tool calls from the same generation step. Its "Call Tools in Parallel" cookbook states that some models support parallel tool calls and that this is useful when multiple tools are independent and can execute during the same generation step.
- Step callbacks such as `onStepFinish` expose tool calls and tool results.
- Tool lifecycle callbacks expose per-call start, finish, output, error, and duration.
- `maxSteps` bounds multi-step tool loops.
- Results are structured and associated with tool call ids.
- `response.messages` is the standard way to append generated assistant and tool messages back into conversation history.
- AI SDK Core supports dynamic tools, including MCP/user-defined tools where schemas are loaded at runtime.

For Kivio, the useful pattern is not the JavaScript API surface itself but the step model: one model planning step may produce a batch of tool calls, and the runtime may execute independent calls in parallel before continuing.

## OpenAI-Compatible Tool Calling

OpenAI-compatible Chat Completions represent assistant tool requests as an assistant message with a `tool_calls` array. Tool results are returned as subsequent `role: tool` messages keyed by `tool_call_id`.

Implementation-relevant convention:

- A single assistant message may contain multiple tool calls.
- The application owns execution and must provide matching tool result messages.
- Ordered, complete replay matters for strict providers.

Some OpenAI APIs expose provider options such as `parallel_tool_calls`; however, local execution still has to be concurrent for real latency wins.

## Anthropic Tool Calling

Anthropic Messages represents tool use as assistant `tool_use` content blocks and tool results as user-side `tool_result` blocks. Kivio's Anthropic adapter maps between this shape and the internal OpenAI-compatible transcript.

Implementation-relevant convention:

- Multiple `tool_use` blocks can appear in one assistant response.
- Tool results must be paired back to their `tool_use_id`.
- Provider-native replay shape can differ from OpenAI's while sharing the same internal call/result abstraction.

## OpenAI Agents SDK

OpenAI Agents SDK frames execution as an agent loop:

- model turn
- tool execution
- next model turn or final output

It also emphasizes tracing and run lifecycle events. For Kivio, the relevant idea is that tool execution is a distinct scheduler phase that can apply policy, safety, tracing, and cancellation before resuming the model loop.

OpenAI Agents SDK now explicitly separates provider-side and runtime-side parallelism:

- `ModelSettings.parallel_tool_calls` controls whether the model may emit multiple tool calls in one response.
- `ToolExecutionConfig(max_function_tool_concurrency=...)` controls how many emitted local function tools the SDK starts at once.
- With `max_function_tool_concurrency=None`, the default behavior is to start all emitted local function tool calls.

This distinction maps directly to Kivio: provider options can encourage multiple emitted calls, but Kivio's Rust runtime still needs its own scheduler to execute emitted local tools concurrently.

## LangChain

LangChain exposes model tool calls as structured call objects and commonly executes them through tool nodes/runnables. Its graph-oriented variants distinguish between model planning and tool execution nodes, making it natural to run a set of tool calls as a batch.

For Kivio, the relevant pattern is the explicit boundary between planning and execution. A scheduler at that boundary can make concurrency decisions without changing provider adapters.

## Takeaways for Kivio

- Preserve the current model-step loop; add a scheduler inside each tool execution step.
- Keep call/result ids stable.
- Emit lifecycle events per call.
- Apply safety policy before parallel execution.
- Preserve deterministic transcript order even when completion order differs.
- Keep a max step/round limit independent from per-step parallelism.
- Treat provider-side parallel tool-call permission and local execution concurrency as separate configuration layers.
