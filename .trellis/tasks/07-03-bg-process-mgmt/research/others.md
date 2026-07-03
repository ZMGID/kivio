# Research: Background / long-running process management in other coding agents

- **Query**: How do Pi, Aider, and Claude Code manage long-running / background processes (esp. dev servers)? Cross-reference for Kivio's `run_command` background model.
- **Scope**: external (git clones + Claude Code public docs)
- **Date**: 2026-07-03
- **Sources studied**:
  - Pi — `earendil-works/pi` (67k★, "AI agent toolkit: unified LLM API, agent loop, TUI, coding agent CLI"), shallow-cloned to `$TMP/kivio-research/pi`. Confirmed identity via GitHub search + the companion repo `badlogic/pi-skills` ("Skills for pi coding agent (compatible with Claude Code and Codex CLI)"). TypeScript/Node monorepo.
  - Aider — `Aider-AI/aider`, shallow-cloned to `$TMP/kivio-research/aider`. Python.
  - Claude Code — closed source; public docs at `https://code.claude.com/docs/en/tools-reference` (the `docs.anthropic.com/en/docs/claude-code` path now redirects here).

The 5 cross-cutting questions per source:
1. Background / long-running process model (start / poll / stop; registry?)
2. Windows console-window suppression (spawn flags / windowsHide / detached)
3. Readiness / running-state legibility to the LLM (ready detection? grace period? up vs starting vs crashed?)
4. Duplicate-launch prevention / idempotency
5. Dev-server-specific handling

---

## 1. Pi (`earendil-works/pi`)

Pi is a Claude-Code-style tool-calling coding agent. Its shell tool is **`bash`** (`packages/coding-agent/src/core/tools/bash.ts`). Process-tree kill/track helpers live in `packages/coding-agent/src/utils/shell.ts`.

### Q1 — Background model: **NONE (fully blocking).**
- The `bash` tool schema exposes only `command` + optional `timeout` (seconds) — there is **no `run_in_background` / background flag** (`bash.ts:40-43`). Repo-wide grep for `run_in_background|is_background|backgroundTask|BashOutput|list_background|kill_background` returns nothing.
- `execute()` does `await ops.exec(...)` and blocks until the child exits: output is streamed via `onData` into an `OutputAccumulator`, then `waitForChildProcess(child)` is awaited for the exit code (`bash.ts:133`, `304-425`). No job registry, no poll tool, no "detach and return a handle" path.
- There is **no default timeout** ("Timeout in seconds (optional, no default timeout)", `bash.ts:42`); a long-running/never-exiting command would block the tool call until the model-supplied timeout fires or the run is aborted. So dev servers are effectively not runnable through the agent's `bash` tool as long-lived processes.
- The only "keep running / interactive" affordances are **user-facing example extensions**, not agent tools: `examples/extensions/interactive-shell.ts` intercepts `!`-prefixed *human* commands (vim, htop, `git rebase -i`) and suspends the TUI — its own header note says *"This only intercepts user `!` commands, not agent bash tool calls. If the agent runs an interactive command, it will fail (which is fine)."*

### Q2 — Windows console suppression: **YES — `windowsHide: true` everywhere; detached only on Unix.**
- `createLocalBashOperations`: `spawn(shell, args, { cwd, detached: process.platform !== "win32", ..., windowsHide: true })` (`bash.ts:97-103`). Detached (own process group) on Unix so the whole tree can be killed via negative-pid signal; **not** detached on Windows.
- `killProcessTree(pid)` (`shell.ts:200-225`): Windows → `spawn("taskkill", ["/F","/T","/PID", pid], { detached:true, windowsHide:true })`; Unix → `process.kill(-pid, "SIGKILL")` (process-group), falling back to `process.kill(pid, "SIGKILL")`. Note: **SIGKILL only, no SIGTERM grace step** on Unix.
- The lower-level agent harness (`packages/agent/src/harness/env/nodejs.ts:125-227`) uses the same pattern (`windowsHide:true`, `taskkill /F /T`, `detached` on non-win32).

