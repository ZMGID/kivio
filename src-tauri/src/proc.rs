//! Shared subprocess-spawn helpers.
//!
//! On Windows, spawning a console subprocess from a GUI (windows-subsystem)
//! process flashes a console window unless the `CREATE_NO_WINDOW` creation flag
//! is set. Every place that spawns an external CLI (external agents, MCP
//! servers, skill scripts, `where`/`which` probes, …) must apply this flag or
//! users see a storm of terminal windows on launch / update / model probe.
//!
//! Both [`std::process::Command`] and [`tokio::process::Command`] implement
//! `std::os::windows::process::CommandExt::creation_flags` on Windows, so a
//! single generic extension trait covers both. On non-Windows the methods are
//! no-ops that return the command unchanged.

/// `CREATE_NO_WINDOW` — suppresses the console window for a child console
/// process. See the Windows process-creation flags documentation.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Extension trait applied to both `std::process::Command` and
/// `tokio::process::Command` so callers can write `cmd.no_console_window()`
/// regardless of which Command flavor they hold.
pub trait NoConsoleWindow {
    /// Apply `CREATE_NO_WINDOW` on Windows; no-op elsewhere. Returns `&mut Self`
    /// so it chains with the usual `Command` builder calls.
    fn no_console_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl NoConsoleWindow for std::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(windows)]
impl NoConsoleWindow for tokio::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        // tokio's Command exposes `creation_flags` as an inherent method on
        // Windows (no `CommandExt` import needed, unlike std's Command above).
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl NoConsoleWindow for std::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        self
    }
}

#[cfg(not(windows))]
impl NoConsoleWindow for tokio::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        self
    }
}
