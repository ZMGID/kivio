//! Persistent peer delivery and multi-Bot rooms.
//!
//! Rakazo keeps a durable peer Bot (own thread, own duties) separate from a
//! turn-scoped child. Hermes here is only an external CLI host, not this
//! queue. Recipient work goes through Chat's idle send, not a second model
//! client and not `sub_agent`.

mod commands;
mod flow;
mod machine;
mod observe;
mod runtime;
mod seam;
mod store;
#[cfg(test)]
pub(crate) use store::Store;
pub mod tools;
mod types;

pub use commands::*;
pub use observe::{
    clean_peer_body, peer_authored_content, peer_envelope, peer_user_message,
    project_trusted_legacy_peers, provider_user_text, trusted_legacy_delivery,
};
pub use runtime::{
    collaboration_conversation, member_assistant_id, member_conversation_identity,
    recorded_deliveries, spawn, BotCollaboration,
};
pub use seam::{reply_seam, ReplySeam};
pub use tools::{append_tools, handle_conversation_tool_call, ENTRIES};
pub use types::*;

#[cfg(test)]
mod tests;
