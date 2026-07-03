# Research: How OpenAI Codex CLI manages long-running / background processes

- **Query**: How does Codex (Rust) start, stream, poll, and stop long-running / background / dev-server processes? Windows console suppression, running-state legibility to the LLM, dedup, dev-server handling.
- **Scope**: external (cloned repo)
- **Date**: 2026-07-03
- **Source**: `git clone --depth 1 https://github.com/openai/codex` @ commit `da4c8ca57` (2026-07-02). Cloned to a scratch dir outside Kivio. Rust workspace lives in `codex-rs/`.
- **Note on paths**: all `file:line` citations below are relative to `codex-rs/`.

## TL;DR

Codex has **two distinct exec paths**:

1. **`shell_command` / legacy `process_exec_tool_call`** — a strictly **foreground, timeout-bounded** one-shot. Spawns, streams to events, waits for exit OR a timeout (default 10s), then kills. No session survives. (`core/src/exec.rs`)
2. **"Unified exec" = `exec_command` + `write_stdin`** — a **session/registry-backed model** for long-running & interactive processes. `exec_command` starts a process, waits a short *yield window* (default 10s, clamped 250ms–30s) for initial output, then if the process is still alive **returns a `session_id` and leaves it running in a registry**. The model polls/feeds it later via `write_stdin` (empty `chars` = poll-only). This is Codex's answer to dev servers / watchers / REPLs. (`core/src/unified_exec/`)

There is **no `CREATE_NO_WINDOW` on the normal child-spawn path** (it uses ConPTY or a plain `tokio::process::Command`). `CREATE_NO_WINDOW` (0x08000000) appears **only** for Codex's own Windows sandbox *helper/installer* processes, not for the model's commands.

---

## Q1 — Background / long-running process model

**Yes. Codex supports long-running background processes via the "unified exec" subsystem**, which is a session registry, not a fire-and-forget background flag.

### The two tools exposed to the model
`core/src/tools/handlers/shell_spec.rs`:
- `exec_command` (`:88-108`): *"Runs a command in a PTY, returning output **or a session ID for ongoing interaction**."* Params: `cmd`, `workdir`, `tty`, `yield_time_ms`, `max_output_tokens`.
- `write_stdin` (`:110-152`): *"Writes characters to an existing unified exec session and returns recent output."* Params: `session_id`, `chars` (*"Defaults to empty, which polls without writing"*), `yield_time_ms`, `max_output_tokens`.