### Q3 — Readiness legibility to LLM: **N/A** (no background = no "starting vs up vs crashed" concept). For blocking commands, the model only ever sees the final result: full stdout+stderr merged, exit code appended as `Command exited with code N` on non-zero (`bash.ts:422-424`), or `Command timed out after Ns` / `Command aborted` (`bash.ts:410-416`). Live streaming (`onUpdate`, throttled 100ms — `bash.ts:174-353`) is for the **TUI**, not fed to the model mid-flight.

### Q4 — Duplicate-launch prevention: **N/A** — no registry, so nothing to dedupe.

### Q5 — Dev-server handling: **NONE.** No `npm run dev`/vite detection, no auto-background. A dev server would have to be run by the human via `!` (interactive extension) or would block/time-out as an agent `bash` call.

### Output truncation (relevant detail)
Output is capped to last `DEFAULT_MAX_LINES` lines / `DEFAULT_MAX_BYTES` KB (whichever hits first); overflow is written to a temp file (`pi-bash*`) and the model is told the path + shown the tail (`bash.ts:301`, `375-392`). Same "spill to file" idea Kivio uses for big output.

**Takeaway:** Pi is the closest structural cousin to Kivio's kill logic (identical `taskkill /F /T` + `windowsHide` + Unix process-group), but deliberately has **no background-process feature at all** — everything is blocking.

---

## 2. Aider (`Aider-AI/aider`)

Aider is **not** a tool-calling agent. The LLM emits SEARCH/REPLACE edit blocks; a human (or auto-`test`/`lint` hooks) runs shell commands. Shell execution is `aider/run_cmd.py`; the user commands that call it are in `aider/commands.py`.

### Q1 — Background model: **NONE (fully blocking, synchronous).**
- `run_cmd_subprocess` (`run_cmd.py:42-86`): `subprocess.Popen(command, stdout=PIPE, stderr=STDOUT, shell=True, bufsize=0, ...)` then a `while True: chunk = process.stdout.read(1)` loop printing char-by-char, then **`process.wait()`** and `return process.returncode, output`. Purely blocking; no job handle/registry.
- On interactive TTY + non-Windows, it instead uses **`pexpect`** and literally hands the terminal to the user via `child.interact(...)` (`run_cmd.py:89-128`) — a foreground interactive session, not a background job.
- No `run_in_background`, no poll/kill tools, no background registry anywhere. `/run` (alias `!`) and `/test` are the only shell entry points.

### Q2 — Windows console suppression: **NO explicit suppression.**
- `run_cmd_subprocess` passes **no `creationflags`, no `CREATE_NO_WINDOW`, no `startupinfo`** — just `shell=True` (`run_cmd.py:62-73`). On Windows it detects a PowerShell parent and rewrites the command to `powershell -Command <cmd>` (`run_cmd.py:51-54`). Because Aider itself is a console app run from a terminal, a flashing child console window isn't a concern the way it is for Kivio's GUI (Tauri) parent — this is a meaningful gap for Kivio to NOT copy.
- No `detached`/process-group setup either; no cross-platform tree-kill (relies on `process.wait()` and Ctrl-C).

### Q3 — Readiness legibility: **N/A** (no background). Output goes to the model only after the command finishes.

### Q4 — Duplicate-launch prevention: **N/A** (no registry).

### Q5 — Dev-server handling: **NONE specific.** The dev-adjacent feature is `/test` + auto-lint/test-after-edit: `cmd_test` runs a command and, via `cmd_run(args, add_on_nonzero_exit=True)`, feeds output to the chat **only on non-zero exit** (`commands.py:993-1053`). A dev server (long-lived, no exit) does not fit this run-to-completion model.

