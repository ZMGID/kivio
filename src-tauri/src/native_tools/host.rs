//! Host state used by the shared shell engine. Credentials are read live.
use std::sync::Arc;
use super::background_registry::BackgroundCommandRegistry;

pub trait ShellHost: Send + Sync {
    fn playwright_extension_token(&self) -> String;
    fn background_commands_handle(&self) -> Arc<BackgroundCommandRegistry>;
}
