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

## Scenario: Provider Retry And Threshold-Based Key Failover

### 1. Scope / Trigger

- Trigger: changing retry/failover logic in `src-tauri/src/api.rs` (`send_with_retry`, `send_with_failover`, `send_with_retry_status_policy`, `FailoverRetryPolicy`, `is_immediate_failover_status`, `is_failover_error`, `retry_delay_ms`, the `RETRY_*` / `RATE_LIMIT_KEY_SWITCH_THRESHOLD` constants) or the retry defaults in `src-tauri/src/settings.rs` (`default_retry_attempts`, `clamp_retry_attempts`).
- Problem prevented: transient errors (5xx / timeout / connect / 429) immediately failing a whole run; and 429 burning backup keys by switching keys on the first rate-limit instead of backing off on the current key first.

### 2. Defaults

- `default_retry_attempts` = `5`; `clamp_retry_attempts` clamps to `1..=8`; `retry_enabled` default `true`.
- `RETRY_BASE_DELAY_MS` = `5_000` (start backoff ~5s); `RETRY_MAX_DELAY_MS` = `30_000` (cap). Exponential per attempt, capped at the max.
- `RATE_LIMIT_KEY_SWITCH_THRESHOLD` = `2` (consecutive 429s on the same key before switching keys).
- Retry-After always takes priority over computed backoff.
- Worst-case wall-clock for a fully transient failure on `attempts = 5` is the sum of inter-attempt sleeps `5s + 10s + 20s + 30s = ~65s` per key. 5xx/timeout do not switch keys, so they stay bounded to one key's ~65s regardless of key count. 429 across N keys switches at the threshold (one ~5s sleep) per intermediate key and runs the full ~65s only on the final (backup-less) key; with two keys this is ~5s + ~65s ≈ 70s. Provider/test calls that need a tighter bound should lower `retry_attempts` or wrap with an explicit operation timeout, since this resilience is intentionally allowed to take that long.

### 3. Error Classification Contract

- **401 / 402 / 403 (bad/expired key)** -> `is_immediate_failover_status` true; inner does NOT retry; bubbles immediately so `send_with_failover` switches keys.
- **429 (rate limit)** -> inner backs off and retries on the current key. Only when the same key hits `RATE_LIMIT_KEY_SWITCH_THRESHOLD` consecutive 429s AND a switchable backup key exists (an untried key that `pick_active_key` would return next — it prefers un-cooled keys but falls back to a cooled untried key when every key is cooled, so the outer always has somewhere to go) does the 429 bubble so the outer switches keys (counter resets on the new key). With no backup key, 429 keeps backing off up to the total attempt limit.
- **5xx / timeout / connect (transient)** -> inner backs off and retries up to the total attempt limit; never switches keys (not a key problem).
- **400 / 404 / 422 and other deterministic 4xx** -> no retry, fast fail (retrying always fails).

### 4. Implementation Shape

- `send_with_retry_status_policy(label, attempts, send, policy: FailoverRetryPolicy)` is the single retry loop. `FailoverRetryPolicy { rate_limit_cap: Option<usize> }`: `Some(N)` means a backup key is available and 429 stops retrying after N attempts; `None` means retry 429/5xx up to the total attempt limit.
- `send_with_failover` computes `has_backup_key` per key via `state.pick_active_key(provider_id, total, &tried)` and passes `Some(RATE_LIMIT_KEY_SWITCH_THRESHOLD)` only when a backup is available.
- `is_failover_error` (outer key-switch decision) stays `401 | 402 | 403 | 429`: a bubbled 429 means the inner threshold was reached, so the outer is allowed to switch.

### 5. Validation & Error Matrix

- 5xx repeated -> retries to attempt limit, then fails.
- timeout/connect repeated -> retries to attempt limit, then fails.
- 400/404/422 -> single attempt, immediate failure.
- 401/402/403 -> single attempt, bubbles to key switch.
- 429 with no backup key -> backs off to attempt limit.
- 429 with backup key -> backs off to threshold N on the current key, then switches keys.
- Retry-After header present -> its delay wins over exponential backoff.

### 6. Tests Required

- Run `cargo test --manifest-path src-tauri/Cargo.toml` after retry/failover changes.
- Unit tests use mock `send` closures (call-count driven) plus `reqwest::Response::from(http::Response)` to drive `send_with_retry_status_policy` / `send_with_failover`. Use `#[tokio::test(start_paused = true)]` so backoff sleeps are instant (requires tokio `test-util` dev feature).

