//! CLI entry shared by the standalone `kivio-code` binary and the `kivio code`
//! subcommand of the main GUI binary.
//!
//! The argument parsing, prompt resolution, print-mode run, and interactive TUI
//! launch all live here (rather than in `bin/kivio-code.rs`) so the GUI binary
//! can dispatch `kivio code <args>` into the exact same code path without
//! shipping a second executable. The standalone bin is now a thin wrapper that
//! forwards `std::env::args()` to [`run`].
//!
//! ## Windows console attach
//! The main `kivio.exe` is a GUI (`windows`-subsystem) program: launched from a
//! terminal it has no attached console, so a TUI/`println!` would go nowhere. On
//! Windows [`run`] first attaches to the parent terminal's console
//! (`AttachConsole(ATTACH_PARENT_PROCESS)`, falling back to `AllocConsole`) and
//! re-syncs the Rust std handles before any output. The standalone bin is a
//! console-subsystem program and already has a console, so the attach is a
//! cheap no-op there (AttachConsole simply fails with ERROR_ACCESS_DENIED).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use crate::kivio_code::interactive::{self, InteractiveOptions, ResumeRequest};
use crate::kivio_code::{
    build_app_state, load_settings_from_disk, read_stdin_prompt, resolve_provider_model, run_print,
    PrintOptions,
};

#[derive(Parser, Debug)]
#[command(
    name = "kivio code",
    bin_name = "kivio code",
    version,
    about = "Kivio Code — terminal coding agent (reuses Kivio's Rust agent runtime)",
    long_about = None
)]
pub struct Cli {
    /// Run a single task non-interactively and print the answer to stdout.
    /// If omitted, a positional PROMPT or piped stdin is used.
    #[arg(short = 'p', long = "print", value_name = "PROMPT")]
    pub print: Option<String>,

    /// Task prompt (positional alternative to -p). Ignored if -p is given.
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Model override as `providerId:model` or just `model`.
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Provider id override (takes precedence over the providerId in --model).
    #[arg(long, value_name = "PROVIDER_ID")]
    pub provider: Option<String>,

    /// Working directory the agent operates in (tools are rooted here).
    #[arg(short = 'C', long = "cwd", value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Deny sensitive tools (write/edit/bash); leave only read-only tools.
    #[arg(long = "no-approve")]
    pub no_approve: bool,

    /// Stream model reasoning to stderr.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Resume the most recent session for the working directory (interactive).
    #[arg(short = 'c', long = "continue")]
    pub continue_recent: bool,

    /// Resume a specific session by id (or partial id) or `.jsonl` path (interactive).
    #[arg(short = 'r', long = "resume", value_name = "ID|PATH")]
    pub resume: Option<String>,
}

