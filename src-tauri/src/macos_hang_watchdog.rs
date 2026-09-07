//! Collect evidence for intermittent macOS UI hangs without involving Tokio or
//! application-state locks. Only one main-thread ping may be outstanding.

use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
const PING_INTERVAL: Duration = Duration::from_secs(5);
const HANG_TIMEOUT: Duration = Duration::from_secs(10);

struct HangProbe {
    last_check: Instant,
    consecutive_timeouts: u8,
    sampled: bool,
}

impl HangProbe {
    fn new(now: Instant) -> Self {
        Self {
            last_check: now,
            consecutive_timeouts: 0,
            sampled: false,
        }
    }

    fn timed_out(&mut self, now: Instant) -> bool {
        // Suspend/resume or severe scheduling delay is not evidence of a UI
        // deadlock. Require two normally spaced missed acknowledgements again.
        if now.duration_since(self.last_check) > HANG_TIMEOUT * 2 {
            self.consecutive_timeouts = 0;
        } else {
            self.consecutive_timeouts = self.consecutive_timeouts.saturating_add(1);
        }
        self.last_check = now;
        if self.consecutive_timeouts >= 2 && !self.sampled {
            self.sampled = true;
            true
        } else {
            false
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn start(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Ok(log_dir) = app.path().app_log_dir() else {
        return;
    };
    let log_dir = log_dir.join("hangs");
    let app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("kivio-ui-watchdog".into())
        .spawn(move || {
            let mut sample_slot = 0;
            loop {
                std::thread::sleep(PING_INTERVAL);
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                if app
                    .run_on_main_thread(move || {
                        let _ = tx.send(());
                    })
                    .is_err()
                {
                    break;
                }
                let mut probe = HangProbe::new(Instant::now());
                loop {
                    match rx.recv_timeout(HANG_TIMEOUT) {
                        Ok(()) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            if probe.timed_out(Instant::now()) {
                                capture(&log_dir, sample_slot);
                                sample_slot = (sample_slot + 1) % 3;
                            }
                        }
                    }
                    // Keep waiting on this same ping, rather than filling a
                    // blocked event loop with more callbacks or sampling forever.
                }
            }
        })
    {
        eprintln!("macOS UI watchdog could not start: {error}");
    }
}

#[cfg(target_os = "macos")]
fn capture(log_dir: &std::path::Path, slot: usize) {
    use std::process::{Command, Stdio};
    use wait_timeout::ChildExt;

    if std::fs::create_dir_all(log_dir).is_err() {
        return;
    }
    let output = log_dir.join(format!("kivio-hang-{slot}.txt"));
    eprintln!(
        "macOS main thread unresponsive for at least 20s; sampling to {}",
        output.display()
    );
    // Apple's sample collects native stacks, not conversation text or credentials.
    // Three rotating files bound retention. All process operations run here on
    // the watchdog thread, including kill + wait if the sampler itself stalls.
    let child = Command::new("/usr/bin/sample")
        .arg(std::process::id().to_string())
        .args(["1", "-file"])
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        eprintln!("macOS hang sampling failed to start");
        return;
    };
    match child.wait_timeout(Duration::from_secs(10)) {
        Ok(Some(status)) if status.success() => {}
        Ok(Some(status)) => eprintln!("macOS hang sampling exited: {status}"),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("macOS hang sampling timed out or failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_once_per_sustained_hang_and_rearms_for_a_new_ping() {
        let start = Instant::now();
        let mut probe = HangProbe::new(start);
        assert!(!probe.timed_out(start + HANG_TIMEOUT));
        assert!(probe.timed_out(start + HANG_TIMEOUT * 2));
        assert!(!probe.timed_out(start + HANG_TIMEOUT * 3));
        let mut next = HangProbe::new(start + HANG_TIMEOUT * 3);
        assert!(!next.timed_out(start + HANG_TIMEOUT * 4));
        assert!(next.timed_out(start + HANG_TIMEOUT * 5));
    }

    #[test]
    fn sleep_or_scheduler_pause_requires_fresh_timeouts() {
        let start = Instant::now();
        let mut probe = HangProbe::new(start);
        assert!(!probe.timed_out(start + HANG_TIMEOUT));
        let wake = start + Duration::from_secs(3600);
        assert!(!probe.timed_out(wake));
        assert!(!probe.timed_out(wake + HANG_TIMEOUT));
        assert!(probe.timed_out(wake + HANG_TIMEOUT * 2));
    }
}
