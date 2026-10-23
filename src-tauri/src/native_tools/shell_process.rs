// Shared platform shell selection and process-tree cancellation.
// Included by both hosts with std::{Path,PathBuf} and tokio::process::Command.
/// Windows: the PowerShell executable to run agent shell commands through.
/// Prefer `pwsh` (PowerShell 7+: UTF-8 by default, supports `&&`) when it is on
/// PATH; otherwise fall back to `powershell` (Windows PowerShell 5.1, always
/// present on Windows). Cached for the process lifetime. Mirrors opencode's
/// `win()` shell precedence (pwsh → powershell → …). We run via PowerShell (not
/// `cmd.exe`) so the model's natural modern one-liners work — `cmd.exe` steers
/// models toward removed/hanging tools like `wmic`.
#[cfg(target_os = "windows")]
fn windows_powershell_exe() -> &'static str {
    use std::sync::OnceLock;
    static SHELL: OnceLock<&'static str> = OnceLock::new();
    SHELL.get_or_init(|| if pwsh_on_path() { "pwsh" } else { "powershell" })
}

/// Whether `pwsh.exe` (PowerShell 7+) is discoverable on PATH. Scans PATH
/// directly to avoid pulling a `which` dependency.
#[cfg(target_os = "windows")]
fn pwsh_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("pwsh.exe").is_file())
    })
}

/// Prefix a PowerShell command so its stdout is emitted as UTF-8. Windows
/// PowerShell 5.1 defaults `[Console]::OutputEncoding` to the OEM code page, so
/// without this CJK/non-ASCII output arrives mojibake'd when we read the pipe as
/// UTF-8. pwsh 7 already defaults to UTF-8, so the prefix is a harmless no-op
/// there. The `try/catch` guards the rare case where setting the encoding throws
/// (no console) — the user command still runs.
#[cfg(target_os = "windows")]
fn wrap_ps_command(command: &str) -> String {
    format!(
        "try {{ [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}; {command}"
    )
}

/// Windows: absolute path to Git Bash's `bash.exe`, if discoverable. Cached for
/// the process lifetime — installing/uninstalling Git for Windows requires a
/// Kivio restart to be picked up, which is an accepted tradeoff (see R4 design
/// notes; existsSync-per-call like pi does is cheap in Node, less clean in Rust).
///
/// Resolution order (pi's `getShellConfig` precedence, minus the hard error):
/// 1. Known install locations, in order: `%ProgramFiles%\Git\bin\bash.exe` →
///    `%ProgramFiles(x86)%\Git\bin\bash.exe` →
///    `%LocalAppData%\Programs\Git\bin\bash.exe`. Missing env vars just skip
///    that probe rather than failing.
/// 2. Fallback: `where.exe bash.exe`, first line only, re-verified with
///    `Path::is_file` — `where` is known to surface stale/ghost PATH entries.
/// 3. Reject anything resolving to the WSL bash shim
///    (`...\Windows\System32\bash.exe` / `...\Windows\sysnative\bash.exe`):
///    its `/mnt/c/...` filesystem view does not match the Windows paths Kivio
///    passes around, unlike pi (which special-cases WSL via stdin), Kivio
///    just excludes it.
///
/// `None` means "no usable Git Bash" — callers fall back to the existing
/// PowerShell path unchanged. Unlike pi/Claude Code, Kivio never hard-errors:
/// Git for Windows is not an install prerequisite.
#[cfg(target_os = "windows")]
pub fn find_git_bash() -> Option<PathBuf> {
    use std::sync::OnceLock;
    static GIT_BASH: OnceLock<Option<PathBuf>> = OnceLock::new();
    GIT_BASH.get_or_init(detect_git_bash).clone()
}

