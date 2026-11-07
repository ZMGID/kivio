# SDK API and lifecycle contracts

This reference describes the SDK v2 working API (not yet published). [The README](../README.md) provides commands and a runnable integration.

## Public interfaces

`agent::run_agent_loop` combines a `ModelRuntime`, `ToolExecutor`, `AgentHost`, `SessionPolicy`, `AgentEventSink`, and `AgentRunHandle`. `agent::RuntimeMessage` carries typed model history and opaque host source metadata. The `model` module provides normalized model types and standalone Chat, Responses, Anthropic and Gemini providers. The `tools` module provides tool definitions, argument schemas, and tool output.

Rustdoc lists the exported items. Other public modules contain lower-level stream, protocol, and loop helpers. Their visibility does not provide a stable compatibility guarantee during this alpha. Private and `pub(crate)` modules are implementation details. Consumers use the crate dependency and public imports, rather than compiling SDK source files directly.

The application owns desktop IPC, settings, OAuth, credential lookup and rotation, billing, skills, Todo, OMP sub-agents, session storage, and UI approval. Those capabilities do not appear automatically in another host.

The existing hosts keep their admission and persistence owners while sharing the core:

| Host | Admission, execution and completion |
|---|---|
| Desktop chat | `commands/reply.rs` reserves its run, calls `session_policy::run_session`, saves the assistant through the repository, then finishes the application protocol. `agent_host.rs` saves partial drafts and admits persisted incoming input. |
| Automation | `automation/application.rs` starts its generation, calls the same adapter, ends that generation, then converts the result into node output. Workflow storage and capacity remain outside the SDK. |
| OMP | `sub_agent/executor.rs` owns worker/job identity, metered requests, mailbox and session file; it calls the same adapter and saves history before final job/IRC delivery. Core inactivity does not release a worker slot. |

The adapter saves the supplied partial draft before rendering a structured core failure into the application's existing error path. The SDK retains the machine-readable cause; product retry and error presentation stay in the application.

## Owned AgentSession

`AgentSessionConfig::new(Arc<dyn ModelRuntime>, model)` supplies an in-memory host, empty registry, and `NoSessionPolicy`. The session owns its history and each run's identity, queues, subscriptions, and cancellation handle. Desktop, automation, and OMP keep their existing lower-level owners shown above; they do not create a second session store or capacity manager.

`prompt` accepts a nonempty user message, appends it on successful admission, and starts work immediately. `continue_run` accepts history ending in a user message or a complete tool round. Empty history, an assistant tail, and incomplete or mismatched tool rounds are rejected. Neither operation re-executes historical tool results. An active session rejects another run with `Admission(Busy)` before changing history.

Session states are `Idle`, `Running`, `StopRequested`, and `Finishing`. `abort` only requests stop. `wait_inactive` captures the current generation when called and waits for core cleanup, `AgentHost::commit_run`, and release of session admission. A later run cannot prolong that wait. Keep the Tokio runtime alive through cleanup; runtime shutdown and process termination cannot guarantee asynchronous draining or persistence.

`SessionRun` is the awaitable result. Dropping a pending waiter requests cancellation; the owned worker still drains managed tools and commits the result. Dropping the session also requests stop. `commit_run` defaults to no persistence. Its failure returns `Failed { cause, partial }` with a failed status and preserves the completed in-memory history. Core `RunEnded` describes the loop, not successful storage. Hosts must return commit errors instead of hiding them.

`history()` returns the admitted input plus the last completed core snapshot; it does not expose each streaming mutation. `steer` and `follow_up` target the current running generation. Repeated IDs return `Duplicate` across both queues in that generation. After settlement, `unconsumed_inputs` and `take_unconsumed_inputs` expose inputs that were accepted but not consumed, including inputs held in the core queue when a hook fails. These inputs are never silently submitted to the next run.

`subscribe` returns a guard; dropping it unregisters the subscriber. `Closed` or a panicking subscriber detaches that subscriber without cancelling execution. Callbacks run outside the session-state lock, so reading `history()` inside one is allowed. Delivery remains synchronous and ordered with no unbounded queue.

## Execution hooks and limits

Session configuration owns the policy and registry used by a run. `with_policy` configures request and turn hooks; `set_policy` only succeeds while idle. Tool hooks use the existing registry executor. Advanced hosts pass the same interfaces directly to the loop. There is no second plugin dispatcher.

