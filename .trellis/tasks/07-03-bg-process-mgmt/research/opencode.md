# Research: How OpenCode manages long-running / background processes

- **Query**: How does the OpenCode coding agent start/poll/stop background processes (esp. dev servers), suppress Windows consoles, expose running-state to the LLM, prevent duplicate launches, and handle dev servers?
- **Scope**: external (cloned repo)
- **Date**: 2026-07-03
- **Source**: `github.com/sst/opencode` shallow clone, commit `a4fed69a826d72b7bb3280fab0460cc7aa698023` (2026-07-03 10:31 UTC), TypeScript. This is the current **V2 rewrite** on `main`.

## Headline finding (read first)

In the current V2 codebase, **the LLM-facing shell tool (`bash`) has NO background mode at all.** It runs one command synchronously, capped by a timeout, and returns combined output + exit code. Background-shell support was **deliberately removed** during the V2 rewrite; the intent to re-add it (with a proper owner-bound get/wait/cancel design) is documented as TODOs in the tool source.

There *is* a generic `BackgroundJob` registry, but it is wired only to **sub-agent "task" execution** and the session runner — **not** to shell commands. There is no readiness detection, no port detection, no dev-server heuristics, and no duplicate-launch guard for shell commands anywhere.

Everything below is evidence for that picture.

---

## Q1. Background / long-running process model

### The bash tool is synchronous-only, timeout-bounded

`packages/core/src/tool/bash.ts`

- Input schema has only `command`, `workdir`, `timeout` — **no `background` field** (lines 23-33).
- Timeouts: `DEFAULT_TIMEOUT_MS = 2*60*1000` (2 min), `MAX_TIMEOUT_MS = 10*60*1000` (10 min), output cap `MAX_CAPTURE_BYTES = 1MB` (lines 19-21).
- Spawns one child and awaits full completion via `appProcess.run(...)` with `combineOutput: true`, `timeout`, `maxOutputBytes` (lines 154-172):

```ts
// bash.ts:154-167
const command = ChildProcess.make(input.command, [], {
  cwd: target.canonical,
  shell,
  stdin: "ignore",
  detached: process.platform !== "win32",
  forceKillAfter: Duration.seconds(3),
})
const timeout = input.timeout ?? DEFAULT_TIMEOUT_MS
const result = yield* appProcess
  .run(command, { combineOutput: true, timeout: Duration.millis(timeout), maxOutputBytes: MAX_CAPTURE_BYTES })
```

- On timeout it returns a text result telling the model to retry with a bigger timeout — the process is killed, not backgrounded (lines 173-180):

```ts
// bash.ts:174-179
return {
  output: `Command exceeded timeout of ${timeout} ms. Retry with a larger timeout if the command is expected to take longer.`,
  truncated: false,
  timeout: true,
  ...
}
```

### The removal is explicit in the source

`packages/core/src/tool/bash.ts:62-78` — a block of TODOs documenting that the legacy background/long-running-process machinery was stripped and the conditions for bringing it back:

```ts
// bash.ts:71-77
// TODO: Add durable/live progress metadata streaming for long-running commands once V2 tool invocation progress context is wired.
// TODO: Persist background job status and define restart recovery before exposing remote observation.
// TODO: Re-add model-facing background launch only with owner-bound get/wait/cancel tools and completion delivery.
// TODO: Add HTTP background-job observation only after durable status, restart recovery, and authorization are defined.
// TODO: Revisit process-group cleanup and platform coverage with shell-specific tests if current AppProcess semantics do not fully cover it.
```

So: there is **no separate "run in background" vs foreground path** for shell in V2, and **no `bash_output`/poll tool** for shell. (grep for `BashOutput`, `background:`/`runInBackground` in shell/tool code returns nothing.)

### There IS a job registry — but it's for sub-agent tasks, not shell

`packages/core/src/background-job.ts` — a full process-local job registry keyed by id, service id `@opencode/BackgroundJob`:

- `Info` shape: `{ id, type, title?, status: "running"|"completed"|"error"|"cancelled", started_at, completed_at?, output?, error?, metadata? }` (lines 7-19).
- Interface: `list / get / start / extend / wait / waitForPromotion / promote / cancel` (lines 88-97).
- `start(input)` runs an `Effect` (`input.run`) in a forked scope, tracks status, and settles to completed/error/cancelled (lines 202-254, `settle` lines 126-171).
- `wait({ id, timeout })` blocks on a `Deferred` until the job finishes or times out (lines 292-301) — this is the "poll/await output" mechanism, but callers wait on the *Effect result*, not on incremental stdout.
- **Explicitly non-durable / process-local** (class doc, lines 113-119):

```ts
// background-job.ts:114-118
// Entries are intentionally not durable: process restart or owner-scope closure
// loses status and interrupts live work. Persisted observation, restart recovery,
// and remote workers need a separate durable ownership slice ...
```