#[cfg(target_os = "windows")]
fn detect_git_bash() -> Option<PathBuf> {
    let known_locations: [(&str, &[&str]); 3] = [
        ("ProgramFiles", &["Git", "bin", "bash.exe"]),
        ("ProgramFiles(x86)", &["Git", "bin", "bash.exe"]),
        ("LocalAppData", &["Programs", "Git", "bin", "bash.exe"]),
    ];
    for (env_var, tail) in known_locations {
        let Some(base) = std::env::var_os(env_var) else {
            continue;
        };
        let mut candidate = PathBuf::from(base);
        candidate.extend(tail.iter().copied());
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    where_bash_exe().filter(|path| path.is_file() && !is_wsl_bash_path(path))
}

/// Run `where.exe bash.exe` and return the first line as a path, unverified —
/// callers must confirm the file actually exists (`where` is known to surface
/// stale PATH entries that no longer resolve to a real file).
#[cfg(target_os = "windows")]
fn where_bash_exe() -> Option<PathBuf> {
    use crate::proc::NoConsoleWindow;
    let mut cmd = std::process::Command::new("where.exe");
    cmd.arg("bash.exe");
    cmd.no_console_window();
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?.trim();
    if first_line.is_empty() {
        return None;
    }
    Some(PathBuf::from(first_line))
}

/// True when `path` is the WSL bash shim (`...\Windows\System32\bash.exe` or
/// `...\Windows\sysnative\bash.exe`), matched case-insensitively. WSL bash's
/// filesystem view (`/mnt/c/...`) does not match the Windows paths Kivio hands
/// to commands, so it must never be selected as the agent's shell even if it
/// is the first `bash.exe` on PATH.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_wsl_bash_path(path: &Path) -> bool {
    let lowered = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('/', "\\");
    lowered.ends_with(r"\windows\system32\bash.exe")
        || lowered.ends_with(r"\windows\sysnative\bash.exe")
}

/// Describe the shell selected by `build_shell_command` in the tool contract.
pub fn run_command_shell_hint() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        if find_git_bash().is_some() {
            return " Commands run in Git Bash; use bash syntax and Windows paths with forward slashes (C:/Users/...).";
        }
        " Commands run in PowerShell; use PowerShell syntax and cmdlets, with $env:NAME for environment variables."
    }
    #[cfg(not(target_os = "windows"))]
    {
        " Commands run through sh -c; use POSIX shell syntax."
    }
}

/// Build the PowerShell invocation of `command` (`<pwsh|powershell> -NoLogo
/// -NoProfile -NonInteractive -Command …`). Split out of `build_shell_command`
/// so tests can exercise the PowerShell-specific quoting/UTF-8 behavior
/// directly, independent of whether this machine also has Git Bash installed
/// (which would otherwise make `build_shell_command` pick bash instead).
#[cfg(target_os = "windows")]
fn build_windows_powershell_command(command: &str) -> Command {
    let mut c = Command::new(windows_powershell_exe());
    // 用 .arg()(而非 cmd.exe 路径的 raw_arg):PowerShell 用标准 CommandLineToArgvW
    // 解析自身参数,认 \" 转义。把整段脚本作为**单个带引号参数**传给 -Command,内部
    // 引号才能原样还原(python -c "..." 不被拆散)。raw_arg 会让脚本裸拼到命令行,
    // PowerShell 的 argv 分词会在 -Command 处理前吃掉内部引号 → 命令被拆坏。
    c.arg("-NoLogo");
    c.arg("-NoProfile");
    c.arg("-NonInteractive");
    c.arg("-Command");
    c.arg(wrap_ps_command(command));
    c
}

/// Build the platform shell `Command` that runs `command`. Windows prefers Git
/// Bash when discoverable (`find_git_bash`): `bash.exe -c <command>`, the
/// whole command as a single `.arg()` (same shape as the PowerShell branch
/// below — bash's own argv parsing keeps quoting/heredocs intact). No Git
/// Bash found → unchanged fallback to PowerShell
/// (`<pwsh|powershell> -NoLogo -NoProfile -NonInteractive -Command …`); every
/// other platform uses `sh -c`. Callers still set cwd/stdio/env/creation
/// flags/kill_on_drop themselves — this only owns the program + argument shape so
/// the foreground and background paths cannot drift apart.
pub(crate) fn build_shell_command(command: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        if let Some(bash) = find_git_bash() {
            let mut c = Command::new(bash);
            // 与 PowerShell 分支同法:.arg() 把整段命令当单个 argv 传给 bash 的
            // -c,由 bash 自己的引号/转义规则解析,不在 Kivio 侧二次拆分。
            c.arg("-c");
            c.arg(command);
            return c;
        }
        build_windows_powershell_command(command)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    }
}

pub fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    {
        let gid = pid as libc::pid_t;
        unsafe {
            // Graceful first; if the group ignores SIGTERM, follow with SIGKILL.
            libc::kill(-gid, libc::SIGTERM);
            libc::kill(-gid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // /T kills the whole tree, /F forces it. Detached background commands
        // are their own process group (CREATE_NEW_PROCESS_GROUP), so this
        // reaches the children too. CREATE_NO_WINDOW suppresses the taskkill
        // console flash.
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}