| Hook | Result and ordering |
| --- | --- |
| `SessionPolicy::prepare_request` | Mutates only the next `ModelCall`, after input admission and before projection. Model/provider changes use explicitly supplied capabilities or unknown capabilities, never the previous model's assumptions. |
| `SessionPolicy::transform_context` | Returns the complete temporary request history. It does not mutate canonical history or commit a compaction. |
| `ToolExecutor::before_tool_call` | Trusted host boundary for normalization, policy, final validation and authorization. `Ready` permits execution; `Complete` records a refusal or other final result without executing. |
| `ToolExecutor::after_tool_call` | `ToolResultUpdate` replaces only supplied model-facing text or the entire follow-up list. `None` retains the original field. Raw records, structured execution output, errors and artifacts remain facts. |
| `SessionPolicy::finish_turn` | Runs after each completed assistant/tool turn. `Default` follows normal scheduling, `Continue(messages)` appends messages and schedules another turn, and `Stop` ends the run. It cannot revive cancellation or bypass limits. |

The order is input admission, request preparation, context transformation, existing `request_from_history` conversion, provider generation, tool before/call/after, then turn decision. Conversion gathers system text and excludes opaque source metadata; native provider blocks still obey the effective provider/model replay rules. No separate `convertToLlm` hook duplicates this boundary.

Hooks are asynchronous and receive the current cancellation handle. Cancellation discards late modifications and prevents new tool execution. Returned hook errors become `AgentFailure::Hook`, retain paired history and completed effects, and use normal cleanup. After-hook failure does not rerun a tool. Different tools retain the existing parallel execution rules; one tool's before/call/after phases remain ordered. Managed tool cleanup is awaited even after cancellation or a deadline.

`SessionPolicy::start` and `input` retain workflow injection; `prepare_context` retains explicit reset/compaction and checkpoint coordination; recovery remains with its existing owner. The application moves transient image pruning into request projection. Todo reminders remain canonical messages admitted through the existing `before_step` lifecycle operation, preserving cadence across runs and queued follow-ups. Todo final checks use `finish_turn`. `ContextUpdate::SendView` remains an explicit history reset, distinct from `transform_context`'s temporary view. `before_step` remains the synchronous canonical-input admission operation; it is not interchangeable with the temporary `prepare_request` view. The old `before_finish` method is removed. Executors migrate `prepare` to `before_tool_call` and `after` to `after_tool_call` with explicit result fields.

`RunLimits` optionally sets `max_model_steps: NonZeroU32` and `timeout: Duration`. Both are unlimited by default to preserve existing desktop behavior. Step limits cover loop-owned planning, synthesis, and recovery model requests; a custom policy must bound any independent I/O it performs. Exhaustion returns `AgentFailure::LimitExceeded` and partial history. The deadline requests cancellation and awaits managed cleanup, so elapsed wall time can exceed the deadline while cleanup finishes. It is not a durable execution checkpoint or an automatic retry policy.

## Tool registration and authorization

`AgentRunConfig.tools` advertises model-visible definitions. Each `ToolDefinition` supplies a schema and execution metadata. The core matches calls to registered definitions and reports unknown or blocked tools. A schema describes valid arguments. It does not grant permission to execute them.

`ToolExecutor::before_tool_call` is the host's authorization and validation boundary. The host selects an authorization policy explicitly, parses untrusted model arguments, and validates them before returning `ToolPreparation::Ready`. The echo example allows exactly its registered echo tool and calls `tools::schema::validate_arguments` before execution. It has no desktop approval dialog and no filesystem, shell, or network tools.

An executor can await host approval in `before_tool_call`. The SDK event observer does not grant that approval. Missing or closed event observers cannot approve tools. When `before_tool_call` returns `ToolPreparation::Complete`, the core publishes the final record exactly once. Executors must not publish that terminal update themselves. A rejected call produces feedback for the model without calling the tool implementation.

Tool authors own validation that depends on real resources, such as permitted paths and target state. Timeouts and cancellation do not undo effects already performed by a tool.

## Identity, admission, and input consumption

