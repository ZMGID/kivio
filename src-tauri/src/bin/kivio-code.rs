//! Kivio Code — Rust terminal coding agent (standalone binary).
//!
//! Thin wrapper: forwards process args to the shared CLI entry in the library
//! (`kivio::kivio_code::cli::run`). All real logic — clap parsing, prompt
//! resolution, print mode, and the interactive TUI — lives there so the main
//! `kivio` GUI binary can dispatch `kivio code <args>` through the exact same
//! path without shipping a second executable. This bin stays available for
//! development.

use std::process::ExitCode;

fn main() -> ExitCode {
    kivio::kivio_code::cli::run(std::env::args_os())
}
