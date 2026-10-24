//! Native lifecycle adapter for Kivio's shared non-blocking hook dispatcher.
use crate::hooks::{HookDispatcher, HookEvent, RunContext};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
static FAILURES: Mutex<()> = Mutex::new(());
fn path(root: &Path) -> PathBuf {
    root.join("hook-failures.json")
}
pub fn failures(root: &Path) -> Result<Vec<serde_json::Value>, String> {
    let _guard = FAILURES.lock().unwrap_or_else(|e| e.into_inner());
    read_failures(root)
}
fn read_failures(root: &Path) -> Result<Vec<serde_json::Value>, String> {
    match std::fs::read(path(root)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}
pub struct Lifecycle {
    dispatcher: Option<HookDispatcher>,
    completed: bool,
}
impl Lifecycle {
    pub fn begin(root: PathBuf, hooks: &[crate::settings::HookDef], context: RunContext) -> Self {
        let dispatcher = dispatcher(root, hooks, context);
        let this = Self {
            dispatcher,
            completed: false,
        };
        this.emit(HookEvent::AgentStart);
        this.emit(HookEvent::TurnStart);
        this.emit(HookEvent::MessageStart);
        this
    }
    fn emit(&self, event: HookEvent) {
        if let Some(dispatcher) = &self.dispatcher {
            dispatcher.dispatch(event, None, Some(1), None);
        }
    }
    pub fn complete(&mut self) {
        self.completed = true;
    }
}
impl Drop for Lifecycle {
    fn drop(&mut self) {
        if !self.completed {
            if let Some(dispatcher) = &self.dispatcher {
                dispatcher.cancel();
            }
        }
        self.emit(HookEvent::MessageEnd);
        self.emit(HookEvent::TurnEnd);
        self.emit(HookEvent::AgentEnd);
    }
}

/// Build the same failure-reporting dispatcher for the shared Agent loop.
pub fn dispatcher(root: PathBuf, hooks: &[crate::settings::HookDef], context: RunContext) -> Option<HookDispatcher> {
    HookDispatcher::with_sink(
            hooks,
            context,
            Box::new(move |failure| {
                let _guard = FAILURES.lock().unwrap_or_else(|e| e.into_inner());
                let result = (|| {
                    let mut entries = read_failures(&root)?;
                    entries.push(failure);
                    if entries.len() > 64 {
                        entries.drain(..entries.len() - 64);
                    }
                    crate::file_io::atomic_write(
                        &path(&root),
                        &serde_json::to_string(&entries).map_err(|e| e.to_string())?,
                        "hook failures",
                    )
                })();
                if let Err(error) = result {
                    eprintln!("Cannot persist hook failure: {error}");
                }
            }),
            &tokio::runtime::Handle::current(),
        )
}