The host supplies a conversation ID, run ID, result message ID, and generation. Generation isolates a process-local run from later runs. Durable history revisions belong to the host's session store. Each new `AgentRunHandle` also receives a process-local invocation ID. Clones retain that invocation ID and share its completion state.

A handle starts at most one core invocation. Reusing a running or finished handle returns an error. A fresh invocation uses a fresh handle and a control bound to that invocation. The example allocates a fresh mailbox for continuation. A host that reuses its own mailbox must isolate generations and clear unconsumed input during cancellation and cleanup.

`steer` and `follow_up` return `InputAdmission::Accepted`, `Inactive`, `Unsupported`, or `Duplicate`. Accepted means the host queued the input. It does not mean the core consumed or persisted it. Input admitted near completion may remain unconsumed. Finished handles reject new input and stop requests.

Steer is consumed at step boundaries after prior tool work and before the next model step. Follow-up is consumed at an answer boundary after the current answer. These boundaries also apply when no tools are registered or the provider rejects tools. Both inject one message per applicable boundary. The host-supplied message ID survives in `ContentKind::Input`. A consumed input appears once in the ordered content blocks and replay history. The constructor `SteeringMessage::new` trims text, rejects empty input, and retains at most 4,000 Unicode characters.

The SDK does not reserve application-wide run capacity. The host prevents conflicting concurrent runs against its session store. `AgentRunHandle::active()` becoming false does not authorize a new run before the host finishes persistence and cleanup.

## Event order and completion

`AgentEventSink::on_event` is a synchronous, serialized callback. There is no internal event backlog. Slow callbacks delay the producer. Callbacks must return and must not re-enter the same invocation's event producer. Returning `EventDelivery::Closed` detaches that observer. It does not cancel the run, bypass approval, or stop host lifecycle hooks.

Within one invocation, sequence numbers increase strictly. `RunStarted` precedes step, model message, and tool facts. Step and message events have matching end events. Tool events pair by round and call ID. Concurrent tool work can interleave facts, so consumers use these identities instead of assuming adjacent pairs.

When an adapter retries a visible stream, `AttemptDiscarded` identifies text, reasoning, blocks, and tools withdrawn from the failed attempt. A consumer that renders tentative output must apply that withdrawal before rendering the next attempt. The standalone HTTP provider has no SDK-level retry loop. Its supplied HTTP client owns transport retries and redirects.

`RunEnded` is emitted once after paired inner scopes end. No later facts reach the sink. Its outcome is `Completed`, `Recovered`, `CompactionBlocked`, `Cancelled`, `Failed(AgentFailure)`, or `Dropped`. The core returned status and terminal event use the same completion reason. An outer session commit can fail afterward and changes the session result to a host failure; it does not rewrite the already emitted core event.

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

## Skill resources

The `skills` feature adds explicit-source discovery and `SkillTool` without new dependencies. Source order determines duplicate priority. Discovery stops at a skill directory instead of recursively indexing its resources. Initial discovery can return a partial catalog with `scan_failed`; `reload` rejects a failed scan before replacing the current catalog. Hosts keep ownership of when to scan and publish the next snapshot.

`SkillRunContext` holds a frozen `Arc<SkillRegistry>` and access policy. Skill identity, metadata and location remain fixed until the host selects a new catalog. Activation reads and parses the current body, so edits become visible on the next explicit load. Missing, invalid or unreadable files return errors. Optional arguments use the existing single-pass substitution and never re-interpret text introduced by argument values.

`SkillTool` implements `StaticTool` and registers through the existing `ToolRegistry` with explicit `ToolAuthorization`. Disabled IDs and an optional whitelist are checked before body loading. `disable_model_invocation` controls catalog visibility; it is not a separate permission grant or a replacement for host authorization. Recommended tools and `allowed-tools` metadata do not register or authorize tools. The default reader only reads Markdown; activation never runs skill scripts.

`SkillRecord::resolve_resource` resolves an existing path within the skill directory and rejects parent traversal, absolute paths and symlink escapes. It does not read the resource or grant filesystem access. Hosts continue to authorize actual file and process operations. Custom readers are host-provided code and remain the host's responsibility.

The application shares the parser, loader, formatting and typed activation tool. It retains source enumeration, settings, assistant policy, plugin transformations and `ResourceSessions` publication of skills, roles and hooks together. A normal send or retry does not rescan an existing catalog. The existing `skill` tool name and `<skill_content>` wrapper remain compatible with context compaction and subsequent reload instructions.

