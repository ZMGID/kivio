//! Bot identity stays on `ChatAssistant` and `conv_bot_{assistantId}`.
//! This module stores the primary marker, capability/model/approval source,
//! and private memory. Parent wires `chat/mod.rs` (already declared here),
//! `lib.rs`, `native_registry.rs`, `reply.rs`, and protocol export.

mod commands;
mod context;
mod lifecycle;
mod memory_scope;
mod policy;
mod store;
pub mod tools;
mod types;

pub use commands::*;
pub use context::{assistant_id_from_member_rows, resolve_stable_assistant_id};
/// `bot_collaboration::member_assistant_id` calls this as `bot_profile::encoded_member_bot`.
pub use context::encoded_member_bot;
pub use lifecycle::{
    apply_persona, apply_runtime_scope, commit_assistant_edit, hide_global_memory_tools,
    is_global_memory_tool, runtime_scope, LifecycleGuard,
};
pub use tools::{
    append_tools, handle_conversation_tool_call, ENTRIES, CREATE_TOOL, GET_TOOL, LIST_TOOL,
    MEMORY_CLEAR_TOOL, MEMORY_READ_TOOL, MEMORY_UPDATE_TOOL, SET_PRIMARY_TOOL, UPDATE_TOOL,
};
pub use types::*;

#[cfg(test)]
mod tests;
