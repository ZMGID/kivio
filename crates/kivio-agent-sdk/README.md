# Kivio Agent SDK

`kivio-agent-sdk` runs a model and tool loop in a Rust host without Tauri or the Kivio application. The desktop application uses this same crate through a path dependency.

Version `0.1.0-alpha.1` is a source-consumed prerelease under [GPL-3.0-or-later](LICENSE). SDK v2 names the design iteration, not a published `2.0` release. This repository does not publish the crate as part of the migration or change its license. Public APIs can change between prereleases; pin a repository commit and keep your lockfile. External distribution requires a separate license-suitability decision by the project owner. No minimum supported Rust version is promised; local verification used Rust 1.96.0 on Windows.

## Run the examples without credentials

Run these commands from the repository root with a current stable Rust toolchain.

```powershell
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example session_offline
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example injections_offline
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- minimal
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- cancel
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- inputs
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- continue
```

[The offline session example](examples/session_offline.rs) combines a custom model runtime, typed and explicitly authorized echo tool, request hook, host commits, forks and JSON snapshot restoration. Assertions verify the actual tool output, retained branches and exactly one tool execution across restoration. Its host keeps commits in memory; a durable host must atomically save before acknowledging a commit.

The `agent` example starts a bounded loopback HTTP server and calls the production `OpenAiChatProvider`. Neither offline example reads model credentials or application settings. Cargo may download locked dependencies on the first build.

The modes have these observable results.

| Mode | Result |
| --- | --- |
| `minimal` | Executes the explicitly authorized `echo` tool once, prints stream event facts, and returns `Completed`. |
| `cancel` | Calls `stop()` on the first nonempty text delta after the tool round, awaits the loop, and verifies `Cancelled`. |
| `inputs` | Submits steer and follow-up after the tool finishes, then verifies that each input is consumed once. |
| `continue` | Starts a fresh invocation with the returned canonical history and a new user message. It verifies that the old tool is not executed again. |

The output includes the invocation ID, increasing event sequence numbers, tool status, and the final status. The local answers are deterministic. The example checks its results with assertions and exits unsuccessfully if a check fails.

[The runnable entry point](examples/agent.rs) shows how the public API fits together. [The host adapter](examples/support/mod.rs) owns the per-run mailbox and authorizes only a harmless echo tool. [The local HTTP server](examples/support/http.rs) supplies responses for the example. The independent consumer fixture has additional protocol and failure tests.

## Integrate a Rust host

Use a path dependency from your own project. Adjust the SDK path to your checkout.

| Cargo feature | Scope |
| --- | --- |
| Default (empty) | Session, hooks, tools, four providers, context management and snapshots |
| `skills` | Filesystem discovery and read-only activation; no additional feature dependencies |
| `mcp` | rmcp transports and process dependencies for stdio and Streamable HTTP tools |

```toml
[dependencies]
kivio-agent-sdk = { path = "../kivio/crates/kivio-agent-sdk", version = "=0.1.0-alpha.1" }
reqwest = { version = "0.13", features = ["json", "rustls", "stream"] }
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
```

For a normal multi-turn host, use `AgentSession`. It owns history, input queues, subscriptions, and one active invocation:

```rust,no_run
use std::sync::Arc;
use kivio_agent_sdk::agent::{AgentSession, AgentSessionConfig, provider_runtime::ModelRuntime};

async fn chat(runtime: Arc<dyn ModelRuntime>) -> Result<(), Box<dyn std::error::Error>> {
    let session = AgentSession::new(AgentSessionConfig::new(runtime, "your-model"));
    let first = session.prompt("Explain agent sessions briefly.").await?;
    let next = session.prompt("Give one example.").await?;
    println!("{}\n{}", first.content, next.content);
    Ok(())
}
```

