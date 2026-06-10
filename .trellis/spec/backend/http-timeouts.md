# HTTP Timeouts

> Contracts for provider/network timeouts in the Tauri backend.

## Scenario: Provider Requests And SSE Streams

### 1. Scope / Trigger

- Trigger: changing `src-tauri/src/api.rs`, provider adapters under `src-tauri/src/chat/model/**`, `send_with_retry`, `send_with_failover`, or any code that reads SSE chunks from a provider.
- Problem prevented: a global `reqwest::Client::timeout(60s)` applies from connection start until the entire body finishes. That is correct for small non-streaming calls, but it kills active long SSE responses after 60 seconds even when tokens/tool-call arguments are still arriving.

### 2. Signatures

- Shared client constructor: `build_http_client() -> reqwest::Client`
- Non-streaming helper: `with_standard_request_timeout(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder`
- Retry wrappers continue to accept closures returning `Future<Output = Result<reqwest::Response, reqwest::Error>>`.

### 3. Contracts

- `build_http_client()` must not set a total request timeout with `.timeout(...)`.
- `build_http_client()` must set a connection timeout to avoid hanging during DNS/TCP/TLS setup.
- `build_http_client()` must set a read idle timeout so stalled response bodies eventually fail.
- Non-streaming provider/API calls must wrap their request builder with `with_standard_request_timeout(...)`.
- Streaming SSE calls must not use `with_standard_request_timeout(...)`; they rely on client connect timeout plus read idle timeout.
- Long downloads and generation calls may use their own explicit longer `.timeout(...)` when the operation has a known different SLA.

### 4. Validation & Error Matrix

- Non-streaming request exceeds standard total timeout -> request returns a reqwest timeout error and existing retry/failure handling applies.
- SSE stream keeps receiving chunks for longer than 60 seconds -> stream remains alive.
- SSE stream receives no chunks for the read idle window -> request returns a reqwest timeout/read error and existing stream failure handling applies.
- Provider returns 401/402/403/429 -> existing failover behavior applies.
- Provider returns 429/5xx on non-failover path -> existing retry behavior applies.

### 5. Good / Base / Bad Cases

- Good: Chat agent streams tool-call arguments for several minutes with active chunks; no total 60-second cutoff.
- Base: Fetch models/test provider/search requests use `with_standard_request_timeout(...)` and fail/retry normally if the whole request takes too long.
- Bad: Putting `.timeout(Duration::from_secs(60))` on the shared `Client` or on a streaming provider request.

### 6. Tests Required

- Run `cargo check --manifest-path src-tauri/Cargo.toml` after timeout changes.
- Run targeted backend tests for changed provider/API code.
- Run `cargo test --manifest-path src-tauri/Cargo.toml` when practical.
- For touched Rust files, run `rustfmt --edition 2021 --check <changed files>` if full `cargo fmt --check` is blocked by unrelated formatting drift.

### 7. Wrong vs Correct

#### Wrong

```rust
Client::builder()
    .timeout(Duration::from_secs(60))
    .build()
```

#### Correct

```rust
Client::builder()
    .connect_timeout(HTTP_CONNECT_TIMEOUT)
    .read_timeout(HTTP_READ_IDLE_TIMEOUT)
    .build()
```

```rust
with_standard_request_timeout(
    state.http.get(url).bearer_auth(key),
)
.send()
```
