# Kivio Agent SDK

`kivio-agent-sdk` runs a model and tool loop in a Rust host without Tauri or the Kivio application. The desktop application uses this same crate through a path dependency.

Version `0.1.0-alpha.1` is a source-consumed prerelease under [GPL-3.0-or-later](LICENSE). This repository does not publish the crate as part of the SDK migration. Public APIs can change between prereleases. Pin a repository commit and keep your lockfile when integrating this version. No minimum supported Rust version is declared.

## Run the examples without credentials

Run these commands from the repository root with a current stable Rust toolchain.

```powershell
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- minimal
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- cancel
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- inputs
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- continue
```

The default example starts a bounded loopback HTTP server and calls the production `OpenAiChatProvider`. It does not read model credentials or application settings. Cargo may download locked dependencies on the first build.

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

The [session example](examples/session.rs) constructs the provider and sends two prompts. Run it with the environment variables below using `cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example session`. No empty executor, host, or mailbox implementation is required. Set `config.tools` to a `ToolRegistry` to add tools. Set `config.host` only when the host needs checkpoints or a final `commit_run` operation.

`prompt` admits immediately and returns a waiter; concurrent sends return `Admission(Busy)`. `abort()` requests cancellation. `wait_inactive().await` waits for the current invocation's tool cleanup and commit. Dropping the waiter requests stop while the owned worker finishes cleanup. Keep the Tokio runtime alive until this wait completes. `continue_run()` continues an unanswered user message or completed tool round; it never executes saved tool results. See [session and hook contracts](docs/contracts.md#owned-agent-session) for queue, subscription, and failure semantics.

Advanced hosts with existing admission and persistence owners can compose the lower-level interfaces as the original example does:

1. Construct `OpenAiChatProvider` with your HTTP client and an `OpenAiChatConfig` containing the complete Chat Completions endpoint URL.
2. Implement `AgentRunControl` for one invocation's input mailbox and stop signal. Create an `AgentRunHandle`, and clone that handle for controls that target the same invocation.
3. Implement `ToolExecutor::before_tool_call` to authorize the registered tool and validate its arguments. Implement `call` to execute only the prepared operation.
4. Set `AgentRunConfig.runtime_messages`, model, tools, and host-supplied identity. Use matching identity values when constructing the handle.
5. Call `run_agent_loop(config, &host, &executor, &mut NoSessionPolicy, &events, handle).await`.
6. Check both the returned `Result` and `AgentRunResult.status`. Persist `AgentRunResult.history` in your own session store before releasing your application's run capacity.

Start with `NoSessionPolicy` and an empty `AgentHost` implementation, as the example does. These enable no product policies or persistence. Add checkpoints, context management, or recovery only when your host needs them. A host with no tools still supplies a `ToolExecutor` implementation, but sets `tools` to an empty vector.

The SDK does not register desktop tools or load permissions from Kivio settings. Registering a `ToolDefinition` advertises a model-visible tool. Authorization remains an explicit operation in your executor. See [the API and lifecycle contracts](docs/contracts.md) before adding tools with external effects.

## Run against a real provider

The standalone providers currently support these protocols. Desktop adapters share their request encoders and response decoders.

| Protocol | Public config and provider | Endpoint |
| --- | --- | --- |
| Chat Completions | `model::openai::{OpenAiChatConfig, OpenAiChatProvider}` | Complete `/chat/completions` URL |
| Responses, including xAI format | `model::responses::{OpenAiResponsesConfig, OpenAiResponsesProvider}` | Complete `/responses` URL |
| Anthropic Messages | `model::anthropic::{AnthropicMessagesConfig, AnthropicMessagesProvider}` | Complete `/messages` URL |

Construct a provider with your HTTP client and config. Each implements `LanguageModelProvider` for direct generation and `ModelRuntime` for `AgentSession`. Responses accepts buffered JSON or SSE-only responses, preserves encrypted reasoning for same-model replay, and supports function tools and provider-side search. Use `ResponsesFormat::Xai` explicitly for xAI wire behavior. The host resolves cache keys, credentials and compatibility options; the standalone providers do not learn endpoint failures or rotate keys.

Anthropic preserves signed thinking, redacted thinking and native server-tool blocks. Its explicit options select adaptive or budget-based thinking, cache TTL and an opaque provider identity for replay isolation. Native `pause_turn` responses continue up to eight times; final context usage remains separate from cumulative billing usage. Claude Code identity and model-profile selection remain host responsibilities.

Gemini remains an application adapter until its T3 extraction step.

Set environment variables in the example process. `KIVIO_SDK_BASE_URL` is the **full endpoint**, such as `https://api.example.com/v1/chat/completions`, rather than a base directory. `KIVIO_SDK_MODEL` selects your provider's model. `KIVIO_SDK_API_KEY` is optional for endpoints without bearer authentication. Supply the key through your process environment or secret manager, and omit it from command arguments and committed files.

```powershell
$env:KIVIO_SDK_BASE_URL = 'https://api.example.com/v1/chat/completions'
$env:KIVIO_SDK_MODEL = 'your-tool-capable-model'
cargo run --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --example agent -- minimal --live
```

You can also append `--live` to `cancel`, `inputs`, or `continue`. These modes make model requests and depend on the provider's tool behavior. The loop, rather than a fixed request script, decides how many model steps are needed. The example sets a 60-second HTTP client timeout and retains the client's default retry policy. The SDK adds no retry loop. Hosts that need a stricter transport policy configure their client as described in [the retry contract](docs/contracts.md#errors-and-retries).

Only the example reads these environment variables. The SDK receives the resolved endpoint, credentials, headers, and HTTP client. The live example keeps reqwest's client defaults for proxy behavior. Configure `.proxy(...)`, `.no_proxy()`, TLS, and timeouts on your client as needed. The local example explicitly disables proxies for its loopback requests.

## Verify the SDK and an independent consumer

These checks do not start the desktop application or require model credentials.

```powershell
cargo test --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml
cargo check --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --examples
cargo doc --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --no-deps
cargo test --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml
```

The [consumer fixture](../../tests/fixtures/agent-sdk-consumer) is a separate Cargo package. It imports only the public SDK, reqwest, serde_json, and Tokio. Its loopback HTTP tests cover tool results in subsequent requests, stream events, history continuation, steer and follow-up, cancelled connections, authorization rejection, and protocol failures. Run its CLI with the same explicitly supplied live environment.

```powershell
cargo run --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml
```

Windows is the locally verified SDK platform for this prerelease. The repository's SDK CI configures stable Rust checks on Windows, macOS, and Linux. A configured job is not evidence that a platform has passed. The recorded live smoke test used `deepseek-v4.1-flash`, executed echo once, and returned `Completed`. Compatibility with every OpenAI-compatible endpoint or model is not claimed. See [the SDK acceptance record](../../docs/prd/kivio-agent-sdk-plan.md) for the actual validation scope.