**Who consumes it** (grep for `BackgroundJob.Service`, non-test):
- `packages/opencode/src/tool/task.ts:85` — the **sub-agent/task tool** (see Q4).
- `packages/opencode/src/session/session.ts:497` and `session/run-state.ts:32` — session lifecycle / cascade cancellation.
- `packages/opencode/src/server/routes/instance/httpapi/handlers/experimental.ts:36` — an experimental HTTP endpoint.
- **No shell/bash file references it.**

### Stop / kill

Process-group kill lives in the spawner and a helper:
- `packages/core/src/shell.ts:31-60` `killTree(proc)` — Windows `taskkill /pid <pid> /f /t` (windowsHide), POSIX `process.kill(-pid, SIGTERM)` then `SIGKILL` after 200ms.
- `packages/core/src/cross-spawn-spawner.ts:292-312` `killGroup` — same split (Windows `taskkill /pid <pid> /T /F`, POSIX `process.kill(-pid, signal)`), plus `forceKillAfter` escalation SIGTERM→SIGKILL (lines 324-343, 395-401).
- The bash tool sets `forceKillAfter: Duration.seconds(3)` (bash.ts:159).

---

## Q2. Windows console-window suppression

Handled centrally in the spawner, **not** in the bash tool.

`packages/core/src/cross-spawn-spawner.ts`

- Every child spawn sets `windowsHide` on Windows and disables `detached` on Windows (lines 373-381):

```ts
// cross-spawn-spawner.ts:374-381
spawn(command, {
  cwd: dir,
  env: env(command.options),
  stdio: stdios(sin, sout, serr, extra),
  detached: command.options.detached ?? process.platform !== "win32",  // false on win32
  shell: command.options.shell,
  windowsHide: process.platform === "win32",   // <-- console suppression
}),
```

- Actual spawn uses the `cross-spawn` package (`import launch from "cross-spawn"`, line 26; `const proc = launch(...)`, line 270).
- Kill paths also pass `windowsHide: true`:
  - `cross-spawn-spawner.ts:299` — `NodeChildProcess.exec(\`taskkill /pid ${proc.pid} /T /F\`, { windowsHide: true }, ...)`.
  - `shell.ts:37-40` — `spawn("taskkill", [...], { stdio: "ignore", windowsHide: true })`.

Notes:
- The bash tool itself sets `detached: process.platform !== "win32"` (bash.ts:158) — i.e. detached process group on POSIX (for group-kill), non-detached on Windows.
- **No** use of `CREATE_NO_WINDOW` / `0x08000000` / `DETACHED_PROCESS` / raw `creationFlags` — it relies entirely on Node's `windowsHide` option (grep for those flags returns nothing in src).

---

## Q3. Readiness / running-state legibility to the LLM

**There is no readiness detection of any kind for shell processes.** grep across `core/src` + `opencode/src` for `listening|server ready|compiled|Local:` finds only unrelated hits (a `serve.ts` log line, effect-runtime `ready` latches, LSP readiness) — **nothing that scans child-process stdout for "ready"/"listening"/"Local:"/"compiled".**

What the model actually gets back from `bash` (`bash.ts:114-192`):
- `toModelOutput` returns two text parts: the raw combined output, then a status line (lines 114-117).
- Status line (`modelOutput`, lines 51-57): either `Command timed out before completion.` or `Command exited with code <n>.`
- Structured output surfaced to the model: `{ exit?, truncated, timeout? }` (lines 35-39, 109-113).

So the model distinguishes states only by:
- **exit code** (crashed vs clean) — only available *after* the process ends,
- a **`timeout: true`** flag when the command outran its timeout (bash.ts:55, 177),
- an output-truncation notice at the 1MB cap (bash.ts:183-185).

There is **no grace period that returns initial output while leaving the process running** — a long-running server either finishes within the timeout, or is killed and reported as timed out. "Still starting vs up vs crashed" is not modeled. (The bash.ts TODO at line 71 flags "durable/live progress metadata streaming for long-running commands" as future work.)

For the *task/sub-agent* path, `BackgroundJob.wait({ timeout })` (background-job.ts:292-301) returns `{ info, timedOut }` — the only place a caller can get a "still running after N ms" answer — but again this is for sub-agents, not shell, and it returns the job's Effect result, not streamed process output.

---

## Q4. Duplicate-launch prevention / idempotency

**For shell: none.** The bash tool has no dedup by command/cwd/port; each call spawns unconditionally.

**For the BackgroundJob registry (sub-agent tasks): idempotent by job id only.** `start()` no-ops if a job with the same id is already running:

```ts
// background-job.ts:213-216
const existing = jobs.get(id)
if (existing?.info.status === "running") {
  return [{ info: snapshot(existing) }, jobs] as readonly [StartResult, Map<string, Active>]
}
```

