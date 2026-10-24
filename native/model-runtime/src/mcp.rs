//! Native assembly of the original MCP transport, pool and OAuth refresh owner.
#[path = "../../../src-tauri/src/mcp/native_policy.rs"]
pub mod native_policy;
pub use native_policy as native_registry;
#[path = "../../../src-tauri/src/mcp/conn.rs"]
pub mod conn;
#[path = "../../../src-tauri/src/mcp/manager.rs"]
pub mod manager;
#[path = "../../../src-tauri/src/mcp/result.rs"]
pub mod result;
#[path = "../../../src-tauri/src/mcp/tool_schema.rs"]
mod schema;
#[path = "../../../src-tauri/src/mcp/state.rs"]
mod state;
#[path = "../../../src-tauri/src/mcp/tool_definitions.rs"]
mod definitions;
pub mod types {
    pub use super::definitions::*;
    pub use super::schema::McpTool;
    pub use crate::tool_contract::*;
}
pub mod host {
    pub type EventSink = dyn Fn(&str, serde_json::Value) + Send + Sync;
    pub fn emit(sink: &EventSink, name: &str, value: serde_json::Value) {
        sink(name, value);
    }
}
use crate::{
    native_preferences::{self, Change},
    settings::{ChatMcpServer, Settings},
};
use manager::{McpManager, McpManagerConfig, McpSettingsPersistence};
use state::McpRuntimeState;
use std::path::PathBuf;
pub struct NativeMcp {
    root: PathBuf,
    runtime: McpRuntimeState,
    http: reqwest::Client,
}
impl NativeMcp {
    pub fn new(root: PathBuf) -> Self {
        let http = reqwest::Client::new();
        Self {
            runtime: McpRuntimeState::load(&root),
            root,
            http,
        }
    }
    fn manager(&self, settings: &Settings) -> McpManager<'_> {
        McpManager::new(
            &self.runtime,
            &self.http,
            McpManagerConfig::from_settings(settings),
            self,
        )
    }
    pub async fn call(
        &self,
        server: &ChatMcpServer,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<types::McpToolCallResult, String> {
        let settings = native_preferences::load(&self.root)?;
        self.manager(&settings)
            .mcp_call_tool(None, server, name, arguments)
            .await
    }
    pub async fn list(&self, server: &ChatMcpServer) -> Result<Vec<types::McpTool>, String> {
        let settings = native_preferences::load(&self.root)?;
        self.manager(&settings).mcp_list_tools(None, server).await
    }
    pub async fn maintain(&self) {
        let Ok(settings) = native_preferences::load(&self.root) else {
            return;
        };
        let manager = self.manager(&settings);
        manager.mcp_reap_idle(manager.mcp_idle_timeout()).await;
        manager.mcp_keepalive_http(None).await;
    }
    pub async fn shutdown(&self) {
        let settings = native_preferences::load(&self.root).unwrap_or_default();
        self.manager(&settings).mcp_disconnect_all().await;
    }
}
impl McpSettingsPersistence for NativeMcp {
    fn store_refreshed_server(&self, _: manager::McpEventSink<'_>, server: &ChatMcpServer) {
        let result = native_preferences::save(&self.root, Change::RefreshedMcpAuth(server.clone()));
        if let Err(error) = result {
            eprintln!("Failed to persist refreshed MCP authorization: {error}");
        }
    }
}

pub use crate::tool_contract::ChatToolDefinition;

#[path = "../../../src-tauri/src/mcp/native_results.rs"]
pub mod native_results;

#[path = "../../../src-tauri/src/mcp/search_policy.rs"]
pub mod search_policy;

#[path = "../../../src-tauri/src/mcp/server_policy.rs"]
pub mod server_policy;