The [session example](examples/session.rs) constructs the provider, sends a prompt, round-trips a snapshot through JSON, and continues in a restored session. Run it with the environment variables below using `cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example session`. No empty executor, host, or mailbox implementation is required. Set `config.tools` to a `ToolRegistry` to add tools. Set `config.host` only when the host needs checkpoints or a final `commit_run` operation.

`prompt` admits immediately and returns a waiter; concurrent sends return `Admission(Busy)`. `abort()` requests cancellation. `wait_inactive().await` waits for the current invocation's tool cleanup and commit. Dropping the waiter requests stop while the owned worker finishes cleanup. Keep the Tokio runtime alive until this wait completes. `continue_run()` continues an unanswered user message or completed tool round; it never executes saved tool results. See [session and hook contracts](docs/contracts.md#owned-agent-session) for queue, subscription, and failure semantics.

Advanced hosts with existing admission and persistence owners can compose the lower-level interfaces as the original example does:

1. Construct `OpenAiChatProvider` with your HTTP client and an `OpenAiChatConfig` containing the complete Chat Completions endpoint URL.
2. Implement `AgentRunControl` for one invocation's input mailbox and stop signal. Pass its `Arc` to `AgentRunHandle::new`, and clone the handle for controls that target the same invocation.
3. Implement `ToolExecutor::before_tool_call` to authorize the registered tool and validate its arguments. Implement `call(self: Arc<Self>, invocation: ToolInvocation)` with an owned future. The SDK constructs and polls it in a separate task. Await `invocation.progress.updated(&record)` when reporting progress.
4. Set `AgentRunConfig.history` to a `SessionHistory`, plus model, tools, and host-supplied identity. Use matching identity values when constructing the handle.
5. Call `run_agent_loop(config, &host, executor.clone(), &mut NoSessionPolicy, &events, handle).await` with an `Arc<dyn ToolExecutor>`.
6. Check both the returned `Result` and `AgentRunResult.status`. Persist `AgentRunResult.history` in your own session store before releasing your application's run capacity.

Start with `NoSessionPolicy` and an empty `AgentHost` implementation, as the example does. These enable no product policies or persistence. Add checkpoints, context management, or recovery only when your host needs them. A host with no tools still supplies a `ToolExecutor` implementation, but sets `tools` to an empty vector.

The SDK does not register desktop tools or load permissions from Kivio settings. Registering a `ToolDefinition` advertises a model-visible tool. Authorization remains an explicit operation in your executor. See [the API and lifecycle contracts](docs/contracts.md) before adding tools with external effects.

