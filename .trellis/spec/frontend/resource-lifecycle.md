# Resource Lifecycle and Bounded Runtime State

## 1. Scope / Trigger

Apply this contract when code creates or retains an OS child process, helper thread, Web Worker,
TCP listener, long-lived connection task, delayed task, or process-wide cache/registry. These
resources must have an explicit owner and a bounded cleanup path; dropping a registry key or UI
reference alone is not sufficient when work can continue outside that owner.

## 2. Signatures

Representative boundaries:

```rust
fn capture_stdout_with_timeout(
    cmd: std::process::Command,
    timeout: std::time::Duration,
) -> Option<String>;

impl MacOcrClient {
    pub fn shutdown(&self);
}

pub fn stop_all_previews();
```

```ts
runPythonInSandbox(
  code: string,
  timeoutMs: number,
  files?: PythonInputFile[],
): Promise<PythonRunOutcome>

disposePythonSandbox(): void
```

Process-wide cwd discovery caches use `TimedCacheEntry<V>` and shared get/set helpers with a
capacity of 64 and a retention TTL of 300 seconds.

## 3. Contracts

- The layer that creates a resource stores its stop handle: child handle, task handle, shutdown
  sender, Worker reference, or cache entry.
- Child-process timeout/exit cleanup means terminate **and** reap. Any stdout/stderr reader thread
  must be joined after the pipe closes; it must not be detached on timeout.
- Worker reset is one operation: reject every pending promise, clear every guard timer, terminate
  the Worker, and clear the reusable reference. The next request may create a fresh Worker.
- A backend timeout that waits for frontend Worker completion must expire after the frontend reset
  budget, never before it.
- Listener accept loops, request reads, SSE loops, and heartbeat waits share one shutdown signal.
  Stop broadcasts shutdown, aborts the owning listener task as a fallback, and clears sender/port
  registry state.
- Delayed idle tasks must be cancellable and must not be recreated after explicit shutdown.
  Shutdown state is checked both before and after acquiring the child-owner lock to close races.
- App-exit cleanup is idempotent and covers Preview, OCR, MCP/external sessions, and tracked
  background commands without depending on a window still being mounted.
- Long-lived maps are bounded by TTL/capacity or use `Weak` values. LRU ties use a deterministic
  key fallback. Locks are held only for map operations, never for external I/O.

## 4. Validation & Error Matrix

| Condition | Required behavior |
|---|---|
| Child finishes normally | Join readers, reap child, return its compatible result |
| Child exceeds timeout | Kill, wait/reap, join readers, return the existing fallback/error |
| Worker timeout/error/message error/dispose | Reject and clear all pending runs, terminate once, allow later rebuild unless disposed UI is gone |
| OCR idle deadline | Do nothing while requests are pending; otherwise request shutdown and clear the current child slot |
| OCR explicit shutdown races with request startup | No replacement helper may spawn after shutdown wins |
| Preview client has not sent a request yet | Shared shutdown interrupts the pending read and closes the socket |
| Preview stop is called repeatedly | No panic; sender/task/port remain empty and restart still works |
| Cache entry is expired | Remove it during the next read or write sweep |
| Cache exceeds capacity | Evict least-recently-used entry with deterministic tie-breaking |
| Weak per-key lock has no strong owners | Prune it; concurrent lookup for the same key still creates one live lock |

## 5. Good / Base / Bad Cases

- **Good**: a Python run hangs, all concurrent runs reject immediately, the Worker terminates, and
  the next run starts in a new Worker.
- **Base**: Preview starts once, serves HTML/SSE, stops, releases its port, and restarts normally.
- **Good**: two knowledge libraries acquire independent locks while concurrent work for one library
  shares a single lock.
- **Bad**: a timeout returns while a child, reader thread, Worker, listener, heartbeat, or pending
  promise continues running.
- **Bad**: shutdown wakes a pending request whose error path schedules a new idle task.
- **Bad**: a cwd-keyed `HashMap` retains every directory visited for the rest of the process.

## 6. Tests Required

- Child process: successful output and timeout kill/reap complete within a bounded wall-clock time.
- Worker: idle termination, multi-pending timeout rejection, error/message-error reset, idempotent
  dispose, and successful lazy rebuild.
- Preview: sender/task/port state clears; SSE and half-open sockets close; listener port can be
  rebound; a second start/stop succeeds.
- Cache: TTL full-table sweep, value update/TTL refresh, deterministic LRU eviction, and concurrent
  capacity bound.
- Weak locks: live-lock reuse, concurrent same-key identity, dead-entry pruning, and different-key
  independence.
- macOS OCR: Swift helper builds; shutdown protocol exits; Rust app-exit path calls `shutdown()`;
  idle/restart behavior is smoke-tested when changing sidecar process APIs.
- Run frontend lint/type-check/tests, Rust lib tests, and the Swift helper build before completion.

## 7. Wrong vs Correct

### Wrong

```rust
let _ = timeout(duration, child.wait_with_output()).await;
// Timeout returns, but ownership needed to kill/reap the child is gone.
```

```ts
worker?.terminate()
worker = null
// Pending promises and their timers remain alive.
```

### Correct

```rust
let _ = child.kill();
let _ = child.wait();
let _ = reader.join();
```

```ts
function resetWorker(error: Error) {
  rejectAllPending(error) // clears every guard timer
  terminateWorker()
}
```