/// Parse `args` (program name first, e.g. `["kivio code", "-p", "hi"]`) and run
/// the agent. Used by both the standalone bin (`std::env::args()`) and the
/// `kivio code` subcommand (which synthesizes `["kivio code", <rest…>]`).
///
/// On Windows the parent terminal's console is attached first so TUI / stdout
/// reaches the user even when invoked from the GUI binary.
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    #[cfg(windows)]
    attach_parent_console();

    let cli = Cli::parse_from(args);

    // Resolve the working directory first (shared by both modes).
    let cwd = match cli.cwd.clone() {
        Some(dir) => dir,
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    if !cwd.is_dir() {
        eprintln!(
            "kivio code: working directory does not exist: {}",
            cwd.display()
        );
        return ExitCode::from(2);
    }

    // Resolve the prompt: -p > positional > piped stdin.
    let prompt = cli
        .print
        .clone()
        .or(cli.prompt.clone())
        .or_else(read_stdin_prompt)
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());

    // No prompt + a real interactive TTY (both stdin and stdout) → interactive shell.
    // Otherwise fall through to print mode (which errors if there is still no prompt).
    use std::io::IsTerminal;
    if prompt.is_none() && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        return run_interactive(&cli, &cwd);
    }

    let Some(prompt) = prompt else {
        eprintln!(
            "kivio code: no prompt provided. Use -p \"<task>\", a positional prompt, or pipe stdin.\nTry --help."
        );
        return ExitCode::from(2);
    };

    let options = PrintOptions {
        prompt,
        model: cli.model,
        provider: cli.provider,
        cwd,
        no_approve: cli.no_approve,
        verbose: cli.verbose,
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("kivio code: failed to start async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(async move {
        let settings = load_settings_from_disk();
        let state = build_app_state(settings);
        match run_print(options, &state).await {
            Ok(content) => {
                if content.trim().is_empty() {
                    eprintln!("kivio code: run completed but produced no answer.");
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(err) => {
                eprintln!("kivio code: {err}");
                ExitCode::FAILURE
            }
        }
    })
}

/// Launch the interactive TUI shell. Resolves the model for the footer from
/// settings + `--model`/`--provider`, threads the CLI overrides (cwd / model /
/// provider / no-approve / verbose / resume) into [`InteractiveOptions`], and
/// runs the event loop.
fn run_interactive(cli: &Cli, cwd: &Path) -> ExitCode {
    // GC any header-only ("empty shell") sessions left in this cwd by earlier
    // bare launches that exited without a turn. Best-effort and conservative —
    // only positively-confirmed header-only files are removed.
    crate::kivio_code::session::gc_empty_sessions(cwd);

    let settings = load_settings_from_disk();
    let cfg = crate::kivio_code::config::load_merged(cwd);
    let model = match resolve_provider_model(
        &settings,
        &cfg,
        cli.provider.as_deref(),
        cli.model.as_deref(),
    ) {
        Ok((provider, model)) => format!("{}:{}", provider.id, model),
        // No configured model yet — still launch the shell; the footer shows a hint.
        Err(_) => "<no model>".to_string(),
    };

    // `-r` takes precedence over `-c`.
    let resume = if let Some(reference) = cli.resume.clone() {
        Some(ResumeRequest::Reference(reference))
    } else if cli.continue_recent {
        Some(ResumeRequest::Recent)
    } else {
        None
    };

    let options = InteractiveOptions {
        cwd_display: display_cwd(cwd),
        model,
        cwd: cwd.to_path_buf(),
        provider_override: cli.provider.clone(),
        model_override: cli.model.clone(),
        no_approve: cli.no_approve,
        verbose: cli.verbose,
        resume,
    };

    match interactive::run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("kivio code: interactive mode failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Collapse `$HOME` prefix to `~` for footer display.
fn display_cwd(cwd: &Path) -> String {
    let full = cwd.display().to_string();
    if let Some(home) = dirs_home() {
        let home = home.display().to_string();
        if full == home {
            return "~".to_string();
        }
        if let Some(rest) = full.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    full
}

fn dirs_home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Attach the GUI process to the parent terminal's console so `kivio code`
/// launched from a terminal can read stdin and write stdout/stderr there.
///
/// The main `kivio.exe` is built with `windows_subsystem = "windows"`, which
/// means Windows does NOT give it a console and does NOT attach it to the
/// terminal that launched it. Without a console, `println!`/the TUI write to
/// nothing. `AttachConsole(ATTACH_PARENT_PROCESS)` borrows the parent terminal's
/// console; if there is no parent console (e.g. launched from Explorer) we
/// allocate a fresh one so output is at least visible somewhere. After attaching
/// we re-open the CRT std streams against the new console handles — without this,
/// the C runtime's cached stdout/stdin/stderr still point at the (invalid)
/// pre-attach handles and printing silently fails.
#[cfg(windows)]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};

    unsafe {
        // Prefer the parent terminal's console; if that fails (no parent console),
        // allocate our own so the user still sees output.
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            let _ = AllocConsole();
        }

        // Re-bind the CRT std streams to the freshly attached console, otherwise
        // stdout/stderr/stdin keep pointing at the invalid pre-attach handles and
        // all I/O is silently dropped.
        reopen_std_streams();
    }
}

/// Re-open C runtime stdin/stdout/stderr against `CONIN$`/`CONOUT$` so buffered
/// std I/O targets the console attached above. Best-effort: failures are ignored.
#[cfg(windows)]
unsafe fn reopen_std_streams() {
    use std::ffi::CString;

    // `freopen` rebinds the CRT FILE* for the standard streams. We link the CRT
    // symbol directly to avoid pulling a heavier dependency just for this.
    extern "C" {
        fn freopen(
            filename: *const std::os::raw::c_char,
            mode: *const std::os::raw::c_char,
            stream: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        fn __acrt_iob_func(index: u32) -> *mut std::ffi::c_void;
    }

    let conout = CString::new("CONOUT$").unwrap();
    let conin = CString::new("CONIN$").unwrap();
    let mode_w = CString::new("w").unwrap();
    let mode_r = CString::new("r").unwrap();

    // stdin = iob(0), stdout = iob(1), stderr = iob(2).
    let _ = freopen(conin.as_ptr(), mode_r.as_ptr(), __acrt_iob_func(0));
    let _ = freopen(conout.as_ptr(), mode_w.as_ptr(), __acrt_iob_func(1));
    let _ = freopen(conout.as_ptr(), mode_w.as_ptr(), __acrt_iob_func(2));
}
