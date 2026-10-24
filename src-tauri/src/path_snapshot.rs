// Process `PATH` enrichment for GUI launches.
//
// GUI programs don't always inherit the *current* user `PATH`, so packaged
// builds can fail to find user-installed CLIs (claude / codex / pi, Homebrew,
// npm globals, …) even though a terminal-launched `npm run dev` works fine.
// Two platforms need a fixup, for related-but-distinct reasons:
//
// - **macOS**: a `.app` launched from Finder/Dock/Launchpad does **not**
//   inherit the user's login-shell `PATH`. It gets only a minimal default
//   (`/usr/bin:/bin:/usr/sbin:/sbin`). User CLIs live in `/opt/homebrew/bin`,
//   `/usr/local/bin`, `~/.local/bin`, `~/.cargo/bin`, etc. — none of which are
//   on that minimal `PATH`. [`enrich_path_macos`] runs the user's login shell
//   to read its `PATH` and merges in common install dirs.
//
// - **Windows**: a GUI program inherits its environment from `explorer.exe`,
//   whose environment is a **snapshot taken at login**. A user who installs a
//   CLI (mutating the registry `Path`) but doesn't log out/reboot leaves
//   `explorer` — and any Kivio it launches — with a *stale* `PATH` that lacks
//   the new directory, so `where <cli>` finds nothing. (Developers who've
//   rebooted don't see this — "works on my machine".) [`enrich_path_windows`]
//   reads the **current** `Path` straight from the registry (user + system
//   hives), expands `%VAR%` references, and merges in common install dirs.
//   The registry, though, can't cover version managers like **fnm / nvm** that
//   inject a *per-shell* directory into `PATH` from the user's PowerShell
//   profile (fnm runs `fnm env | Invoke-Expression`, prepending a
//   `%LOCALAPPDATA%\fnm_multishells\<pid>_<ts>` dir that never lands in the
//   registry). So Windows adds a second source symmetric to macOS's
//   login-shell probe: it runs the user's PowerShell **with** their profile and
//   reads the live `$env:PATH`, merging those additions in (best-effort, hard
//   timeout, silent fallback to registry-only on any failure).
//
// Both run once at the very start of app startup, before any window creation
// or CLI probing. Because every downstream subprocess (detection,
// `spawn_agent`, MCP stdio servers, skill scripts) inherits the process
// `PATH`, a single fix here covers all of them. Both are read-only,
// never panic, never block startup, and are harmless to re-run / no-ops in
// `dev` (where the process already has the full `PATH`; merge dedups it).
//
// The same GUI-launch gap applies to **locale**. Finder/Dock (macOS) and
// explorer (Windows) often hand the process no `LANG` / `LC_*`, so libc
// defaults to C/POSIX. BSD `ls` then treats CJK as non-printable and prints
// `?` for those filenames — the dock PTY and `run_command` both inherit this.
// [`ensure_utf8_locale`] fills `LANG` with a UTF-8 locale (and lifts a
// blocking C/POSIX `LC_ALL` / `LC_CTYPE`) so every child sees printable
// Unicode. Already-UTF-8 values are left alone.
//
// On Linux this module compiles to just the shared pure helpers, which are
// unused there (the platform entry points are `#[cfg]`-gated to their OS).

#[cfg(any(target_os = "macos", target_os = "windows", test))]
use std::collections::HashSet;

/// Push `seg` onto `out` if non-empty and not already present (case-sensitive
/// on macOS, but Windows paths fold below). Used by all platforms' merge logic.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn push_unique(
    seg: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
    key: impl Fn(&str) -> String,
) {
    let seg = seg.trim();
    if seg.is_empty() {
        return;
    }
    if seen.insert(key(seg)) {
        out.push(seg.to_string());
    }
}

