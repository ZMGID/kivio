//! Kivio Code — Rust terminal coding agent.
//!
//! Thin binary entry: parse args with clap, resolve the prompt, then either run
//! one print-mode agent turn (`-p` / piped prompt) or launch the interactive TUI
//! shell (no prompt + a real TTY). All real logic lives in the library module so
//! it stays unit-testable (`kivio::kivio_code`).

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use kivio::kivio_code::interactive::{self, InteractiveOptions, ResumeRequest};
use kivio::kivio_code::{
    build_app_state, load_settings_from_disk, read_stdin_prompt, resolve_provider_model, run_print,
    PrintOptions,
};

#[derive(Parser, Debug)]
#[command(
    name = "kivio-code",
    version,
    about = "Kivio Code — terminal coding agent (reuses Kivio's Rust agent runtime)",
    long_about = None
)]
struct Cli {
    /// Run a single task non-interactively and print the answer to stdout.
    /// If omitted, a positional PROMPT or piped stdin is used.
    #[arg(short = 'p', long = "print", value_name = "PROMPT")]
    print: Option<String>,

    /// Task prompt (positional alternative to -p). Ignored if -p is given.
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,

    /// Model override as `providerId:model` or just `model`.
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,

    /// Provider id override (takes precedence over the providerId in --model).
    #[arg(long, value_name = "PROVIDER_ID")]
    provider: Option<String>,

    /// Working directory the agent operates in (tools are rooted here).
    #[arg(short = 'C', long = "cwd", value_name = "DIR")]
    cwd: Option<PathBuf>,

    /// Deny sensitive tools (write/edit/bash); leave only read-only tools.
    #[arg(long = "no-approve")]
    no_approve: bool,

    /// Stream model reasoning to stderr.
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Resume the most recent session for the working directory (interactive).
    #[arg(short = 'c', long = "continue")]
    continue_recent: bool,

    /// Resume a specific session by id (or partial id) or `.jsonl` path (interactive).
    #[arg(short = 'r', long = "resume", value_name = "ID|PATH")]
    resume: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Resolve the working directory first (shared by both modes).
    let cwd = match cli.cwd.clone() {
        Some(dir) => dir,
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    if !cwd.is_dir() {
        eprintln!(
            "kivio-code: working directory does not exist: {}",
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
    if prompt.is_none() && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        return run_interactive(&cli, &cwd);
    }

    let Some(prompt) = prompt else {
        eprintln!(
            "kivio-code: no prompt provided. Use -p \"<task>\", a positional prompt, or pipe stdin.\nTry --help."
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
            eprintln!("kivio-code: failed to start async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(async move {
        let settings = load_settings_from_disk();
        let state = build_app_state(settings);
        match run_print(options, &state).await {
            Ok(content) => {
                if content.trim().is_empty() {
                    eprintln!("kivio-code: run completed but produced no answer.");
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(err) => {
                eprintln!("kivio-code: {err}");
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
    kivio::kivio_code::session::gc_empty_sessions(cwd);

    let settings = load_settings_from_disk();
    let cfg = kivio::kivio_code::config::load_merged(cwd);
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
            eprintln!("kivio-code: interactive mode failed: {err}");
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
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}
