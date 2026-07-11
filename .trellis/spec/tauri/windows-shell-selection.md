# Windows run_command Shell Selection (Git Bash First)

> **Purpose**: Contract for which shell `run_command` uses on Windows and how the model is told about it.
> **Origin**: Task `07-10-plugin-runtime-robustness` — skills are written in bash (heredoc, `$VAR`, pipes, `seq`); under PowerShell they were syntax errors, forcing the model into slow detours. Survey of pi/opencode/codex in that task's `research/shell-execution-survey.md`.

---

## Scenario: executing `run_command` on Windows

### 1. Scope / Trigger

`native_tools/shell.rs` — both foreground `run_shell_command` and background `run_shell_command_background` (they share `build_shell_command`). Touch this file → re-read this spec.

### 2. Signatures

```rust
// shell.rs (#[cfg(target_os = "windows")])
pub fn find_git_bash() -> Option<PathBuf>            // cloned from OnceLock process-level cache
fn is_wsl_bash_path(path: &str) -> bool               // rejects \Windows\System32|sysnative\bash.exe
pub fn run_command_shell_hint() -> &'static str       // "" unless Git Bash selected
```

### 3. Contracts

- **Detection order** (pi's order, from research): `%ProgramFiles%\Git\bin\bash.exe` → `%ProgramFiles(x86)%\...` → `%LocalAppData%\Programs\Git\bin\bash.exe` → `where.exe bash.exe` first line **re-verified with `is_file()`** (`where` returns ghost paths) and filtered through `is_wsl_bash_path`.
- **WSL bash is rejected, not adapted**: `\Windows\System32\bash.exe` / `sysnative` sees the `/mnt/c` filesystem view — Windows paths passed by Kivio break silently. (pi feeds it via stdin instead; we deliberately don't.)
- **Execution**: Git Bash found → `bash.exe -c <command>` with the whole command as ONE `.arg()` (never `raw_arg` — same rule as the PowerShell path, see memory `windows-run-command-powershell`). Not found → the pre-existing PowerShell invocation byte-for-byte (`build_windows_powershell_command`): pwsh→powershell, `-NoLogo -NoProfile -NonInteractive -Command`, UTF-8 wrap. **PowerShell fallback must never be removed** — GUI users without Git must keep working.
- **Shell choice must be visible to the model** (opencode issue #16479 — invisible selection ⇒ silent command corruption): `mcp/types.rs::native_run_command_tool` splices `run_command_shell_hint()` into the tool description. The hint tells the model: bash syntax, and **Windows paths with forward slashes** (`C:/Users/...`) because backslashes are bash escapes (opencode #15810). The system-prompt sentence in `chat/agent/prepare.rs` derives from the SAME probe — prompt and tool description can never disagree.
- **Never rewrite command content silently** (opencode's core failure class).
- Cache is process-level: installing/uninstalling Git requires app restart (documented in code comment).
- `kill_process_group` (taskkill `/T /F`) is shell-agnostic — untouched.

### 4. Validation & Error Matrix

| Condition | Behavior |
|---|---|
| Git Bash at known path | bash -c, hint active |
| Only WSL bash present | Rejected → PowerShell fallback |
| `where` returns ghost path | `is_file()` re-check → skipped |
| No bash at all | PowerShell, description unchanged from legacy |
| PowerShell-syntax command under bash | Fails visibly; model self-corrects from dynamic description |

### 5. Tests Required

`shell.rs` test module: `is_wsl_bash_path` matrix (System32/sysnative, both slash forms, case-insensitive; must NOT misfire on `D:\tools\System32\bash.exe` or msys64); real bash-syntax execution test (heredoc + `seq` + pipe) that self-skips when Git Bash absent; the three PowerShell regression tests must target `build_windows_powershell_command` explicitly (otherwise they silently start testing bash on dev machines). Run via `scripts/win-cargo-test.ps1`. When asserting a resolved workspace cwd on Windows, compare against `std::fs::canonicalize(root)`, not the lexical input path: canonicalization may add the `\\?\` prefix.

### 6. Wrong vs Correct

#### Wrong
```rust
// Hardcode PowerShell (codex approach — model writes bash, commands corrupt)
// or: auto-detect shell but keep a static tool description (opencode approach)
```

#### Correct
```rust
// Detect Git Bash with fallback, and make the SELECTED shell visible to the
// model in both the tool description and the system prompt, from one probe.
```