/// Spawn `cmd` (already configured with stdio + `NoConsoleWindow` by the
/// caller), wait for it to finish on a helper thread, and return its stdout as
/// a lossy `String` if it exits successfully within `timeout`. Returns `None`
/// on spawn error, non-zero exit, I/O error, or timeout — so callers fall back
/// to their defaults. Never panics; timeout cleanup may add only the short
/// kill/reap margin. Output
/// post-processing (trim / empty / validity checks) is left to the caller.
///
/// Stdout is drained on a helper thread while the caller retains ownership of
/// the child. On timeout we kill + reap the process and join the reader before
/// returning, so startup never leaves a zombie process or detached thread.
/// Shared by the macOS login-shell probe and the Windows profile probe so their
/// timeout semantics can't drift apart.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn capture_stdout_with_timeout(
    mut cmd: std::process::Command,
    timeout: std::time::Duration,
) -> Option<String> {
    use std::io::Read;

    let mut child = cmd.spawn().ok()?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = std::time::Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().ok()?.ok()?;
                return status
                    .success()
                    .then(|| String::from_utf8_lossy(&bytes).to_string());
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // Timeout or an inability to poll the process: terminate it, reap
            // it, and join the pipe reader before returning. This avoids both
            // zombie children and detached helper threads during app startup.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

/// Hard timeout for invoking the login shell to read its `PATH`. Some users'
/// shell rc files are slow (network calls, version managers); we must never
/// block app startup on them, so we cap the wait and fall back to the
/// common-directory defaults if it doesn't return in time.
#[cfg(target_os = "macos")]
const LOGIN_SHELL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Merge the current `PATH` with the login-shell `PATH` and common install
/// directories, deduplicate (preserving order), and write the result back to
/// the process `PATH`.
///
/// The expensive part — spawning the user's login shell to read its `PATH` —
/// runs **at most once per process** (guarded by [`Once`]). Startup calls this
/// first (`lib.rs`), so every later caller (plugin/CLI detection via
/// `refresh_process_path_for_detection`) is a no-op: the process `PATH` is
/// already fixed and doesn't change, and re-running `fish -l -i` each time the
/// plugins page opens added seconds for no benefit.
// ponytail: once-per-process. A CLI installed into a *new* dir mid-session
// (not one of common_dirs_macos) won't be seen until restart — rare on macOS,
// where AI installs land in ~/.local/bin etc. (already in defaults).


/// Merge the current `PATH`, the (optional) login-shell `PATH`, and the
/// fallback `defaults` into a single `:`-joined string, deduplicated and
/// order-preserving. Pure — no env access — so it is unit-testable without
/// mutating shared process state.
///
/// Order: existing `PATH` first (preserves current resolution order), then any
/// login-shell additions, then defaults for entries neither source provided.
#[cfg(any(target_os = "macos", test))]
fn merge_paths_unix(current: &str, login: Option<&str>, defaults: &[String]) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<String> = Vec::new();
    for source in [current, login.unwrap_or("")] {
        for seg in source.split(':') {
            push_unique(seg, &mut seen, &mut merged, |s| s.to_string());
        }
    }
    for dir in defaults {
        push_unique(dir, &mut seen, &mut merged, |s| s.to_string());
    }
    merged.join(":")
}

/// Common directories where CLIs get installed but which are absent from the
/// minimal Finder/Dock `PATH`. `$HOME`-relative entries are expanded against
/// `home`; if `home` is `None`/empty those entries are simply skipped. Takes
/// `home` as a parameter (rather than reading `$HOME`) so it is testable
/// without env mutation.
#[cfg(any(target_os = "macos", test))]
fn common_dirs_macos(home: Option<std::path::PathBuf>) -> Vec<String> {
    let mut dirs = vec![
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/local/sbin".to_string(),
        "/usr/bin".to_string(),
        "/bin".to_string(),
        "/usr/sbin".to_string(),
        "/sbin".to_string(),
    ];
    if let Some(home) = home {
        if !home.as_os_str().is_empty() {
            // This function models macOS paths even when its pure helpers are
            // exercised by tests on Windows, so do not inherit the host path
            // separator from `PathBuf::join`.
            let home = home.to_string_lossy().replace('\\', "/");
            let home = home.trim_end_matches('/');
            for rel in [".local/bin", ".cargo/bin", ".bun/bin"] {
                dirs.push(format!("{home}/{rel}"));
            }
        }
    }
    dirs
}

