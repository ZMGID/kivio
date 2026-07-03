# Research: How Gemini CLI manages background / long-running processes

- **Query**: How does Google's Gemini CLI start/poll/stop background processes (esp. dev servers), suppress Windows consoles, expose running-state to the LLM, dedup launches, handle dev servers?
- **Scope**: external (cloned repo)
- **Date**: 2026-07-03
- **Source**: `github.com/google-gemini/gemini-cli` @ `f7af4e5` (2026-07-02). Cloned shallow to a scratch dir; all citations are `packages/core/src/...`.

## Key files

| File | Role |
|---|---|
| `packages/core/src/tools/shell.ts` | The `run_shell_command` tool: `is_background`/`delay_ms` params, background hand-off logic, POSIX bg-PID capture trap, LLM-facing result strings |
| `packages/core/src/services/shellExecutionService.ts` | Core executor: PTY vs child_process spawn, `ShellExecutionService.background()`, per-session background-process **history registry**, per-PID log files |
| `packages/core/src/services/executionLifecycleService.ts` | `ExecutionLifecycleService`: owns the backgrounding lifecycle, `completionBehavior` (`inject`/`notify`/`silent`), completion re-injection into the model conversation |
| `packages/core/src/tools/shellBackgroundTools.ts` | Two extra tools exposed to the model: `list_background_processes`, `read_background_output` |
| `packages/core/src/utils/process-utils.ts` | `killProcessGroup` — cross-platform tree kill (Windows `taskkill /f /t`, Unix pgroup + pgrep tree walk + SIGTERM→SIGKILL) |
| `packages/core/src/tools/definitions/dynamic-declaration-helpers.ts` | Model-facing schema + description text for `run_shell_command` (`is_background`, `delay_ms`) |
| `packages/core/src/utils/getPty.ts` | PTY implementation resolver (`@lydell/node-pty` → `node-pty` → null=child_process) |

---

## Q1. Background / long-running process model (start → poll → stop)

**There IS an explicit background mode.** The model sets `is_background: true` on the `run_shell_command` tool (no `&` parsing — `&` is explicitly discouraged; see Q5). Params defined at `shell.ts:86-93`:

```ts
export interface ShellToolParams {
  command: string;
  description?: string;
  dir_path?: string;
  is_background?: boolean;
  delay_ms?: number;
  ...
}
```

**Start / hand-off flow** (`shell.ts:690-723`): every command is spawned normally first; if `is_background`, the tool waits a short delay (`delay_ms ?? BACKGROUND_DELAY_MS=200`, `shell.ts:65,707`) to catch immediate crashes, then calls `ShellExecutionService.background(pid, sessionId, cmd)` and, if the process is still running, returns early:

```ts
// shell.ts:697-721
if (this.params.is_background) {
  resultPromise.then(() => { completed = true; }).catch(() => { completed = true; });
  const delay = this.params.delay_ms ?? BACKGROUND_DELAY_MS;
  setTimeout(() => {
    ShellExecutionService.background(pid, sessionId, strippedCommand);
  }, delay);
  await new Promise((resolve) => setTimeout(resolve, delay));
  if (!completed) {
    return { llmContent: `Command is running in background. PID: ${pid}. Initial output:\n${cumulativeOutput}`,
             returnDisplay: `Background process started with PID ${pid}.` };
  }
}
```

**Registry of running processes.** `ShellExecutionService` keeps several static maps (`shellExecutionService.ts:289-296`):

```ts
private static activePtys = new Map<number, ActivePty>();
private static activeChildProcesses = new Map<number, ActiveChildProcess>();
private static backgroundLogPids = new Set<number>();
private static backgroundLogStreams = new Map<number, fs.WriteStream>();
private static backgroundProcessHistory = new Map<string /*sessionId*/, Map<number, BackgroundProcessRecord>>();
```

`background()` (`shellExecutionService.ts:1418-1489`) registers the PID into the **per-session** `backgroundProcessHistory` (capped at `MAX_BACKGROUND_PROCESS_HISTORY_SIZE = 100`, oldest evicted), opens a per-PID append log stream at `<globalTempDir>/background-processes/background-<pid>.log` (`getLogFilePath`, `shellExecutionService.ts:319-321`; dir created `mode: 0o700`, `wx` flag), flushes any already-captured buffer into the log, adds the pid to `backgroundLogPids`, then calls `ExecutionLifecycleService.background(pid)` which resolves the execution promise with `backgrounded: true` (`executionLifecycleService.ts:477-519`). Live output continues to be tee'd to the log via `syncBackgroundLog` on every data chunk (`shellExecutionService.ts:712-717`, `1201-1203`).

