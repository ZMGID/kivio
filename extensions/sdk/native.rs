//! Native SDK v1 for trusted, statically linked packages. Re-exports preserve the
//! existing owners; packages must not reach into their private implementation paths.
pub(crate) use crate::chat::model_metadata::thinking_capabilities_for_model;
pub(crate) use crate::chat::storage::{
    atomic_write, conversation_attachments_dir, conversation_file_path, conversations_dir,
    load_conversation,
};
pub(crate) use crate::chat::{model, ChatMessage};
pub(crate) use crate::extensions::{command_lease, is_active};
pub(crate) use crate::proc::NoConsoleWindow;
#[cfg(windows)]
pub(crate) use crate::proc::CREATE_NO_WINDOW;
pub(crate) use crate::settings::ModelProvider;
pub(crate) use crate::state::AppState;
