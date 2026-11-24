use std::{fs::File, io::Read, path::Path, time::Duration};

#[cfg(unix)]
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd},
    ptr,
};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    process::Command,
    time::{self, Instant},
};

const STATUS_PATH: &str = "KIVIO_ANTIGRAVITY_STATUS_PATH";
const STATUS_FILE: &str = "status.json";
const MAX_STATUS_BYTES: u64 = 64 * 1024;
#[cfg(unix)]
const STATUS_TIMEOUT: Duration = Duration::from_secs(9);
#[cfg(windows)]
const STATUS_TIMEOUT: Duration = Duration::from_secs(18);

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
    let settings = json!({
        "statusLine": {
            "type": "command",
            "enabled": true,
            "command": status_callback()
        },
        "title": { "enabled": false }
    });
    command
        .args(["--settings", &settings.to_string()])
        .env(STATUS_PATH, scratch.path().join(STATUS_FILE))
        .env("TERM", "xterm-256color");
    terminal_identity(command, scratch).await
}

async fn wait_for_account(
    mut read_account: impl FnMut() -> Option<Value>,
    mut exited: impl FnMut() -> bool,
) -> Option<Value> {
    let deadline = Instant::now() + STATUS_TIMEOUT;
    let mut latest = None;
    loop {
        if let Some(current) = read_account() {
            if current["email"].is_string() && current["planType"].is_string() {
                return Some(current);
            }
            latest = Some(current);
        }
        if Instant::now() >= deadline || exited() {
            return latest;
        }
        time::sleep(Duration::from_millis(120)).await;
    }
}

#[cfg(unix)]
fn status_callback() -> &'static str {
    "/bin/cat > \"$KIVIO_ANTIGRAVITY_STATUS_PATH\""
}

#[cfg(unix)]
async fn terminal_identity(mut command: Command, scratch: tempfile::TempDir) -> Option<Value> {
    use crate::external_agents::spawn::AgentChild;

    let (mut terminal, slave) = terminal().ok()?;
    command
        .stdin(slave.try_clone().ok()?)
        .stdout(slave.try_clone().ok()?)
        .stderr(slave);
    let mut child = AgentChild::spawn(&mut command).ok()?;
    drop(command);
    let mut buffer = [0; 8192];
    let status_path = scratch.path().join(STATUS_FILE);
    wait_for_account(
        || {
            // Drain rendered terminal output without saving or forwarding it.
            let _ = terminal.read(&mut buffer);
            account(&status_path)
        },
        || child.try_wait().ok().flatten().is_some(),
    )
    .await
}

#[cfg(unix)]
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

#[cfg(windows)]
fn status_callback() -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // agy passes this through cmd /c. Encoding keeps shell quoting away from the path and script.
    let script = "$stream = [IO.File]::Create($env:KIVIO_ANTIGRAVITY_STATUS_PATH); \
        try { [Console]::OpenStandardInput().CopyTo($stream) } finally { $stream.Dispose() }";
    let encoded = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}")
}

#[cfg(windows)]
async fn terminal_identity(mut command: Command, scratch: tempfile::TempDir) -> Option<Value> {
    use crate::external_agents::spawn::acquire_cli_spawn_resource;
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::os::windows::io::AsRawHandle;
    use tokio_util::sync::CancellationToken;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::AssignProcessToJobObject;

    let resource = acquire_cli_spawn_resource(&mut command).ok()?;
    let runtime = tokio::runtime::Handle::current();
    let cancel = CancellationToken::new();
    let _cancel_on_drop = cancel.clone().drop_guard();
    tokio::task::spawn_blocking(move || {
        let _resource = resource;
        if cancel.is_cancelled() {
            return None;
        }
        let command = command.as_std();
        let mut pty_command = CommandBuilder::new(command.get_program());
        pty_command.args(command.get_args());
        if let Some(cwd) = command.get_current_dir() {
            pty_command.cwd(cwd);
        }
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                pty_command.env(key, value);
            } else {
                pty_command.env_remove(key);
            }
        }
        let job = kill_on_close_job()?;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .ok()?;
        let mut reader = pair.master.try_clone_reader().ok()?;
        // ClosePseudoConsole can block until terminal output has been drained.
        let drain = std::thread::Builder::new()
            .spawn(move || {
                let _ = std::io::copy(&mut reader, &mut std::io::sink());
            })
            .ok()?;
        let mut child = pair.slave.spawn_command(pty_command).ok()?;
        drop(pair.slave);
        let assigned = child.as_raw_handle().is_some_and(|process| {
            // Both handles remain owned through the call.
            unsafe { AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process)) }
                .is_ok()
        });
        let found = if assigned {
            let status_path = scratch.path().join(STATUS_FILE);
            runtime.block_on(async {
                tokio::select! {
                    _ = cancel.cancelled() => None,
                    value = wait_for_account(
                        || account(&status_path),
                        || child.try_wait().ok().flatten().is_some(),
                    ) => value,
                }
            })
        } else {
            let _ = child.kill();
            None
        };
        drop(job);
        let _ = child.wait();
        drop(pair.master);
        let _ = drain.join();
        found
    })
    .await
    .ok()?
}

#[cfg(windows)]
fn kill_on_close_job() -> Option<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::core::PCWSTR;
    use windows::Win32::{
        Foundation::HANDLE,
        System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    let raw = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.ok()?;
    let job = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            HANDLE(job.as_raw_handle()),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of_val(&limits) as u32,
        )
    }
    .ok()?;
    Some(job)
}

#[cfg(all(test, unix))]
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