/// Read the user's login-shell `PATH` by running
/// `$SHELL -l -i -c 'echo $PATH'` with a hard timeout. Returns `None` on any
/// failure (spawn error, non-zero exit, timeout, empty output) so the caller
/// falls back to the common-directory defaults. Never panics, never blocks
/// past [`LOGIN_SHELL_TIMEOUT`].
#[cfg(target_os = "macos")]
fn login_shell_path() -> Option<String> {
    use crate::proc::NoConsoleWindow;
    use std::process::{Command, Stdio};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    // Spawn the login+interactive shell so it sources the rc files that set
    // PATH (e.g. ~/.zshrc), then echo the resulting PATH on a single line.
    let mut cmd = Command::new(&shell);
    cmd.args(["-l", "-i", "-c", "echo \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .no_console_window();

    // Enforce the timeout via the shared helper: if the shell hangs (slow rc),
    // give up rather than blocking startup.
    let path = capture_stdout_with_timeout(cmd, LOGIN_SHELL_TIMEOUT)?;
    let path = path.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// Hard timeout for running the user's PowerShell profile to read `$env:PATH`.
/// Symmetric with macOS's [`LOGIN_SHELL_TIMEOUT`]: PowerShell 5.1 cold-start +
/// an fnm profile is typically well under this; if the user's profile is slow
/// we give up and fall back to registry + common-dir defaults.
#[cfg(target_os = "windows")]
const PROFILE_SHELL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Pick the PowerShell executable for the profile probe: prefer `pwsh`
/// (PowerShell 7+) when discoverable on the *current* process `PATH`, else fall
/// back to Windows PowerShell (`powershell`, 5.1). This must run after the
/// phase-1 registry merge so a freshly-installed pwsh is visible. Mirrors
/// `native_tools::shell::pwsh_on_path`, but implemented locally (no shared
/// `OnceLock`) so it re-evaluates the freshly-merged `PATH` rather than
/// fossilising an early-startup answer. The choice matters because the two
/// shells load different profile files — picking the wrong one reads the wrong
/// (or no) user config.
#[cfg(target_os = "windows")]
fn profile_shell_exe() -> &'static str {
    let has_pwsh = std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("pwsh.exe").is_file())
    });
    if has_pwsh {
        "pwsh"
    } else {
        "powershell"
    }
}