### How output is surfaced to the model (`commands.py:1013-1053`)
- After a blocking run, Aider token-counts the combined output, and (for interactive `/run`) **asks the human** `"Add {k}k tokens of command output to the chat?"` before injecting it. For `/test` it auto-adds on non-zero exit.
- Injection format: appends a synthetic turn — `{"role":"user", content: prompts.run_output.format(command, output)}` followed by `{"role":"assistant","content":"Ok."}` (`commands.py:1036-1044`), and on failure sets a placeholder prompt `"What's wrong? Fix"`.

**Takeaway:** Aider has essentially no background-process story; the human drives long-lived processes in their own terminal. Its only relevant pattern for Kivio is "run-to-completion, then optionally inject stdout+stderr+exit-code into the conversation as a user message."

---

## 3. Claude Code (public docs only — `code.claude.com/docs/en/tools-reference`)

Closed source; the tools-reference page documents behavior but **not** internal tool names/schemas.

### Q1 — Background model: **YES, first-class.** Two mechanisms:
- **Bash `run_in_background: true`** — quoting the docs: *"For long-running processes such as dev servers or watch builds, Claude can set `run_in_background: true` to start the command as a background task and continue working while it runs. List and stop background tasks with `/tasks`."* So: start = a Bash flag; list/stop = the `/tasks` slash command. (Internally the model polls a started shell and can terminate it — but the tool-reference page does **not** name a `BashOutput` / `KillShell` / `KillBash` tool; see Gaps below.)
- **`Monitor` tool** (separate top-level tool, permission-required): *"Runs a command in the background and feeds each output line back to Claude, so it can react to log entries, file changes, or polled status mid-conversation. Can also open a WebSocket and treat each incoming message as an event."* Use cases listed: tail a log & flag errors, poll a PR/CI job & report status changes, watch a directory, track output of any long-running script, connect to a WebSocket feed. *"For most watches Claude writes a small script, runs it in the background, and receives each output line as it arrives."* Stop it *"by asking Claude to cancel it or by ending the session."* Monitor reuses **the same allow/deny permission rules as Bash**. Not available on Bedrock/Vertex/Foundry or when telemetry is disabled. Plugins can declare monitors that auto-start.

### Q2 — Windows console suppression: **N/A in docs.** The docs don't discuss spawn flags. Relevant Windows note instead: there's a dedicated **`PowerShell` tool** ("Executes PowerShell commands natively") whose preview limitations are *"PowerShell profiles are not loaded"* and *"On Windows sandboxing is not supported."* Nothing about hiding child console windows.

### Q3 — Readiness / running-state legibility: **Event/line-push model, not an explicit ready-detector.**
- The docs describe no automatic "server is ready" heuristic. Instead the model *reacts* to output: `run_in_background` lets Claude "continue working while it runs" and check back; `Monitor` actively "feeds each output line back to Claude... so it can react... mid-conversation" and "interjects when an event arrives." So readiness is judged by the model reading the streamed log lines, not by a framework state machine (no documented up/starting/crashed enum, no grace-period constant).
- WebSocket source (v2.1.195+) event semantics are precise: each text message = one event; binary frames → placeholder line; messages > 1 MiB → the watch ends; socket close → watch ends and Claude gets the close code.

### Q4 — Duplicate-launch prevention: **Not documented** (no idempotency/dedupe mention).

### Q5 — Dev-server handling: **Explicitly the motivating use case** for `run_in_background` ("dev servers or watch builds") — but the docs describe no special parsing/detection; it's a manual flag the model sets. `Monitor` covers the "tail the server log and react" follow-up.

### Other Bash-tool behavior worth noting (`tools-reference` Bash section)
- Each command runs in a **separate process**; `cd` in the main session carries over to later commands *if it stays inside the project/added dirs* (else reset to project dir, with `Shell cwd was reset to <dir>` appended to the result); `CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR=1` disables carry-over. **Env vars / `export` do NOT persist** across commands; aliases/functions from `~/.zshrc|~/.bashrc|~/.profile` are sourced once at session start.
- **Timeout**: 2 min default, model can request up to 10 min via `timeout` param; overridable with `BASH_DEFAULT_TIMEOUT_MS` / `BASH_MAX_TIMEOUT_MS`.
- **Output length**: 30 000 chars default; overflow saved to a file in the session dir, model gets the path + a short head preview and reads/searches the file for the rest (`BASH_MAX_OUTPUT_LENGTH` to raise). (Same spill-to-file pattern as Pi and Kivio.)

