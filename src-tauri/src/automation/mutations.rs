//! One write owner for automation storage and its runtime side effects.

use tauri::{AppHandle, Emitter};

use super::hotkeys::fingerprint as hotkey_fingerprint;
use super::runner;
use super::storage;
use super::types::Automation;

pub(super) fn save(app: &AppHandle, automation: Automation) -> Result<Automation, String> {
    let previous = storage::get(app, &automation.id).ok();
    let saved = storage::save(app, automation)?;
    let hotkey_changed = previous
        .as_ref()
        .map(|old| hotkey_fingerprint(old) != hotkey_fingerprint(&saved))
        .unwrap_or(saved.enabled);
    if hotkey_changed {
        refresh_hotkeys(app);
    }
    Ok(saved)
}

pub(super) fn delete(app: &AppHandle, id: &str) -> Result<(), String> {
    runner::cancel(app, id)?;
    storage::delete(app, id)?;
    refresh_hotkeys(app);
    Ok(())
}

pub(super) fn set_enabled(app: &AppHandle, id: &str, enabled: bool) -> Result<Automation, String> {
    let saved = storage::set_enabled(app, id, enabled)?;
    refresh_hotkeys(app);
    Ok(saved)
}

fn refresh_hotkeys(app: &AppHandle) {
    if let Err(err) = crate::shortcuts::register_hotkeys(app) {
        let _ = app.emit("hotkey-warning", err);
    }
}