The task tool passes an explicit id / `task_id` for resume (`packages/opencode/src/tool/task.ts:97, 121-123`), and background sub-agents are gated behind an experimental flag:

```ts
// task.ts:97-102
const runInBackground = params.background === true
if (runInBackground && !flags.experimentalBackgroundSubagents) {
  return yield* Effect.fail(new Error("Background subagents require OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS=true"))
}
```

No dedup by command, cwd, or port exists anywhere.

---

## Q5. Dev-server-specific handling

**None in the shell/tool layer.** No auto-background heuristic (e.g. detecting `npm run dev`/`vite`), no port detection, no URL surfacing. grep for `detectPort`, `localhost:`, `127.0.0.1:`, `port.*listen` in `tool/`, `shell.ts`, `background-job.ts` returns nothing. A dev server started via `bash` will simply run until it hits the (max 10-minute) timeout and then be killed.

There is a separate **PTY subsystem** (`packages/core/src/pty.ts`, `pty/pty.bun.ts` using `bun-pty`), but it is a **user-facing interactive terminal** feature: it buffers output (2MB), retains exited sessions (limit 25), and is attached over a websocket protocol (`pty/protocol.ts`) for the TUI/desktop terminal panel. It is registered as a location service (`location-services.ts:23,59`), **not** exposed as an LLM tool — so it is not how the agent runs dev servers either.

(The `onBackground` callbacks in `packages/opencode/src/cli/cmd/run/*` are the TUI "send this session to the background" UX — unrelated to shell processes.)

---

## Takeaways for Kivio

Context: Kivio *already* has a richer model-facing background-command system than OpenCode currently ships — `run_command background:true` + `bash_output` polling + `kill_background`, with a per-job temp-log registry in `AppState.background_commands`, cross-turn survival, and auto-background heuristics for dev servers. OpenCode is therefore a cautionary/contrast data point more than a template to copy.

1. **OpenCode deliberately REMOVED model-facing background shell in its V2 rewrite.** The bash tool is now synchronous-with-timeout only. Their stated reason (bash.ts:71-77 TODOs): they will only re-add it once they have *durable status, restart recovery, owner-bound get/wait/cancel tools, and completion delivery*. This validates that a naive in-memory background registry (which is what both they had and Kivio has) is considered a liability worth cutting — worth weighing against Kivio's current "results lost on restart" registry.

2. **Their generic job registry is process-local and non-durable by design** (background-job.ts:113-119) and is scoped to **sub-agent tasks**, not shell — the opposite split from Kivio, where sub-agents are blocking and *shell* commands are the thing that survives across turns. If Kivio ever unifies these, OpenCode's `BackgroundJob` interface (`list/get/start/wait/extend/promote/cancel` + `wait({timeout})` returning `{info, timedOut}`) is a clean reference shape.

3. **Windows console suppression is trivial in their Node stack:** just `windowsHide: true` on `spawn`, plus `taskkill /T /F` with `windowsHide: true` for kills, `detached:false` on Windows (cross-spawn-spawner.ts:374-381, 297-303; shell.ts:35-45). They use **no** raw `CREATE_NO_WINDOW`/`DETACHED_PROCESS` flags. Kivio's Rust equivalent is `CREATE_NO_WINDOW (0x08000000)` on `Command`; the takeaway is that suppression is a per-spawn concern centralized in one spawner, and the kill path needs it too (their `taskkill` is the analog of Kivio's `taskkill /T /F`).

4. **No readiness / "server ready" detection exists** — OpenCode does not scan stdout for `ready`/`listening`/`Local:`/`compiled`, and has no grace-period "return initial output, keep running" behavior for shell. The model only learns state from exit code + a `timeout` flag. This is a gap Kivio's `bash_output` polling already fills better; there is nothing to borrow here, only confirmation that readiness detection is a genuinely unsolved/unimplemented piece even in a mature agent.

5. **No duplicate-launch / port dedup for shell** in OpenCode (only id-based idempotency on the task registry, background-job.ts:213-216). If Kivio wants dedup-by-command/cwd/port, it would be inventing it, not porting it.

### Caveats / Not found
- This is a shallow clone of `main` at commit `a4fed69`; **git history is unavailable**, so the *previous* (pre-V2) background-bash implementation could not be inspected directly. Its prior existence is inferred from the removal TODOs in `bash.ts:62-77` and the retained `BackgroundJob` registry.
- Searches performed that returned nothing relevant: `background:`/`runInBackground`/`BashOutput` in shell/tool code; `CREATE_NO_WINDOW`/`0x08000000`/`DETACHED_PROCESS`/`creationFlags`; `listening`/`server ready`/`compiled`/`Local:` for process output; `detectPort`/`localhost:`/port-listen in tool/shell/job files.
