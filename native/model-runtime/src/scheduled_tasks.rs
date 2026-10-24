//! Native host assembly of the original scheduled-task state machine.
#[path = "../../../src-tauri/src/scheduled_tasks/rule.rs"]
mod rule;
#[path = "../../../src-tauri/src/scheduled_tasks/store.rs"]
mod store;
#[path = "../../../src-tauri/src/scheduled_tasks/types.rs"]
pub mod types;
include!("../../../src-tauri/src/scheduled_tasks/core.rs");

/// One definition/run owner per native profile, including multiple viewports.
pub fn shared(directory: std::path::PathBuf) -> Result<std::sync::Arc<ScheduledTasks>, String> {
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static OWNERS: OnceLock<
        Mutex<std::collections::HashMap<std::path::PathBuf, Weak<ScheduledTasks>>>,
    > = OnceLock::new();
    let key = directory
        .ancestors()
        .find_map(|ancestor| {
            ancestor.canonicalize().ok().and_then(|base| {
                directory
                    .strip_prefix(ancestor)
                    .ok()
                    .map(|suffix| base.join(suffix))
            })
        })
        .unwrap_or(directory.clone());
    let mut owners = OWNERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(owner) = owners.get(&key).and_then(Weak::upgrade) {
        return Ok(owner);
    }
    let owner = Arc::new(ScheduledTasks::load(directory)?);
    owners.retain(|_, owner| owner.strong_count() > 0);
    owners.insert(key, Arc::downgrade(&owner));
    Ok(owner)
}

/// Validate before creating a target conversation, avoiding orphan conversations.
pub fn validated_name(input: &types::ScheduledTaskInput) -> Result<String, String> {
    Ok(validate_input(input, now_secs())?.name)
}