### The registry
`core/src/unified_exec/mod.rs`:
```rust
pub(crate) struct UnifiedExecProcessManager {
    process_store: Mutex<ProcessStore>,          // :134-137
    max_write_stdin_yield_time_ms: u64,
}
#[derive(Default)]
pub(crate) struct ProcessStore {                 // :121-132
    processes: HashMap<i32, ProcessEntry>,        // session_id -> live process
    reserved_process_ids: HashSet<i32>,
}
pub(crate) const MAX_UNIFIED_EXEC_PROCESSES: usize = 64;   // :73
```
`ProcessEntry` (`:155-166`) holds an `Arc<UnifiedExecProcess>`, `call_id`, `process_id`, `cwd`, `hook_command`, `tty`, `last_used`, and a **`Weak<Session>`** (so the registry doesn't keep the session alive). Session IDs are random `1_000..100_000` (`process_manager.rs:371-396`, `allocate_process_id`).

### Start → decide foreground vs background
`core/src/unified_exec/process_manager.rs::exec_command` (`:408-639`):
1. `open_session_with_sandbox` runs approval + sandbox selection, then spawns a PTY or pipe process (`:1107-1205` → `open_session_with_prepared_exec_env`).
2. `start_streaming_output(&process, ...)` (`:450`) begins pumping deltas to the UI.
3. It records whether the process is still alive: `let process_started_alive = !process.has_exited() && process.exit_code().is_none();` (`:454`).
4. **If alive, it stores the process in the registry BEFORE waiting** (`store_process`, `:457-470`) — the comment (`:452-453`) explains this is so interrupting the turn can't drop the last `Arc` and kill the background process.
5. It then waits only for a bounded *yield window* (`clamp_yield_time`, `:478`) collecting initial output via `collect_output_until_deadline` (`:490-503`).
6. Return shape (`ExecCommandToolOutput`, `:625-636`): if the process **exited** during the window → `process_id: None` + `exit_code`. If **still running** → `process_id: Some(session_id)`, `exit_code: None`. The process keeps running detached.

### Stream / poll output
- **Live streaming to UI**: `unified_exec/async_watcher.rs::start_streaming_output` (`:40-102`) spawns a task reading a broadcast channel, splits on UTF-8 boundaries, emits `ExecCommandOutputDelta` events (capped, see Q3).
- **Model polling**: `write_stdin` with empty `chars` polls output for a window (`process_manager.rs::write_stdin` `:641-794`). Output buffering uses a `HeadTailBuffer` (`unified_exec/head_tail_buffer.rs`) shared via `OutputHandles` (`process.rs:59-66`). Poll waits on a `tokio::sync::Notify` + `CancellationToken` until deadline (`collect_output_until_deadline` `:1207-1294`).

### Yield / timeout constants
`core/src/unified_exec/mod.rs:64-73`:
```rust
pub(crate) const MIN_YIELD_TIME_MS: u64 = 250;
pub(crate) const WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS: u64 = 2_000; // Windows floor
pub(crate) const MIN_EMPTY_YIELD_TIME_MS: u64 = 5_000;      // empty poll floor
pub(crate) const MAX_YIELD_TIME_MS: u64 = 30_000;
pub(crate) const DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS: u64 = 300_000; // 5 min max poll
```
Note the **Windows-specific floor of 2000ms** on the initial exec yield (`clamp_yield_time`, `:168-175`) — Codex found Windows process startup slower and raises the minimum wait.

### Stop / terminate
- `UnifiedExecProcess::terminate()` / `terminate_confirmed()` / `interrupt()` (`process.rs:208-246`). `Drop` calls `terminate()` (`:614-618`).
- Manager-level: `terminate_process(process_id)` (`process_manager.rs:1416-1448`), `terminate_all_processes()` (`:1379-1395`), `list_processes()` (`:1397-1414`).
- These are surfaced to clients as app-server RPCs `thread/backgroundTerminals/{list,terminate,clean}` (`app-server-protocol/src/protocol/common.rs:592-608`).
- **Interrupt** for a non-tty session: the model sends the ETX byte `"\u{3}"` (Ctrl-C) via `write_stdin` (`process_manager.rs:84` `const INTERRUPT`; handled `:665-671`).

### Contrast: the foreground path
`core/src/exec.rs` is the *other*, non-session path (`process_exec_tool_call` `:297-317` → `exec` `:900-957` → `consume_output` `:961-1098`). It has **no registry**: it spawns, waits on `tokio::select!` for `child.wait()` vs an `ExecExpiration` (timeout/cancel) vs Ctrl-C, and on timeout kills the whole process group (`:1004-1059`). `DEFAULT_EXEC_COMMAND_TIMEOUT_MS = 10_000` (`:58`). The `shell_command` tool (`shell_spec.rs:154-222`) exposes only `timeout_ms` — no session output. So Codex deliberately keeps a simple bounded path AND a session path.

---

## Q2 — Windows console-window suppression

**Key finding: the model's commands are NOT spawned with `CREATE_NO_WINDOW`.** The default Windows spawn paths set no creation flags:

- **PTY path** (`utils/pty/src/pty.rs`): uses `portable_pty` ConPTY on Windows (`platform_native_pty_system` `:113-123` → `crate::win::ConPtySystem`). Console handling is delegated to ConPTY; no `creation_flags` call.
- **Pipe path** (`utils/pty/src/pipe.rs`): plain `tokio::process::Command::new(program)` (`:128`) with `.stdin/.stdout/.stderr` piped (`:157-166`), `command.spawn()` (`:168`). **No `.creation_flags(...)`, no `CommandExt`, no `CREATE_NO_WINDOW`.** On Windows it relies on inheriting the parent console (Codex CLI/TUI is itself a console process, so no new window pops).

`CREATE_NO_WINDOW` / `0x08000000` appears **only in `windows-sandbox-rs`, and only for Codex's own helper/installer processes**, never for the model's commands:

1. `windows-sandbox-rs/src/elevated/runner_client.rs:355-356` — spawning the *elevated sandbox runner helper* via `CreateProcessWithLogonW`:
   ```rust
   windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
       | windows_sys::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT,
   ```
2. `windows-sandbox-rs/src/setup.rs:764` — the sandbox *setup/installer* command:
   ```rust
   .creation_flags(0x08000000) // CREATE_NO_WINDOW
   ```
3. `windows-sandbox-rs/src/bin/setup_main/win.rs:173` — same, another setup helper.

**The actual sandboxed child command** (`windows-sandbox-rs/src/process.rs`, `CreateProcessAsUserW`) does **NOT** use `CREATE_NO_WINDOW`. It uses `CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT` (`:123`) or just `CREATE_UNICODE_ENVIRONMENT` (`:164`), and instead controls window/desktop via `STARTUPINFO.lpDesktop` pointing at the interactive or a **private desktop** (`:100-102`, comment: *"if lpDesktop is not set when launching with a restricted token. Point explicitly at the interactive desktop or a private desktop."*). Console suppression here is a desktop-isolation concern, not a `CREATE_NO_WINDOW` concern.

Windows process kill uses `OpenProcess(PROCESS_TERMINATE)` + `TerminateProcess` on the single pid (`utils/pty/src/pipe.rs:71-87` `kill_process`) — note there is **no** `taskkill /T` process-tree kill on the pipe/Windows path (process-group kill is Unix-only, see Q-cross-cutting).

**Takeaway relevance**: Codex is a console app, so it can get away with no `CREATE_NO_WINDOW` on the default path. Kivio is a **GUI Tauri app**, so a child console program (e.g. a build tool) *can* pop a console window — this is exactly why Kivio needs the `CREATE_NO_WINDOW` flag that Codex's model-command path omits.

---

## Q3 — Readiness / running-state legibility to the LLM

The model receives a structured, capped result. `core/src/tools/context.rs` (`ExecCommandToolOutput`):

### Text rendering to the model (`response_text`, `:412-438`)
Sections joined by newlines:
```
Chunk ID: <id>                                  (if present)
Wall time: <secs>.4 seconds                     (:419-420)
Process exited with code <n>                     (only if exit_code is Some)   :422-424
Process running with session ID <session_id>     (only if still running)        :426-428
Original token count: <n>                        (:430-432)
Output:
<truncated output>
```
So **"done" vs "still running" is explicit**: a finished process reports `Process exited with code N` (and `session_id`/`process_id` is `None`); a live process reports **`Process running with session ID <id>`** and the model knows to use `write_stdin`. The output schema (`shell_spec.rs:261-293`) documents `session_id` as *"Session identifier to pass to write_stdin when the process is still running."*

### Output caps / truncation
- Model output is truncated to a token budget: `model_output_max_tokens = resolve_max_tokens(max_output_tokens).min(truncation_policy.token_budget())` (`context.rs:403-405`), default `DEFAULT_MAX_OUTPUT_TOKENS = 10_000` tokens (`unified_exec/mod.rs:70`). Truncation via `formatted_truncate_text(TruncationPolicy::Tokens(..))` (`context.rs:407-410`), which also reports `original_token_count` so the model knows how much was elided.
- Retained buffer is hard-capped: `UNIFIED_EXEC_OUTPUT_MAX_BYTES = 1 MiB` (`unified_exec/mod.rs:71`) held in a `HeadTailBuffer` (keeps head+tail, drops middle).
- Live event deltas are capped: `MAX_EXEC_OUTPUT_DELTAS_PER_CALL = 10_000` per call (`exec.rs:80`) and each delta ≤ `UNIFIED_EXEC_OUTPUT_DELTA_MAX_BYTES = 8192` bytes (`async_watcher.rs:35`).

### Timeout / exit handling (foreground path `exec.rs`)
- `finalize_exec_result` (`:765-823`): on Unix, a non-timeout signal becomes `CodexErr::Sandbox(Signal)`; timeouts map to conventional exit code `124` (`EXEC_TIMEOUT_EXIT_CODE`, `:65`, `:787-789`) and return `SandboxErr::Timeout { output }` carrying the partial output box (`:803-807`).
- On timeout it kills the process group then `start_kill` (`:1011-1018`); on cancellation it sends SIGTERM to the group, waits a 50ms grace, then SIGKILL (`:1019-1049`).
- I/O drain guard: after the child exits, output-reader tasks are awaited with a `IO_DRAIN_TIMEOUT_MS = 2_000` cap and `abort()`ed if grandchildren hold the pipes open (`:1064-1088`, comment `:82-89`).

### Background exit → single terminal event
When a stored session eventually exits, `spawn_exit_watcher` (`async_watcher.rs:107-157`) waits on the cancellation token, drains trailing output (`TRAILING_OUTPUT_GRACE = 100ms`), then emits **one** `ExecCommandEnd` event (`emit_exec_end_for_unified_exec` `:195-237`) with the aggregated transcript + exit code. So even a detached process produces a final legible record.

---

## Q4 — Duplicate-launch prevention / idempotency

**No command-level dedup / idempotency exists for exec.** Evidence:

- `exec_command` always calls `manager.allocate_process_id()` → a fresh random id (`process_manager.rs:371-396`); there is no lookup of "is this same command already running". Two identical `npm run dev` calls create two independent sessions.
- A repo-wide search for dedup/idempotency of commands found only unrelated hits (context dedup, connector merge, guardian review-session reuse, agent registry) — none for exec. (Grep over `core/` for `already running|deduplicat|idempoten|duplicate command|reuse.*session`.)
- The only "already running" strings are about **threads/agents**, not shell processes: `core/src/thread_manager.rs:1536,1661` and the multi-agent `send_follow_up` spec (`tools/handlers/multi_agents_spec.rs:205`).
- The nearest thing to reuse is **explicit**: the model must pass a known `session_id` to `write_stdin` to talk to an existing session (`unified_exec.rs` tests: `unified_exec_reuses_session_via_stdin`, `core/tests/suite/unified_exec.rs:2576`). Reuse is model-driven, not automatic dedup.

**Implicit back-pressure instead of dedup**: the registry is capped at `MAX_UNIFIED_EXEC_PROCESSES = 64`. When full, `prune_processes_if_needed` (`process_manager.rs:1332-1377`) evicts via LRU — it **protects the 8 most-recently-used** sessions, then prefers to prune an already-*exited* session, else the true LRU; the pruned process is `terminate()`d. So runaway duplicate launches are bounded but not prevented or deduped.

---

## Q5 — Dev-server-specific handling / sandbox & network policy

**No dev-server-specific code path** (no "if command looks like a dev server" heuristic, unlike Kivio's auto-`background:true` for `npm run dev`/`vite`). Codex treats a dev server as just another long-running unified-exec session — the *generic* session model IS the dev-server story. Relevant generic behaviors:

### Environment tuning for non-interactive tools
`core/src/unified_exec/process_manager.rs:69-80` — every unified-exec process gets a fixed env overlay to keep watchers/servers quiet and non-paging:
```rust
const UNIFIED_EXEC_ENV: [(&str, &str); 10] = [
    ("NO_COLOR", "1"), ("TERM", "dumb"),
    ("LANG", "C.UTF-8"), ("LC_CTYPE", "C.UTF-8"), ("LC_ALL", "C.UTF-8"),
    ("COLORTERM", ""), ("PAGER", "cat"), ("GIT_PAGER", "cat"), ("GH_PAGER", "cat"),
    ("CODEX_CI", "1"),
];
```

### tty vs pipes
`exec_command` has a `tty` param (default `false`, `unified_exec.rs:68`). `false`/omitted → plain pipes (`pipe::spawn_process_no_stdin_with_inherited_fds`), `true` → PTY (`pty::spawn_process_with_inherited_fds`) (`process_manager.rs:1079-1100`). Interactive REPLs/servers that need a TTY set `tty:true`; a dev server usually runs fine on pipes.

### Sandbox / network policy that affects servers
- Network is off by default under sandbox; when disabled, `CODEX_SANDBOX_NETWORK_DISABLED=1` is injected (`core/src/spawn.rs:78-80`, `:20`). A managed **network proxy** can be attached per-request (`NetworkProxy`, `spawn.rs:45,72-74`); denial can terminate a running session late via `terminate_process_on_network_denial` (`process_manager.rs:347-368`) — relevant because a dev server that tries to bind/fetch may be killed by network policy.
- Windows guidance injected into the tool description tells the model to background-launch helpers hidden: *"When using `Start-Process` to launch a background helper or service, pass `-WindowStyle Hidden` unless the user explicitly asked for a visible interactive window."* (`shell_spec.rs:402-407`, `windows_shell_guidance`). This is prompt-level, not code-level, console suppression.

### Parent-death cleanup (so servers don't leak)
- Unix: `pre_exec` sets `PR_SET_PDEATHSIG=SIGTERM` on Linux and `setsid`/new process group, so if Codex dies the child gets SIGTERM (`spawn.rs:86-105`; `utils/pty/src/process_group.rs:27-39, 47-78`). `kill_on_drop(true)` on the foreground path (`spawn.rs:125`).
- PTY/pipe children become session/group leaders so the whole tree can be killed (`pty.rs:186-190`, `pipe.rs:133-146`).
- **Windows has none of this** (`process_group.rs` functions are `#[cfg(not(unix))]` no-ops, `:41-45,61-65,80-84,114-118,145-149,157-161,169-173,185-189`). Windows kill is single-pid `TerminateProcess` (`pipe.rs:71-87`) — no process-tree kill. A dev server that spawns children on Windows can orphan them.

---

## Cross-cutting: process-group / kill semantics (`utils/pty/src/process_group.rs`)

| Concern | Unix | Windows |
|---|---|---|
| New group/session | `setsid` / `setpgid(0,0)` in `pre_exec` (`:47-78`) | no-op (`:80-84`) |
| Interrupt | `killpg(pgid, SIGINT)` (`:151-155`) | unsupported signal error |
| Terminate (graceful) | `killpg(pgid, SIGTERM)` (`:136-143`) | no-op |
| Kill (tree) | `killpg(pgid, SIGKILL)` (`:163-167`) | **single pid** `TerminateProcess` (`pipe.rs:71-87`) |
| Parent-death | Linux `PR_SET_PDEATHSIG` (`:27-39`) | none |

Foreground path escalation on cancel: SIGTERM group → 50ms grace (`CANCELLATION_TERMINATION_GRACE_PERIOD`, `exec.rs:66`) → SIGKILL group (`exec.rs:1019-1049`). This mirrors Kivio's own unix `killpg` SIGTERM→SIGKILL described in CLAUDE.md; Codex's Windows story is weaker (no `taskkill /T /F` equivalent).

---

## Takeaways for Kivio

1. **Session/registry model vs Kivio's job-log model.** Codex keeps a live in-memory registry (`HashMap<session_id, Arc<UnifiedExecProcess>>`) with the *actual process handle* + live output buffer, and the model interacts through `write_stdin(session_id, chars="")` to poll and `chars="\u{3}"` to Ctrl-C. Kivio currently captures stdout+stderr to a temp log file (`kivio-bgcmd-<job_id>.log`) and polls with `bash_output`. Codex's approach adds true interactivity (stdin) and avoids temp-file GC, at the cost of holding processes in memory. Both cap the registry / survive across turns.

2. **The "return a session_id instead of blocking" decision is data-driven, not command-name-driven.** Codex spawns, waits a bounded *yield window* (default 10s, min 250ms, Windows floor 2s), and if the process is still alive it returns `session_id` + leaves it running. Kivio instead pattern-matches command names (`npm run dev`/`vite`) to pre-decide `background:true`. Codex's "wait-then-detach" is more general and handles unknown long-runners. Consider adopting the yield-window heuristic.

3. **Windows console suppression is Kivio's problem to solve, and Codex does NOT solve it on its command path.** Codex's model commands use ConPTY or bare `tokio::process::Command` with **no creation flags** — safe only because Codex is itself a console app. `CREATE_NO_WINDOW (0x08000000)` is used by Codex ONLY for its own hidden sandbox/installer helpers (`runner_client.rs:355`, `setup.rs:764`). Kivio, being a GUI Tauri app, still needs `.creation_flags(CREATE_NO_WINDOW)` (via `std::os::windows::process::CommandExt`) on every child spawn to avoid flashing consoles — do not expect to copy this from Codex's command path.

4. **Explicit "still running" legibility for the LLM.** Codex renders a compact text block: `Process exited with code N` vs `Process running with session ID <id>`, plus `Wall time`, `Original token count`, and token-truncated `Output:` (10k-token default budget, 1 MiB retained head+tail buffer). Kivio's `bash_output` could adopt the same explicit exited-vs-running phrasing and an original-token-count hint so the model reasons correctly about truncation.

5. **No dedup — bounded eviction instead.** Codex does not prevent duplicate launches; it caps at 64 sessions and LRU-evicts (protecting the 8 newest, preferring already-exited). If Kivio wants idempotency (avoid two dev servers), it must add it itself — Codex is no precedent for dedup.

6. **Env hygiene for background/non-interactive processes.** Codex force-sets `TERM=dumb`, `NO_COLOR=1`, `PAGER=cat`/`GIT_PAGER=cat`/`GH_PAGER=cat`, `CI`-like `CODEX_CI=1` on every unified-exec child (`process_manager.rs:69-80`). Kivio background commands would benefit from the same to prevent pagers hanging and ANSI noise in captured logs.

7. **Cross-turn survival + cleanup.** Codex stores the process before the initial wait so a turn interrupt can't drop the last `Arc` (`process_manager.rs:452-453`), cascades cancellation from the parent generation, and cleans up via `terminate_all_processes` + `Drop`. Kivio already survives across turns and sweeps on app exit; Codex additionally exposes `backgroundTerminals/{list,terminate,clean}` RPCs — analogous to Kivio's `bash_output`(list) / `kill_background`.

## Caveats / Not found

- No dev-server auto-detection heuristic in Codex (searched; absent by design — the session model is generic).
- No command-dedup/idempotency (confirmed absent by grep).
- Windows process-**tree** kill (`taskkill /T`) is absent from the pipe path; Codex Windows kill is single-pid `TerminateProcess`. Kivio's existing `taskkill /T /F` is actually more robust here.
- Remote "exec-server" backend (`ProcessHandle::ExecServer`, `process.rs:70-72`, `codex_exec_server`) is a parallel transport for cloud/remote environments; not relevant to a local desktop app and not deep-dived here.
- Windows sandbox (`windows-sandbox-rs`, restricted-token / elevated / private-desktop) is a large subsystem only lightly sampled for the creation-flags evidence; if Kivio ever wants OS-level sandboxing on Windows it is worth a dedicated pass.
