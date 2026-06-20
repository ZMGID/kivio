#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    // `kivio code [args]` — dispatch to the terminal coding agent BEFORE any Tauri
    // or window initialization. This must be the very first thing main() does so a
    // CLI invocation never spins up the GUI. The subcommand reuses the kivio-code
    // runtime (interactive TUI / headless print mode) and exits when done; the
    // shared entry attaches the parent terminal's console on Windows so output is
    // visible (the GUI binary has no console of its own).
    let mut args = std::env::args_os();
    let _program = args.next();
    if let Some(first) = args.next() {
        if first == "code" {
            // Rebuild argv for clap as ["kivio code", <rest…>] so usage/version
            // strings read naturally and the `code` token itself is consumed.
            let mut forwarded: Vec<std::ffi::OsString> = Vec::new();
            forwarded.push(std::ffi::OsString::from("kivio code"));
            forwarded.extend(args);
            return kivio::kivio_code::cli::run(forwarded);
        }
    }

    kivio::run();
    ExitCode::SUCCESS
}