**Poll output.** Two dedicated model tools in `shellBackgroundTools.ts`:
- `list_background_processes` (`shellBackgroundTools.ts:71-105`) — lists this session's history: `- [PID x] RUNNING/EXITED: \`cmd\` (Exit Code: n)` (`:56-61`). Session-scoped via `getSessionId()`.
- `read_background_output` (`shellBackgroundTools.ts:249-301`) — tails the per-PID log file. Params `pid`, `lines` (default `DEFAULT_TAIL_LINES_COUNT=100`), `delay_ms` (optional pre-read sleep to let output accumulate, `:136-138`). Verifies PID belongs to the session before reading (`:141-153`); reads at most `MAX_BUFFER_LOAD_CAP_BYTES=64KB` from the tail (`:180-184`); opens with `O_NOFOLLOW` and rejects symlinks (`ELOOP`, `:223-234`).

**Stop.** `ShellExecutionService.kill(pid)` (`shellExecutionService.ts:1405-1410`) → closes log stream, removes from maps, `ExecutionLifecycleService.kill` → the registered `kill` callback → `killProcessGroup` (see Q's below). No process is auto-killed on turn end for backgrounded jobs; they persist in the session registry until killed or the process exits (exit is recorded back into history via the exit handler, `shellExecutionService.ts:774-783` / `1250-1259`).

**Non-background long output**: even foreground commands capture POSIX background child PIDs via an `EXIT` trap wrapper `jobs -p > $_bgpids_file` (`shell.ts:121-138`, invoked `:505-509`); those PIDs are surfaced to the model as `Background PIDs: ...` and the leader as `Process Group PGID: ...` (`shell.ts:822-827`).

---

## Q2. Windows console-window suppression

Two distinct spawn paths, both avoid popup consoles:

1. **Interactive/main path uses a PTY, not `cmd`+window.** `executeWithPty` (`shellExecutionService.ts:915-968`) spawns via `@lydell/node-pty` / `node-pty` (resolved in `getPty.ts:20-45`). On Windows it explicitly requests **ConPTY** (no visible console window):

```ts
// shellExecutionService.ts:950-968
const isWindowsPlatform = os.platform() === 'win32';
const ptyProcess = ptyInfo.module.spawn(finalExecutable, finalArgs, {
  cwd: finalCwd, name: 'xterm-256color', cols, rows, env: finalEnv,
  handleFlowControl: !isWindowsPlatform,
  ...(isWindowsPlatform ? { useConpty: true } : {}),  // force ConPTY, not WinPTY
});
```

2. **child_process fallback** (`childProcessFallback`, `shellExecutionService.ts:547-588`) spawns with `stdio: pipe`, `shell: false`, and — notably — **does NOT set `windowsHide: true`** here; instead it relies on `windowsVerbatimArguments: isWindows ? false : undefined` and `detached: !isWindows && !isBun` (detached only on Unix, for process-group semantics):

```ts
// shellExecutionService.ts:581-588
const child = cpSpawn(finalExecutable, finalArgs, {
  cwd: finalCwd,
  stdio: ['ignore', 'pipe', 'pipe'],
  windowsVerbatimArguments: isWindows ? false : undefined,
  shell: false,
  detached: !isWindows && !isBun,
  env: finalEnv,
});
```

3. **The one place `windowsHide: true` is explicitly used** is the internal `spawnAsync` helper (`shell-utils.ts:973-978`), used for auxiliary commands like `taskkill`/`pgrep`, with the comment *"ensure we don't open a window on windows if possible/relevant"*. Also `agents/auth-provider/value-resolver.ts:63`. So: **PTY path relies on ConPTY to avoid a console; the direct child_process path relies on `shell:false` + piped stdio (no explicit `windowsHide`); `windowsHide:true` is only on the internal utility spawns.**

UTF-8/codepage handling for Windows ConPTY: `injectUtf8CodepageForPty` prefixes `chcp 65001` (`shellExecutionService.ts:100-116`).

Search summary: `grep windowsHide|detached|useConpty|windowsVerbatimArguments` → hits only at `shellExecutionService.ts:584,586,967`, `shell-utils.ts:977`, `secure-browser-launcher.ts:104`, `value-resolver.ts:63`.

---

## Q3. Readiness / running-state legibility to the LLM

**No "server ready" / URL / "listening" detection exists.** Grepping `shell.ts`, `shellBackgroundTools.ts`, `shellExecutionService.ts` for `ready|listening|readiness|dev server|port detect` returns **nothing**. The model is NOT told when a server is up vs starting vs crashed via port/URL heuristics — it must infer from raw log text.

What the model actually receives:

1. **On backgrounding** — a static string, no state detection (`shell.ts:791-797`):
```ts
} else if (this.params.is_background || result.backgrounded) {
  llmContent = `Command moved to background (PID: ${result.pid}). Output hidden. Press Ctrl+B to view.`;
  data = { pid: result.pid, command: this.params.command, initialOutput: result.output };
}
```
(Early-return variant at `:718` includes `Initial output:` — the first ~200ms of stdout, which is how the model sees a startup banner / an immediate crash.)

2. **Up vs starting vs crashed** — determined only by (a) whether the 200ms window elapsed without the process exiting (`completed` flag, `shell.ts:697-716`), and (b) polling `list_background_processes` (RUNNING vs EXITED + exit code) and `read_background_output` (tail of the log). There is no structured "ready" signal.

3. **Background-process completion IS fed back to the model automatically** — the notable mechanism. `ExecutionLifecycleService` supports `completionBehavior: 'inject' | 'notify' | 'silent'` (`executionLifecycleService.ts:86-95`). When a backgrounded execution settles, `settleExecution` (`executionLifecycleService.ts:376-421`) formats output and **injects it directly into the model conversation** via an `InjectionService` (no UI round-trip):

```ts
// executionLifecycleService.ts:386-403
if (execution.backgrounded && !result.aborted) {
  const behavior = execution.completionBehavior ?? (execution.formatInjection ? 'inject' : 'silent');
  const rawInjection = behavior !== 'silent' && execution.formatInjection
      ? execution.formatInjection(result.output, result.error) : null;
  const injectionText = rawInjection ? sanitizeOutput(rawInjection) : null;
  if (injectionText && this.injectionService) {
    this.injectionService.addInjection(injectionText, 'background_completion');
  }
  ...
}
```

The injected text is built by `formatShellBackgroundCompletion` (`shellExecutionService.ts:302-317`):
```ts
if (behavior === 'inject') {
  const truncated = truncateString(output, 5000);
  return `[Background command completed ${status}. Output saved to ${logPath}]\n\n${truncated}`;
}
return `[Background command completed ${status}. Output saved to ${logPath}]`;  // notify
```
Behaviors: `inject` = full (≤5000 char) output injected + task auto-dismisses; `notify` = short pointer to the log file injected; `silent` = nothing injected, task stays in UI. Default is `silent` (`config.ts:1271`, `getShellBackgroundCompletionBehavior` `config.ts:3722-3723`, configurable via `settings.shellBackgroundCompletionBehavior`).

So the "notification back to the model" fires **on completion/crash**, not on readiness. A dev server that stays up produces no injection until it dies.

The model-facing schema description also documents this (`dynamic-declaration-helpers.ts:113-117`):
> "Set to true if this command should be run in the background (e.g. for long-running servers or watchers). The command will be started, allowed to run for a brief moment to check for immediate errors, and then moved to the background."

---

## Q4. Duplicate-launch prevention / idempotency

**None found for background/dev-server launches.** `grep -niE "dedup|duplicate|idempoten|already running"` across `packages/core/src` returns matches only in unrelated subsystems (browser agent MCP connection dedup, agent-registry name-collision, config init dedup) — nothing in `shell.ts` / `shellExecutionService.ts` / `shellBackgroundTools.ts`. There is no check for "this command / this port is already running a background process." Each `run_shell_command(is_background:true)` spawns a fresh process and appends a new PID to the session history unconditionally (`shellExecutionService.ts:1456-1461`). Idempotency is left entirely to the model's judgment (aided by `list_background_processes`).

---

## Q5. Dev-server-specific handling

- **No port detection, no URL surfacing, no auto-background heuristic.** No code inspects output for ports/URLs; the model must choose `is_background` itself. (Contrast with Kivio, which auto-enables background for `npm run dev`/`vite`.)
- **Dev servers are the stated use case for the manual `is_background` flag** — description text: *"e.g. for long-running servers or watchers"* (`dynamic-declaration-helpers.ts:116`).
- **`&` is explicitly discouraged in favor of the flag** (`dynamic-declaration-helpers.ts:60-68`):
  - Windows (interactive): *"To run a command in the background, set the `is_background` parameter to true. Do NOT use PowerShell background constructs."*
  - Bash (interactive): *"...set the `is_background` parameter to true. Do NOT use `&` to background commands."*
  - When interactive shell is disabled, it instead tells the model it *may* use `&` / `Start-Process -NoNewWindow` / `Start-Job`.
- Output volume guidance nudges toward quiet/no-pager flags (`dynamic-declaration-helpers.ts:44-45`) and `PAGER=cat`/`GIT_PAGER=cat` env is forced (`shellExecutionService.ts:489-490`) so long-running commands don't hang on a pager.
- An **inactivity timeout** guards long-running foreground commands: `getShellToolInactivityTimeout()`; the timeout resets on every output event (`shell.ts:491,582-598,605`) — but this does not apply once a command is backgrounded (`is_background` skips the live-flush path).

---

## Takeaways for Kivio

Kivio's current model (per CLAUDE.md): `run_command` with `background:true` (auto-enabled for `npm run dev`/`vite`), per-job temp log `kivio-bgcmd-<job_id>.log`, `AppState.background_commands` registry, `bash_output`/`kill_background` tools, `kill_process_group` (Unix killpg SIGTERM→SIGKILL / Windows `taskkill /T /F`), survives across turns, app-exit sweep. Comparison:

1. **Very similar architecture, converged design.** Both: explicit background flag, short "settle" delay to catch immediate crashes, per-job log file, in-memory registry, poll tool (Kivio `bash_output` ≈ Gemini `read_background_output`+`list_background_processes`), cross-platform tree kill (identical `taskkill /f /t` on Windows). Kivio folded list-into-`bash_output`; Gemini keeps them as two tools. Kivio's design is validated by Gemini's.

2. **Session-scoped registry + 100-entry cap + oldest-eviction** (`shellExecutionService.ts:1434-1454`) and **security checks on the poll tool** (session-ownership check + `O_NOFOLLOW` symlink rejection + 64KB tail cap, `shellBackgroundTools.ts:141-184`) are worth mirroring in Kivio's `bash_output` if not already present.

3. **The completion-injection mechanism is the most interesting idea.** Gemini can push a backgrounded job's terminal output back into the model conversation *automatically on exit* via `InjectionService` + `completionBehavior` (`inject`/`notify`/`silent`, default `silent`; `executionLifecycleService.ts:376-421`, `shellExecutionService.ts:302-317`). This is a cleaner alternative to the model having to poll — and directly addresses "how does the model learn a dev server crashed." Kivio (which explicitly removed dispatch-and-poll for sub-agents) could consider a similar opt-in "notify on background-command exit" injection rather than relying solely on model-initiated `bash_output` polls.

4. **Windows console suppression: two viable strategies.** Gemini's *primary* path is a **ConPTY** (`useConpty: true`, `shellExecutionService.ts:967`), not a plain hidden child. Its child_process fallback uses `shell:false` + piped stdio (no `windowsHide` there), and reserves `windowsHide:true` for utility spawns (`shell-utils.ts:977`). If Kivio's Rust `run_command` on Windows ever shows a console flash, the equivalents are `CREATE_NO_WINDOW` / `windowsHide` on the spawn — Gemini demonstrates that piped-stdio + no shell is usually enough, with ConPTY for interactive cases.

5. **No dev-server intelligence in Gemini** — no port/URL/readiness detection, no duplicate-launch dedup. Kivio's *auto-enable background for `npm run dev`/`vite`* heuristic is actually MORE proactive than Gemini (which leaves `is_background` entirely to the model). If Kivio wants readiness/URL surfacing or dedup, there is **no prior art to copy from Gemini** — it would be net-new. Gemini's stance: keep the tool dumb, let the model poll + reason.

6. **`&` is banned in favor of the structured flag** — Gemini's prompt text explicitly tells the model *"Do NOT use `&`"* when the structured background mode is available (`dynamic-declaration-helpers.ts:61,66`). Kivio may want the same explicit instruction to steer the model to `background:true` over shell backgrounding operators.

## Caveats

- Repo pinned at `f7af4e5` (2026-07-02, shallow clone); line numbers are exact for that commit.
- Injection/`completionBehavior` wiring depends on an `InjectionService` being registered (`ExecutionLifecycleService.setInjectionService`); default behavior is `silent` so injection is opt-in via `settings.shellBackgroundCompletionBehavior`.
- I did not trace the CLI/UI layer (`packages/cli`) beyond the core service; "Press Ctrl+B to view" and background-task UI cards live there and were out of scope for the tool/executor questions.