For automatic context management, use `context::BudgetContextPolicy` with a resolved summary runtime and `SummaryOptions`. Set its main-request model, window and output budget explicitly when they differ from the summary model. Install it with `AgentSessionConfig::with_policy`. Original facts remain available through `session.entries()` while `session.context()` returns the current projection. See [context contracts](docs/contracts.md#context-budgets-and-compaction) and the [independent consumer tests](../../tests/fixtures/agent-sdk-consumer/tests/context.rs) for repeated compaction, cancellation and Skill reload examples.

## Navigate session history

Use `active_leaf()` to remember a position and `leaves()` to list retained branch tips. `fork(entry_id)` selects an inclusive anchor, and the next prompt creates another path. `switch_branch(Some(leaf))` restores that path without executing old tools. `rewind(user_entry)` returns the original input and selects the position before it.

`regenerate(entry_id, None).await` produces a new answer while retaining the old path. Supply a replacement user `RuntimeMessage` to edit a question on a new path. Navigation requires an idle session, including completion of tool cleanup and host commit. See [tree contracts](docs/contracts.md#session-trees-and-navigation) and the [consumer tests](../../tests/fixtures/agent-sdk-consumer/tests/tree.rs) for request-level examples.

## Save and restore a session

Call `session.snapshot()` after the run settles and serialize the returned `SessionSnapshot` with serde. The host owns saving these bytes atomically. Deserialization validates the version, complete entry tree and selected path before returning a usable snapshot. IDs are decimal strings to preserve the full `u64` range across JSON consumers.

`AgentSessionConfig::from_snapshot(runtime, snapshot)` restores saved model identity, standard generation settings and history. Reinstall tools, authorization, hooks, host persistence, run limits and vendor options before starting another run. Provider clients and credentials are supplied through the new runtime; pending queues and active execution are not restored. Message bodies and source metadata remain intact, so hosts still control sensitive transcript storage.

Advanced hosts can use `history.snapshot()` without session configuration, then `snapshot.into_history()` with their own settings. `graft_branch` merges all descendants below a concurrent run's shared anchor, including inactive paths. It reuses shared facts, remaps new entry references and returns their ID mapping without changing the destination selection. See [snapshot contracts](docs/contracts.md#session-snapshots) and [consumer tests](../../tests/fixtures/agent-sdk-consumer/tests/snapshot.rs).

## Run against a real provider

The standalone providers currently support these protocols. Desktop adapters share their request encoders and response decoders.

| Protocol | Public config and provider | Endpoint |
| --- | --- | --- |
| Chat Completions | `model::openai::{OpenAiChatConfig, OpenAiChatProvider}` | Complete `/chat/completions` URL |
| Responses, including xAI format | `model::responses::{OpenAiResponsesConfig, OpenAiResponsesProvider}` | Complete `/responses` URL |
| Anthropic Messages | `model::anthropic::{AnthropicMessagesConfig, AnthropicMessagesProvider}` | Complete `/messages` URL |
| Gemini Generate Content | `model::gemini::{GeminiConfig, GeminiProvider}` | Native API root, for example `/v1beta`; the requested model and method are appended |

Construct a provider with your HTTP client and config. Each implements `LanguageModelProvider` for direct generation and `ModelRuntime` for `AgentSession`. Responses accepts buffered JSON or SSE-only responses, preserves encrypted reasoning for same-model replay, and supports function tools and provider-side search. Use `ResponsesFormat::Xai` explicitly for xAI wire behavior. The host resolves cache keys, credentials and compatibility options; the standalone providers do not learn endpoint failures or rotate keys.

Anthropic preserves signed thinking, redacted thinking and native server-tool blocks. Its explicit options select adaptive or budget-based thinking, cache TTL and an opaque provider identity for replay isolation. Native `pause_turn` responses continue up to eight times; final context usage remains separate from cumulative billing usage. Claude Code identity and model-profile selection remain host responsibilities.

Gemini supports native function calls, images, inline video, grounding and thought signatures. Select `GeminiThinkingMode::Level` to encode the call's thinking level or `Budget(tokens)` for a host-resolved numeric budget. The default requests thoughts without choosing a level or budget. Model profiles and Antigravity authentication remain in the host. New tool signatures carry their model and optional provider identity; old string signatures remain readable.

Set environment variables in the example process. `KIVIO_SDK_BASE_URL` is the **full endpoint**, such as `https://api.example.com/v1/chat/completions`, rather than a base directory. `KIVIO_SDK_MODEL` selects your provider's model. `KIVIO_SDK_API_KEY` is optional for endpoints without bearer authentication. Supply the key through your process environment or secret manager, and omit it from command arguments and committed files.

```powershell
$env:KIVIO_SDK_BASE_URL = 'https://api.example.com/v1/chat/completions'
$env:KIVIO_SDK_MODEL = 'your-tool-capable-model'
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- minimal --live
```

You can also append `--live` to `cancel`, `inputs`, or `continue`. These modes make model requests and depend on the provider's tool behavior. The loop, rather than a fixed request script, decides how many model steps are needed. The example sets a 60-second HTTP client timeout and retains the client's default retry policy. The SDK adds no retry loop. Hosts that need a stricter transport policy configure their client as described in [the retry contract](docs/contracts.md#errors-and-retries).

Only the example reads these environment variables. The SDK receives the resolved endpoint, credentials, headers, and HTTP client. The live example keeps reqwest's client defaults for proxy behavior. Configure `.proxy(...)`, `.no_proxy()`, TLS, and timeouts on your client as needed. The local example explicitly disables proxies for its loopback requests.

## Load skills

Enable the `skills` Cargo feature to use filesystem discovery and the read-only Skill tool. The host supplies ordered `SkillSource` directories and source labels. The SDK does not search home directories, install plugins or change tool permissions.

```rust
use std::sync::Arc;
use kivio_agent_sdk::skills::{SkillAccess, SkillRegistry, SkillRunContext, SkillSource, SkillTool};

let sources = [SkillSource { path: "./skills".into(), source: "project".into() }];
let mut catalog = SkillRegistry::discover(&sources);
let context = SkillRunContext::new(Arc::new(catalog.clone()), SkillAccess::default());
tools.register_static(SkillTool::definition(), SkillTool::new(context), authorization)?;
catalog.reload(&sources)?;
```

Here `tools` is the same `ToolRegistry` used by `AgentSessionConfig`, and `authorization` is the host's `ToolAuthorization`. Discovery returns metadata and diagnostics through `scan_failed` and `warnings`. A failed reload leaves the catalog unchanged. Existing tools retain their frozen catalog; build the next tool snapshot from the reloaded catalog and install it with `session.set_tools(tools)` while idle.

Each activation reads the selected file again, applies optional arguments and returns the full instruction wrapper. `SkillAccess` can deny IDs or restrict activation to a whitelist; `Some(vec![])` allows none. `disable_model_invocation` retains the existing catalog-visibility behavior. Execution authority comes from access policy and host authorization. Resource paths can be resolved relative to a skill directory without reading or executing those resources or granting access to other tools.

Hosts with adapted Markdown sources can supply a discovery reader and `SkillBodyReader`. Kivio uses these seams for plugin metadata, body variables and live plugin availability while retaining its existing resource cache and approval flow.

For a complete executable setup, use [the independent Skill consumer](../../tests/fixtures/agent-sdk-consumer/tests/skills.rs): `cargo test --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml --test skills`.

## Connect MCP tools

Enable the `mcp` Cargo feature for stdio and Streamable HTTP clients. The host supplies the transport configuration, resolved credentials and HTTP client. A client owns one connection and its tool catalog; it does not manage accounts or install servers.

```rust
use std::{collections::HashMap, sync::Arc, time::Duration};
use kivio_agent_sdk::mcp::{McpClient, McpClientConfig, McpTransport};
use kivio_agent_sdk::tools::ToolRegistry;

let client = Arc::new(McpClient::connect(
    McpClientConfig {
        transport: McpTransport::Stdio {
            command: "my-mcp-server".into(), args: vec![],
            cwd: None, env: HashMap::new(),
        },
    },
    reqwest::Client::new(),
).await?);
let mut tools = ToolRegistry::new();
client.register_tools(&mut tools, "my-server", authorization, Duration::from_secs(60)).await?;
session.set_tools(tools)?;
session.prompt("Use the server's tools").await?;
client.close().await?;
```

`authorization` is the host's `Arc<dyn ToolAuthorization>`, and `session` is an idle `AgentSession`. Registration requires authorization even when a server marks a tool read-only. It preserves exact remote names behind namespaced model names and rejects collisions without partially changing the registry.

`list_tools()` follows pagination and refreshes after `tools/list_changed`. Build a new registry and call `set_tools` between runs to publish a changed catalog. Running, stopping and finishing sessions reject replacement. Explicitly close the client after its consumers finish. Stop and timeout notify the MCP server while keeping other calls on the connection usable. A remote side effect can still have an unknown outcome; the SDK does not automatically replay failed calls. The pinned HTTP transport retains its protocol-specific retry for an expired MCP session.

The supported MCP surface is tools, including image and structured tool results. Resources, resource templates, prompts, sampling and elicitation do not have SDK APIs in this version. Embedded resource blocks in a tool result do not imply resource discovery support. OAuth login, token refresh, credential storage, connection pools and UI remain host responsibilities.

[The independent MCP consumer](../../tests/fixtures/agent-sdk-consumer/tests/mcp.rs) supplies real local stdio and HTTP servers and exercises registration, authorization, cancellation and cleanup: `cargo test --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml --test mcp`.

## Verify the SDK and an independent consumer

These checks do not start the desktop application or require model credentials.

```powershell
cargo test --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml
cargo test --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --features skills
cargo test --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --all-features
cargo check --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --examples --all-features
cargo doc --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --no-deps --all-features
cargo test --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml
py -3 scripts/check-agent-sdk.test.py
py -3 scripts/check-agent-sdk.py
```

From a clean checkout, verify the actual distributable and run the same consumer against its extracted contents:

```powershell
cargo package --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml
py -3 scripts/check-agent-sdk.py --package crates/kivio-agent-sdk/target/package/kivio-agent-sdk-0.1.0-alpha.1.crate --test-consumer
```

The [consumer fixture](../../tests/fixtures/agent-sdk-consumer) is a separate Cargo package using the public SDK, reqwest, serde, serde_json, schemars and Tokio. Its tests cover actual requests, tool results, events, queues, cancellation, authorization and protocol failures. The package check copies this fixture, changes only its SDK path and preserves its locked dependencies.

| Integration | Executable public API reference |
| --- | --- |
| Custom model and request hooks | [request_hooks.rs](../../tests/fixtures/agent-sdk-consumer/tests/request_hooks.rs) |
| Typed tool and explicit authorization | [registry_http.rs](../../tests/fixtures/agent-sdk-consumer/tests/registry_http.rs) |
| Session lifecycle and host commit | [agent_session.rs](../../tests/fixtures/agent-sdk-consumer/tests/agent_session.rs) |
| Tree snapshots and authority rebinding | [snapshot.rs](../../tests/fixtures/agent-sdk-consumer/tests/snapshot.rs) |

Run its CLI with the same explicitly supplied live environment.

```powershell
cargo run --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml
```

Windows is the locally verified SDK platform for this prerelease. CI configures stable Rust tests and packaged consumption on Windows, macOS and Linux; current-commit three-platform runs remain unverified. The v2 live Chat smoke test used `deepseek-v4.1-flash`, executed echo once and returned `Completed`. This does not verify live Responses, Anthropic or Gemini compatibility, or desktop UI behavior. See [the v2 acceptance record](../../docs/prd/kivio-agent-sdk-v2-plan.md) and [migration guide](docs/contracts.md#migrate-from-the-initial-sdk) for the actual scope.

## Inject runtime context

Configure ordered `InjectionSource` implementations with `AgentSessionConfig::with_injections(InjectionSources::new().with(source))`. Advanced hosts pass the collection directly to `run_agent_loop` after the policy argument. Sources collect at `BeforeStep` and `BeforeFinish`; the finish snapshot includes the candidate answer. A nonempty finish batch requests another step after pending user and host input has been handled.

Async sources receive an `InjectionAdmission` sink. Add revocable `InjectionBatch` values with `pending`; use validated `PreparedInjection` and `committed` after producer-owned durable transactions. Committed facts survive later source failures. Notification failures are isolated, while persistence failures stop the run with retained partial history.

`InjectionQueue` provides typed kind registration, lazy per-kind batching and freshness checks, per-entry receipts, clear/unregister and a readiness subscription. Register its clone as a source and give typed senders to producers. Hosts use `WhenIdle` readiness with their existing run reservation; the SDK does not start autonomous turns. `Applied` means live-history inclusion, not model consumption or storage acknowledgement. Todo rules, authority and durable delivery remain application-owned.

Run [injections_offline](examples/injections_offline.rs) for a complete source and an owned session with a deterministic provider. The [runtime injection contract](docs/contracts.md#runtime-injections) covers ordering, validation, settlement and migration from `before_step` or `Continue(messages)`.
