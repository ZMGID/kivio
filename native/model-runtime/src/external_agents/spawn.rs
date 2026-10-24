//! Native host launch boundary. Profile selection is validated before launch.
pub use super::process_io::*;
pub fn agent_cli_command(
    _def: &super::types::RuntimeAgentDef,
    program: &std::path::Path,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.env("PATH", crate::path_env::command_path());
    strip_parent_session_env(&mut command);
    #[cfg(unix)]
    command.process_group(0);
    command
}

use std::{collections::HashMap, path::{Path, PathBuf}, process::Stdio, time::Duration};
use tokio::{process::Command, time::timeout};
use super::types::RuntimeAgentDef;
use crate::proc::NoConsoleWindow;
pub fn cli_probe_command(program: &Path) -> Command {
    let mut command = if let Some(target) = super::wsl::parse_wsl_target(program) {
        super::wsl::wsl_exec_command(&target)
    } else { Command::new(program) };
    command.env("PATH", crate::path_env::command_path());
    strip_parent_session_env(&mut command);
    #[cfg(unix)]
    command.process_group(0);
    command.env("AGY_CLI_DISABLE_AUTO_UPDATE", "true").stdin(Stdio::null()).no_console_window().kill_on_drop(true);
    command
}
include!("../../../../src-tauri/src/external_agents/spawn_probe.rs");

pub fn cli_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let path = Path::new(program.as_ref());
    let mut command = if let Some(target) = super::wsl::parse_wsl_target(path) {
        super::wsl::wsl_exec_command(&target)
    } else { Command::new(program) };
    command.env("PATH", crate::path_env::command_path());
    strip_parent_session_env(&mut command);
    #[cfg(unix)]
    command.process_group(0);
    command
}
