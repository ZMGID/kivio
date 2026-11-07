# SDK API and lifecycle contracts

This reference describes `0.1.0-alpha.1`. [The README](../README.md) provides commands and a runnable integration.

## Public interfaces

`agent::run_agent_loop` combines a `ModelRuntime`, `ToolExecutor`, `AgentHost`, `SessionPolicy`, `AgentEventSink`, and `AgentRunHandle`. `agent::RuntimeMessage` carries typed model history and opaque host source metadata. The `model` module provides normalized model types and `model::openai::OpenAiChatProvider`. The `tools` module provides tool definitions, argument schemas, and tool output.

Rustdoc lists the exported items. Other public modules contain lower-level stream, protocol, and loop helpers. Their visibility does not provide a stable compatibility guarantee during this alpha. Private and `pub(crate)` modules are implementation details. Consumers use the crate dependency and public imports, rather than compiling SDK source files directly.

The application owns desktop IPC, settings, OAuth, credential lookup and rotation, billing, skills, Todo, OMP sub-agents, session storage, and UI approval. Those capabilities do not appear automatically in another host.

## Tool registration and authorization

`AgentRunConfig.tools` advertises model-visible definitions. Each `ToolDefinition` supplies a schema and execution metadata. The core matches calls to registered definitions and reports unknown or blocked tools. A schema describes valid arguments. It does not grant permission to execute them.

`ToolExecutor::prepare` is the host's authorization and validation boundary. The host selects an authorization policy explicitly, parses untrusted model arguments, and validates them before returning `ToolPreparation::Ready`. The echo example allows exactly its registered echo tool and calls `tools::schema::validate_arguments` before execution. It has no desktop approval dialog and no filesystem, shell, or network tools.

An executor can await host approval in `prepare`. The SDK event observer does not grant that approval. Missing or closed event observers cannot approve tools. When `prepare` returns `ToolPreparation::Complete`, it publishes the final record with `ToolProgress::updated` first. A rejected call produces feedback for the model without calling the tool implementation.

Tool authors own validation that depends on real resources, such as permitted paths and target state. Timeouts and cancellation do not undo effects already performed by a tool.

## Identity, admission, and input consumption

The host supplies a conversation ID, run ID, result message ID, and generation. Generation isolates a process-local run from later runs. Durable history revisions belong to the host's session store. Each new `AgentRunHandle` also receives a process-local invocation ID. Clones retain that invocation ID and share its completion state.

A handle starts at most one core invocation. Reusing a running or finished handle returns an error. A fresh invocation uses a fresh handle and a control bound to that invocation. The example allocates a fresh mailbox for continuation. A host that reuses its own mailbox must isolate generations and clear unconsumed input during cancellation and cleanup.

`steer` and `follow_up` return `InputAdmission::Accepted`, `Inactive`, or `Unsupported`. Accepted means the host queued the input. It does not mean the core consumed or persisted it. Input admitted near completion may remain unconsumed. Finished handles reject new input and stop requests.

Steer is consumed at step boundaries after prior tool work and before the next model step. Follow-up is consumed at an answer boundary after the current answer. These boundaries also apply when no tools are registered or the provider rejects tools. Both inject one message per applicable boundary. The host-supplied message ID survives in `ContentKind::Input`. A consumed input appears once in the ordered content blocks and replay history. The constructor `SteeringMessage::new` trims text, rejects empty input, and retains at most 4,000 Unicode characters.

The SDK does not reserve application-wide run capacity. The host prevents conflicting concurrent runs against its session store. `AgentRunHandle::active()` becoming false does not authorize a new run before the host finishes persistence and cleanup.

## Event order and completion

`AgentEventSink::on_event` is a synchronous, serialized callback. There is no internal event backlog. Slow callbacks delay the producer. Callbacks must return and must not re-enter the same invocation's event producer. Returning `EventDelivery::Closed` detaches that observer. It does not cancel the run, bypass approval, or stop host lifecycle hooks.

Within one invocation, sequence numbers increase strictly. `RunStarted` precedes step, model message, and tool facts. Step and message events have matching end events. Tool events pair by round and call ID. Concurrent tool work can interleave facts, so consumers use these identities instead of assuming adjacent pairs.

When an adapter retries a visible stream, `AttemptDiscarded` identifies text, reasoning, blocks, and tools withdrawn from the failed attempt. A consumer that renders tentative output must apply that withdrawal before rendering the next attempt. The standalone HTTP provider has no SDK-level retry loop. Its supplied HTTP client owns transport retries and redirects.

`RunEnded` is emitted once after paired inner scopes end. No later facts reach the sink. Its outcome is `Completed`, `Cancelled`, `Failed(String)`, or `Dropped`. `Completed` describes core completion and also covers results such as `Recovered` and `CompactionBlocked`. The returned `AgentRunResult.status` carries that more specific status.