## Errors and retries

Provider calls return `ModelError` with `kind`, an optional HTTP `status`, an optional `provider_code`, an optional `retry_after` duration, and a diagnostic `message`. Kinds distinguish authentication, rate limits, timeouts, transport errors, invalid requests, unsupported tools, context overflow, moderation, cancellation, and other failures. Retry classification is information for the runtime owner. It is not an automatic retry promise.

`run_agent_loop` returns `Result<AgentRunResult, AgentRunError>`. `Admission` means no run lifecycle started. `Failed { cause, partial }` retains completed tool work, history, segments, images and reported usage. The caller owns saving that partial result. `AgentFailure::Model` preserves `ModelError` and the failure stage; `Host` identifies the failed host operation; `InvalidOutput` identifies invalid final output. A successful Rust `Result` can still have status `Cancelled`, `Failed(AgentFailure)`, or `CompactionBlocked`. `Recovered` identifies a policy-assisted recovery. Consumers inspect the status as well as the outer result. Tool failures appear in tool records and feedback, so a model can finish after a failed tool.

The standalone Chat, Responses and Gemini providers call the supplied client's `send()` once per provider invocation. They perform no SDK-level retry, key rotation, or cache fallback. The client can still retry or follow redirects. Reqwest's default retry policy includes safe protocol failures; a host that requires retries to be disabled sets `ClientBuilder::retry(reqwest::retry::never())`. Redirect policy is configured separately. The agent loop can issue further requests for tool rounds, queued input, empty-response handling, or policy-directed recovery. Hosts own the combined retry budget and output withdrawal.

## Credentials and transport

`OpenAiChatConfig.endpoint` is the complete Chat Completions URL. The SDK does not append `/v1/chat/completions`. `api_key` supplies optional bearer authentication, and `headers` supplies host-resolved headers. The caller owns conflicting authentication headers and the request body's extra options. The SDK does not read environment variables, application settings, or credential files.

`OpenAiResponsesConfig` uses the same explicit transport fields and a complete Responses URL. Its `options` selects OpenAI or xAI wire format, encrypted reasoning replay, cache fields, resolved temperature and default extra body fields. Video input is rejected before sending; unsupported content is not silently substituted. Both providers apply host defaults before encoded call options, then explicit `provider_options`, with tool prohibition last.

