use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd},
    path::Path,
    ptr,
    time::Duration,
};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    process::Command,
    time::{self, Instant},
};

use crate::external_agents::spawn::AgentChild;

const STATUS_PATH: &str = "KIVIO_ANTIGRAVITY_STATUS_PATH";
const MAX_STATUS_BYTES: u64 = 64 * 1024;

#[derive(Deserialize)]
struct StatusLine {
    product: String,
    email: Option<String>,
    plan_tier: Option<String>,
}

fn account(path: &Path) -> Option<Value> {
    let mut data = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_STATUS_BYTES + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() as u64 > MAX_STATUS_BYTES {
        return None;
    }
    let status: StatusLine = serde_json::from_slice(&data).ok()?;
    if status.product != "antigravity" {
        return None;
    }
    let email = status
        .email
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let plan = status
        .plan_tier
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    (email.is_some() || plan.is_some())
        .then(|| json!({"type":"google","email":email,"planType":plan}))
}

/// The CLI emits its documented status payload only inside a terminal.
/// Launch without a prompt and apply the callback solely to this process.
pub(super) async fn identity(mut command: Command) -> Option<Value> {
    let scratch = tempfile::tempdir().ok()?;
    let status_path = scratch.path().join("status.json");
    let settings = json!({
        "statusLine": {
            "type": "command",
            "enabled": true,
            "command": "/bin/cat > \"$KIVIO_ANTIGRAVITY_STATUS_PATH\""
        },
        "title": { "enabled": false }
    });
    let (mut terminal, slave) = terminal().ok()?;
    command
        .args(["--settings", &settings.to_string()])
        .env(STATUS_PATH, &status_path)
        .env("TERM", "xterm-256color")
        .stdin(slave.try_clone().ok()?)
        .stdout(slave.try_clone().ok()?)
        .stderr(slave);
    let mut child = AgentChild::spawn(&mut command).ok()?;
    drop(command);
    let deadline = Instant::now() + Duration::from_secs(9);
    let mut latest = None;
    let mut buffer = [0; 8192];
    loop {
        // Drain rendered terminal output without saving or forwarding it.
        let _ = terminal.read(&mut buffer);
        if let Some(current) = account(&status_path) {
            if current["email"].is_string() && current["planType"].is_string() {
                return Some(current);
            }
            latest = Some(current);
        }
        if Instant::now() >= deadline || child.try_wait().ok().flatten().is_some() {
            return latest;
        }
        time::sleep(Duration::from_millis(120)).await;
    }
}

fn terminal() -> io::Result<(File, File)> {
    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize {
        ws_row: 30,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // openpty initializes both owned descriptors on success; no termios override.
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    if unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok((master, slave))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn terminal_receives_only_display_metadata_from_the_native_status_callback() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", r#"
test -t 0 && test -t 1 && test "$1" = --settings || exit 1
case "$2" in *statusLine*KIVIO_ANTIGRAVITY_STATUS_PATH*) ;; *) exit 1;; esac
printf '%s' '{"product":"antigravity","email":"fixture@example.test","plan_tier":"Google AI Pro","cwd":"/private/location"}' > "$KIVIO_ANTIGRAVITY_STATUS_PATH"
"#, "fixture"]);
        assert_eq!(
            identity(command).await,
            Some(json!({
                "type": "google", "email":"fixture@example.test", "planType":"Google AI Pro"
            }))
        );
    }
}