Core completion does not report successful outer persistence. The host awaits the loop, writes the final result, finishes resource cleanup, and only then releases its run slot or publishes its own durable completion notification.

## Stop, cancellation, and dropped futures

`AgentRunHandle::stop()` requests cooperative cancellation and returns `Requested` or `Inactive`. It does not wait for cleanup. `wait_inactive()` observes host inactivity or core completion. A stop can make it return while cleanup is still running. Awaiting `run_agent_loop` is the core cleanup boundary.

Ordinary cancelled tool calls stop accepting results and drop the in-flight tool future. `PreparedToolCall::drain_on_cancel` makes the core await that tool instead. This flag also bypasses the ordinary core timeout around `call`, so the tool must bound its own I/O and cleanup. A host that requires tool completion preserves its own outer cleanup ordering. Drained work can report actual completion in the old invocation before its terminal event. Cancellation prevents a new model step from using late output.

The standalone provider cancels an HTTP read when its generation future is dropped. Dropping the entire loop future closes paired event scopes and emits `RunEnded::Dropped` when the loop has started. Dropping does not await asynchronous tool cleanup or save a final result. Tools that spawn work outside their futures require host-owned cancellation and joining. Hosts that need drain guarantees request stop and await the loop instead of aborting it.

## History and persistence

`AgentRunConfig.runtime_messages` is an owned input snapshot. The core owns its working copy during the invocation. `AgentRunResult.history` is the canonical returned history for continuation, including applied context changes. `generated_history` contains messages generated or admitted for append during this invocation. Appending it to `history` duplicates messages. Appending it blindly to the original input can also ignore compaction or host-persisted input.

`AgentHost::checkpoint_runtime` can persist a checkpoint and return `SessionInput`. `InputPersistence::Append` includes the new message in both working history and the generated append set. `AlreadyPersisted` includes the message in working history while excluding it from that append set. The host chooses the variant according to its durable write, rather than source metadata or a later guess.

`SessionPolicy` owns optional context and product decisions. `NoSessionPolicy` enables none. Before adopting a compacted history, the core checkpoints the original history. A checkpoint error prevents that replacement. The host still owns durable commits, version checks, conflict handling, draft persistence, and attachment storage. A successful core return is not a transaction against the host store.

Continuation rebuilds model context from saved messages and completed tool results, then starts new model work under current authorization. It does not replay tool records as an execution queue. A crash between an external effect and its durable result leaves an uncertain outcome. The SDK provides no exactly-once guarantee for files, commands, or remote operations. Hosts and tools reconcile target state or use their own idempotency keys before retrying uncertain work.

## Errors and retries

Provider calls return `ModelError` with `kind`, an optional HTTP `status`, an optional `provider_code`, and a diagnostic `message`. Kinds distinguish authentication, rate limits, timeouts, transport errors, invalid requests, unsupported tools, context overflow, moderation, cancellation, and other failures. Retry classification is information for the runtime owner. It is not an automatic retry promise.

`run_agent_loop` returns `Result<AgentRunResult, String>`. Its error string is not a stable machine-readable provider error contract. A successful Rust `Result` can still have status `Cancelled`, `Failed(String)`, or `CompactionBlocked`. `Recovered` identifies a policy-assisted recovery. Consumers inspect the status as well as the outer result. Tool failures appear in tool records and feedback, so a model can finish after a failed tool.

The standalone `OpenAiChatProvider` calls the supplied client's `send()` once per provider invocation. It performs no SDK-level retry, key rotation, or cache fallback. The client can still retry or follow redirects. Reqwest's default retry policy includes safe protocol failures; a host that requires retries to be disabled sets `ClientBuilder::retry(reqwest::retry::never())`. Redirect policy is configured separately. The agent loop can issue further requests for tool rounds, queued input, empty-response handling, or policy-directed recovery. Hosts own the combined retry budget and output withdrawal.

## Credentials and transport

`OpenAiChatConfig.endpoint` is the complete Chat Completions URL. The SDK does not append `/v1/chat/completions`. `api_key` supplies optional bearer authentication, and `headers` supplies host-resolved headers. The caller owns conflicting authentication headers and the request body's extra options. The SDK does not read environment variables, application settings, or credential files.

The host supplies `reqwest::Client` and selects proxy behavior, TLS, connection limits, and stream timeouts. `OpenAiChatConfig.request_timeout` applies only to buffered generation. The agent runtime uses streaming generation, including collected delivery, so it relies on the client configuration for its timeout.

Events can contain model context, tool arguments, and output. Provider diagnostics can include remote response text. The host selects what to retain or redact before logs, telemetry, or UI display.