/// Run the user's PowerShell **with** their profile and read the resulting
/// `$env:PATH`. This is the only way to pick up version-manager dirs (fnm/nvm)
/// injected per-shell from the profile — they never appear in the registry.
///
/// Deliberately omits `-NoProfile` (loading the profile is the whole point),
/// unlike `native_tools::shell` which keeps `-NoProfile` for fast, deterministic
/// tool execution. Sets stdout to UTF-8 first (PS 5.1 defaults to the OEM code
/// page, mangling non-ASCII dir names; pwsh is already UTF-8 so it's a no-op).
/// stdin/stderr are nulled so profile banners/warnings don't pollute the result,
/// and `NoConsoleWindow` prevents a console flash. Returns `None` on any
/// failure/timeout so the caller keeps the registry-only PATH.
#[cfg(target_os = "windows")]
fn profile_shell_path() -> Option<String> {
    use crate::proc::NoConsoleWindow;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(profile_shell_exe());
    cmd.args([
        "-NoLogo",
        "-NonInteractive",
        "-Command",
        "try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}; $env:PATH",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .no_console_window();

    let out = capture_stdout_with_timeout(cmd, PROFILE_SHELL_TIMEOUT)?;
    parse_profile_path_output(&out)
}

/// Extract the `PATH` from the profile probe's stdout. Takes the last non-empty
/// line (a profile may print banners/output before `$env:PATH` — the PATH is the
/// last thing echoed) and validates it *looks* like a PATH: it must contain a
/// `;` separator or be a single drive-rooted path (`X:\...`). This rejects
/// stray profile chatter mistakenly captured as the last line. Returns `None`
/// if nothing valid is found. Pure — unit-testable on all platforms.
#[cfg(any(target_os = "windows", test))]
fn parse_profile_path_output(output: &str) -> Option<String> {
    let candidate = output
        .lines()
        .map(str::trim)
        .rev()
        .find(|line| !line.is_empty())?;

    // Legitimacy check: a real PATH has at least one ';' separator, or is a
    // single drive-rooted path (matches `^[A-Za-z]:\`).
    let looks_like_path = candidate.contains(';') || is_drive_rooted(candidate);
    if looks_like_path {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// Whether `s` begins with a Windows drive-root prefix (`X:\`). Used to accept a
/// single-entry PATH that has no `;` separator.
#[cfg(any(target_os = "windows", test))]
fn is_drive_rooted(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

/// Read the *current* user + system `PATH` from the registry, expand `%VAR%`
/// references, merge with the (possibly stale) process `PATH` and common CLI
/// install dirs, and write the deduplicated result back to the process `PATH`.
///
/// This works around the stale-`PATH`-snapshot problem (see module docs): a
/// user who installs a CLI but hasn't logged out/rebooted has an `explorer`
/// environment — and thus a Kivio process — whose `PATH` predates the install.
/// Reading the registry gives us the *current* value. Read-only (never writes
/// the registry), never panics, never blocks; on any failure it still merges
/// in the common-directory defaults.
///
/// The registry, however, can't cover version managers like **fnm / nvm** that
/// inject a *per-shell* directory into `PATH` from the user's PowerShell
/// profile (fnm's `fnm env | Invoke-Expression` prepends
/// `%LOCALAPPDATA%\fnm_multishells\<pid>_<ts>`, which never touches the
/// registry). So this runs in two phases:
///
/// 1. Merge registry + defaults into the process `PATH` and set it. This also
///    makes a freshly-installed `pwsh` (typically under
///    `%ProgramFiles%\PowerShell\7`, absent from a stale `PATH`) discoverable
///    by the probe below.
/// 2. Run the user's PowerShell **with** their profile to read the live
///    `$env:PATH`, then merge those additions in. Best-effort with a hard
///    timeout: on no-profile / profile error / missing PowerShell / empty
///    output / timeout it silently keeps the phase-1 result, so behaviour is
///    identical to the registry-only path.


/// Re-read the live registry `Path` into this process.
///
/// [`enrich_path_windows`] is once-per-process so startup stays cheap. A Node
/// installer that writes `D:\Program Files\nodejs` into the registry mid-session
/// is invisible until this runs — which is exactly the Kivio one-click install
/// path, and why `npm.cmd` could start while `node ./cnoke.cjs` printed
/// `'node' 不是内部或外部命令`.




// ---------------------------------------------------------------------------
// Locale (UTF-8)
// ---------------------------------------------------------------------------

pub use crate::utf8_locale::*;
/// Fill process `LANG` with a UTF-8 locale when the GUI-inherited env would
/// leave libc in C/POSIX. Runs once; never panics. Children inherit the result,
/// so one fix covers the dock PTY, `run_command`, MCP, and CLI probes.


/// Merge the process `PATH` with the system + user registry `PATH` values, the
/// (optional) PowerShell-profile `PATH`, and the fallback `defaults` into a
/// single `;`-joined string, deduplicated and order-preserving. Windows path
/// comparison is case-insensitive, so dedup folds case (the first-seen spelling
/// is kept). Pure — no env/registry access — so it is unit-testable without
/// mutating shared state.
///
/// Order: process `PATH` first (preserves current resolution order), then
/// system, then user, then profile, then common-dir defaults. A dir already
/// present from an earlier source (e.g. a system node install) therefore wins
/// over a later profile source (e.g. fnm's node) — a documented tradeoff that
/// mirrors the macOS branch's `current`-first ordering.
#[cfg(any(target_os = "windows", test))]
fn merge_paths_windows(
    current: &str,
    system: Option<&str>,
    user: Option<&str>,
    profile: Option<&str>,
    defaults: &[String],
) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<String> = Vec::new();
    for source in [
        current,
        system.unwrap_or(""),
        user.unwrap_or(""),
        profile.unwrap_or(""),
    ] {
        for seg in source.split(';') {
            push_unique(seg, &mut seen, &mut merged, |s| s.to_ascii_lowercase());
        }
    }
    for dir in defaults {
        push_unique(dir, &mut seen, &mut merged, |s| s.to_ascii_lowercase());
    }
    merged.join(";")
}

/// Expand `%VAR%` references in a registry `PATH` string using the current
/// process environment (`std::env::var`). Unknown variables are left verbatim
/// (matching Windows behaviour). `REG_EXPAND_SZ` values in particular contain
/// unexpanded `%USERPROFILE%` / `%APPDATA%` etc., so this must run on whatever
/// the registry hands back. Pure aside from reading env vars.
#[cfg(target_os = "windows")]
fn expand_env_vars(input: &str) -> String {
    expand_env_vars_with(input, |name| std::env::var(name).ok())
}

/// Core of [`expand_env_vars`], parameterised over the variable lookup so it
/// can be unit-tested deterministically without touching the process env.
#[cfg(any(target_os = "windows", test))]
fn expand_env_vars_with(input: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            // Find the closing '%'.
            if let Some(end) = input[i + 1..].find('%') {
                let name = &input[i + 1..i + 1 + end];
                if name.is_empty() {
                    // "%%" → literal '%'.
                    out.push('%');
                    i += 2;
                    continue;
                }
                match lookup(name) {
                    Some(val) => out.push_str(&val),
                    // Unknown variable: keep the literal `%VAR%`.
                    None => out.push_str(&input[i..i + 1 + end + 1]),
                }
                i += 1 + end + 1;
                continue;
            }
        }
        // Push this UTF-8 char whole (input slicing above is on ASCII '%' only,
        // so char boundaries are safe).
        let ch_len = input[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&input[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Common directories where Windows CLIs get installed but which may be missing
/// from a stale process `PATH`. Built from the current process env; entries
/// whose base var is absent are skipped. Read-only.
#[cfg(target_os = "windows")]
fn common_dirs_windows() -> Vec<String> {
    let mut dirs = Vec::new();
    let mut push = |base: Option<String>, rel: &str| {
        if let Some(base) = base {
            if !base.trim().is_empty() {
                dirs.push(format!("{}\\{}", base.trim_end_matches('\\'), rel));
            }
        }
    };
    let appdata = std::env::var("APPDATA").ok();
    let userprofile = std::env::var("USERPROFILE").ok();
    let localappdata = std::env::var("LOCALAPPDATA").ok();

    push(appdata.clone(), "npm");
    push(std::env::var("ProgramFiles").ok(), "nodejs");
    push(userprofile.clone(), ".cargo\\bin");
    push(userprofile.clone(), ".bun\\bin");
    push(userprofile.clone(), "scoop\\shims");
    push(localappdata.clone(), "Microsoft\\WinGet\\Links");
    // 国内小白常把软件装到 D:/E: 的 Program Files；%ProgramFiles% 仍指向 C:。
    dirs.push(r"D:\Program Files\nodejs".to_string());
    dirs.push(r"E:\Program Files\nodejs".to_string());

    // --- Version-manager stable dirs (second line of defense for the profile
    // probe: used when the probe times out or PowerShell is unavailable). fnm's
    // *active* dir is per-shell (fnm_multishells\<pid>_<ts>) and thus not
    // knowable here, but the `default` alias and nvm's symlink are stable.
    //
    // nvm-windows: NVM_SYMLINK is the fixed path of the active node install.
    if let Ok(symlink) = std::env::var("NVM_SYMLINK") {
        let symlink = symlink.trim();
        if !symlink.is_empty() {
            dirs.push(symlink.to_string());
        }
    }
    // fnm: the `default` alias is a stable dir. Prefer FNM_DIR; else probe the
    // default roots (%LOCALAPPDATA%\fnm and %USERPROFILE%\.fnm). The node binary
    // may sit directly in `aliases\default` or under an `installation\` subdir
    // depending on fnm version, so push both candidates — nonexistent dirs are
    // harmless in PATH.
    let fnm_roots: Vec<String> = if let Ok(fnm_dir) = std::env::var("FNM_DIR") {
        vec![fnm_dir]
    } else {
        let mut roots = Vec::new();
        if let Some(local) = localappdata.as_deref() {
            roots.push(format!("{}\\fnm", local.trim_end_matches('\\')));
        }
        if let Some(profile) = userprofile.as_deref() {
            roots.push(format!("{}\\.fnm", profile.trim_end_matches('\\')));
        }
        roots
    };
    for root in fnm_roots {
        let root = root.trim().trim_end_matches('\\');
        if root.is_empty() {
            continue;
        }
        dirs.push(format!("{}\\aliases\\default", root));
        dirs.push(format!("{}\\aliases\\default\\installation", root));
    }

    dirs
}

/// Read the `Path` value from either the system or user environment registry
/// hive. Returns `None` if the key/value is absent or any registry call fails
/// (callers fall back to defaults). Read-only — never opens for write, never
/// modifies the registry. Uses the `RegOpenKeyExW`/`RegQueryValueExW` pattern.
///
/// - `system == true`  → `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`
/// - `system == false` → `HKCU\Environment`
#[cfg(target_os = "windows")]
fn read_registry_path(system: bool) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ,
    };

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let (root, subkey) = if system {
        (
            HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        )
    } else {
        (HKEY_CURRENT_USER, "Environment")
    };

    unsafe {
        let mut hkey = HKEY::default();
        let subkey_w = wide(subkey);
        let status = RegOpenKeyExW(root, PCWSTR(subkey_w.as_ptr()), None, KEY_READ, &mut hkey);
        if status != ERROR_SUCCESS {
            return None;
        }

        let value_name = wide("Path");
        let mut value_type = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let mut size: u32 = 0;
        let q = RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        );
        if q != ERROR_SUCCESS || size == 0 {
            let _ = RegCloseKey(hkey);
            return None;
        }

        let mut buf = vec![0u8; size as usize];
        let mut sz = size;
        let q2 = RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut value_type),
            Some(buf.as_mut_ptr()),
            Some(&mut sz),
        );
        let _ = RegCloseKey(hkey);
        if q2 != ERROR_SUCCESS {
            return None;
        }

        // Bytes → UTF-16 → String, trimming any trailing NUL(s).
        let u16s: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let mut s = String::from_utf16_lossy(&u16s);
        while s.ends_with('\0') {
            s.pop();
        }
        if s.trim().is_empty() {
            None
        } else {
            Some(s)
        }
    }
}


/// Resolve a child-process PATH without mutating the multithreaded host environment.
pub fn command_path() -> std::ffi::OsString {
    let current = std::env::var("PATH").unwrap_or_default();
    #[cfg(target_os = "macos")]
    { return merge_paths_unix(&current, None, &common_dirs_macos(std::env::var_os("HOME").map(std::path::PathBuf::from))).into(); }
    #[cfg(target_os = "windows")]
    { return merge_paths_windows(&current, read_registry_path(true).map(|p| expand_env_vars(&p)).as_deref(), read_registry_path(false).map(|p| expand_env_vars(&p)).as_deref(), None, &common_dirs_windows()).into(); }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { current.into() }
}
