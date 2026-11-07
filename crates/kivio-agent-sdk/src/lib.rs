//! Host-independent model and tool execution.
//!
//! Start with [`agent::AgentSession`] and [`agent::AgentSessionConfig`]. Hosts
//! supply a model runtime, explicitly authorized tools, and persistence.
//! Existing hosts with their own run ownership can use [`agent::run_agent_loop`].

pub mod agent;
pub mod context;
pub mod dsml_tools;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod model;
#[cfg(feature = "skills")]
pub mod skills;
pub mod tools;