Responses JSON summaries and encrypted replay items are separate outputs and both survive decoding. A complete JSON response may omit `status`, as in the [official Response type](https://github.com/openai/openai-node/blob/master/src/resources/responses/responses.ts); the SDK retains that compatibility. Streamed responses still require their terminal event, including when SSE is returned to buffered generation.

`AnthropicMessagesConfig` takes a complete Messages URL and the same explicit client, headers, credential and buffered timeout inputs. It uses `x-api-key` authentication. `AnthropicMessagesOptions` selects resolved thinking mode, default token limit, cache TTL and compatibility fields without a model catalogue. Video input is rejected before HTTP. Tool prohibition remains the final request override.

Anthropic's public generation completes at most eight native `pause_turn` continuations after the initial request. Each HTTP request sends once without an SDK transport retry. It replays completed native assistant blocks, rejects unresolved client tools at that boundary, and suppresses intermediate finish events. `usage` describes the last request; `billing_usage` sums the continuation requests. Hosts must not use the cumulative bill as a context-size anchor.

Anthropic native output records its model and optional opaque `provider_id`. Replay excludes blocks from a different model or an explicitly different provider identity. Legacy blocks without provenance remain readable. Hosts that need provider isolation supply a stable identity; the SDK does not derive it from credentials. Application adapters share the encoder, decoders and bounded continuation helper while retaining authentication, tool-name policy, retry budgets and accounting.

`GeminiConfig.base_url` is the native API root. The provider appends the requested model and `generateContent` or `streamGenerateContent` method, and authenticates with `x-goog-api-key`. Options explicitly select numeric thinking budget or the call's thinking level, image output, tool IDs and response envelope format. The host resolves model profiles. The encoder and decoders are shared with the desktop adapter; Antigravity endpoints, request wrapping, OAuth and capability policy remain in the application.

Gemini tool signatures use `ProviderSignature`, which keeps opaque data and its origin together. A different model or explicitly different provider omits the signature while preserving the tool call and result. Typed history and the legacy storage bridge retain scoped signatures; old strings deserialize as `Legacy`. Chat wire requests omit this metadata. Rust callers constructing a legacy signature use `Some(value.into())`; callers reading its opaque value use `signature.value()`.

Gemini's standalone provider validates the serialized body against the existing 20,000,000-byte inline-video request budget before sending. The desktop shares that check on its final body, including any wrapper. Filesystem reads, attachment restoration and model capability decisions remain host-owned.

The host supplies `reqwest::Client` and selects proxy behavior, TLS, connection limits, and stream timeouts. `OpenAiChatConfig.request_timeout` applies only to buffered generation. The agent runtime uses streaming generation, including collected delivery, so it relies on the client configuration for its timeout.

Events can contain model context, tool arguments, and output. Provider diagnostics can include remote response text. The host selects what to retain or redact before logs, telemetry, or UI display.

## Registry and generation options

`ToolRegistry` owns ordered definitions and their executors. `register_static` derives the existing `StaticTool` schema and requires explicit `ToolAuthorization`; `register_dynamic` adapts a host executor without moving its approval or preparation rules. Duplicate keys and ambiguous names are rejected. Take a registry snapshot before constructing a run and use that snapshot both for `definitions()` and execution. Later registrations affect only later snapshots.

Static registration uses a 60-second execution timeout, no output truncation and ordinary cancellation. Dynamic preparation owns those settings when different behavior is needed. All typed follow-up messages follow all tool results in a round, including across parallel batches. Tool errors and artifacts remain in typed history. Cancellation observed after preparation prevents execution, including managed tools that have not started.

`AgentRunConfig.options` carries the complete `GenerateOptions`. Planning and plain requests retain the caller's tool choice. Final synthesis and recovery prohibit tools. Provider defaults are applied before call options and explicit `provider_options`; an effective `ToolChoice::None` is enforced last and removes native tools too. Host-resolved `ModelCapabilities` distinguish unknown (`None`) from explicitly unsupported (`Some(false)`). No model-name catalogue is embedded in the SDK. Vendor extensions not recognized by capability checks remain the host's responsibility.

An absent thinking level retains the existing automatic/default behavior, including models without a reasoning parameter. An explicit unsupported reasoning level is rejected. Context-window and additional modality configuration remain with the existing context policy until their extraction uses them; T1 does not add unused capability fields.

## Entry and projection design contract

The following defines T5's history migration; it does not claim the tree API is implemented in T1. An entry has a stable session-local ID, a parent ID and a payload. Message entries reuse `RuntimeMessage`, including tool calls and results; compaction entries record a summary and a covered ancestor range. `active_leaf` selects a path independently of the most recently appended entry. Parent links define topology; a second mandatory `branch_id` is unnecessary.

Projection selects the active parent chain, applies valid compaction ranges, then calls `request_from_history`. It preserves complete tool rounds and provider-native replay blocks. `RuntimeSource` survives export but never enters a model request. UI metadata does not become model text. An unknown model payload is an import error, not silently discarded UI metadata. Imported history never executes tools.

| Input facts and active position | Expected model projection |
|---|---|
| `s(system) -> u(user) -> a(assistant)`, leaf `a` | System `s`, messages `[u, a]` |
| `u -> a(call c) -> t(result c)`, leaf `t` | `[u, a, t]`; exactly one result for `c`; navigation cannot split the round |
| `u1 -> a1 -> u2 -> a2 -> compact(summary, covers u1..a1, retain u2)`, leaf `compact` | Summary plus `[u2, a2]`; original entries remain available |
| `u -> a1` and `u -> a2`, leaf `a1` or `a2` | `[u, a1]` or `[u, a2]`; switching alone changes and persists the selected leaf |
| `u` with source `{message_id, UI selection}`, plus a signed native assistant block | User text without source; native block retained only for its compatible provider/model |

A compaction range must be on the selected ancestor chain. Rewinding before it restores the earlier projection. Entry snapshots store history and selection, never credentials, active futures, execution cursors or tool execution capabilities. Host CAS and atomic persistence remain authoritative.