### Gaps / honesty
- The public tools-reference page does **not** document the internal tool names `BashOutput`, `KillShell`, or `KillBash`, nor a poll-by-offset API or a "shell_id" handle. Those are commonly known from Claude Code's tool schemas but are **not confirmable from the docs I fetched** — I'm not asserting them as documented fact. What the docs *do* confirm: `run_in_background: true` (Bash), `/tasks` to list/stop, and the separate `Monitor` tool for line-by-line background output feeding.
- Kill semantics for background bash beyond `/tasks` ("list and stop") are not spelled out in the docs.

---

## Takeaways for Kivio

Context: Kivio's `run_command` already has `background:true` (auto-enabled for `npm run dev`/`vite`), a per-job temp log (`kivio-bgcmd-<job_id>.log`), an `AppState.background_commands` registry, `bash_output` (poll by offset / list) + `kill_background` (cross-platform `kill_process_group`: unix SIGTERM→SIGKILL, Windows `taskkill /T /F`), survival across turns, and app-exit sweep.

1. **Kivio's background model is the most feature-complete of the three** — Aider and Pi have *no* background process feature at all (both fully blocking). Only Claude Code has a peer feature (`run_in_background` + `/tasks` + `Monitor`). So there is no richer open-source reference implementation to copy; Kivio is already ahead of Pi/Aider here.

2. **Windows console suppression — validate against Pi (the one good reference).** Pi consistently uses `windowsHide: true` on every spawn and `taskkill /F /T /PID` for tree kill — matching Kivio's approach. Aider does **not** suppress console windows (it's a console app, so it doesn't need to; Kivio, a GUI/Tauri parent, does). Confirm Kivio's background spawns set the Windows no-window / hide flag; don't take Aider as a model here.

3. **Kill semantics — Kivio's SIGTERM→SIGKILL grace step is better than Pi's.** Pi kills with **SIGKILL only** (`shell.ts:215`), no graceful term. Kivio's documented `kill_process_group` (SIGTERM then SIGKILL) is the more correct pattern; keep it.

4. **Readiness detection is nobody's strong suit — the industry pattern is "let the model read the log," not a state machine.** Claude Code's `run_in_background` + `Monitor` both surface raw output lines and let the *model* judge readiness; there is no documented up/starting/crashed enum or ready-heuristic in any of the three. If Kivio wants explicit "server ready" legibility (Q3), that would be a genuine differentiator, not something to crib — but the low-risk aligned move is to make `bash_output`/status expose enough (exit-yet? recent log tail? still-running flag?) for the model to reason itself.

5. **Duplicate-launch / idempotency is unsolved everywhere.** None of the three documents dedupe or "already-running" detection. Kivio's auto-background-on-`npm run dev` plus a registry gives it the raw material to add a "this dev server is already running (job N)" guard that none of the references have — a clean improvement area, but note it's Kivio-original, not borrowed.

6. **Output spill-to-file is a shared, validated pattern.** Pi (`pi-bash*` temp file, tail preview), Claude Code (session-dir file + head preview + `BASH_MAX_OUTPUT_LENGTH`), and Kivio (`kivio-bgcmd-*.log`) all cap inline output and offload the rest to a file. Kivio is consistent with the field here.

7. **Poll-by-offset (`bash_output` with offset) is a Kivio/Claude-Code-shaped idea, absent in Pi/Aider.** Claude Code feeds incremental lines; Kivio polls by offset. Both avoid re-sending the whole log. Aider/Pi don't have this because they don't background. No cross-reference improvement needed; Kivio's offset polling is sound.
