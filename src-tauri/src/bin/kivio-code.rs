//! Kivio Code — Rust terminal coding agent.
//!
//! Thin binary entry: parse args with clap, resolve the prompt, build the
//! headless runtime, and run one print-mode agent turn via
//! `kivio::kivio_code::run_print`. All real logic lives in the library module so
//! it stays unit-testable (`kivio::kivio_code`).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use kivio::kivio_code::{
    build_app_state, load_settings_from_disk, read_stdin_prompt, run_print, PrintOptions,
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Resolve the prompt: -p > positional > piped stdin.
    let prompt = cli
        .print
        .or(cli.prompt)
        .or_else(read_stdin_prompt)
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());

    let Some(prompt) = prompt else {
        eprintln!(
            "kivio-code: no prompt provided. Use -p \"<task>\", a positional prompt, or pipe stdin.\nTry --help."
        );
        return ExitCode::from(2);
    };

    let cwd = match cli.cwd {
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
