//! Host-independent model and tool execution.
//!
//! Start with [`agent::run_agent_loop`], [`agent::session::NoSessionPolicy`],
//! and [`model::openai::OpenAiChatProvider`]. Hosts supply authorization,
//! run control, event observation, transport configuration, and persistence.

pub mod agent;
pub mod context;
pub mod dsml_tools;
pub mod model;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "skills")]
pub mod skills;
pub mod tools;
