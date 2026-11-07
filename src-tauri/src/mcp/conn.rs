//! Application settings adapters for the shared MCP client.

use crate::settings::ChatMcpServer;
use kivio_agent_sdk::mcp::{McpClient, McpClientConfig, McpTransport};
use std::time::Duration;

pub use kivio_agent_sdk::mcp::{
    is_method_not_found, is_oauth_error, HANDSHAKE_TIMEOUT, LIST_TOOLS_TIMEOUT, PING_TIMEOUT,
};

pub(crate) fn client_config(server: &ChatMcpServer) -> McpClientConfig {
    McpClientConfig {
        transport: if server.transport == "streamable_http" {
            McpTransport::StreamableHttp {
                url: server.url.clone(),
                headers: server.headers.clone(),
            }
        } else {
            McpTransport::Stdio {
                command: server.command.clone(),
                args: server.args.clone(),
                cwd: server.cwd.as_ref().map(Into::into),
                env: server.env.clone(),
            }
        },
    }
}

pub async fn list_tools_once(
    server: &ChatMcpServer,
    http: &reqwest::Client,
) -> Result<Vec<super::types::McpTool>, String> {
    let client = McpClient::connect(client_config(server), http.clone()).await?;
    let result = client.list_tools().await;
    let closed = client.close().await;
    match result {
        Ok(tools) => closed.map(|()| tools),
        Err(error) => Err(error),
    }
}

pub async fn call_tool_once(
    server: &ChatMcpServer,
    http: &reqwest::Client,
    name: &str,
    arguments: serde_json::Value,
    timeout: Duration,
) -> Result<super::types::McpToolCallResult, String> {
    #[cfg(all(debug_assertions, target_os = "windows"))]
    crate::chat::probe::log_stack("MCP tool connecting");
    let client = McpClient::connect(client_config(server), http.clone()).await?;
    let result = client
        .call_tool(name, arguments, timeout, std::future::pending())
        .await;
    let closed = client.close().await;
    match result {
        Ok(result) => closed.map(|()| result.into()),
        Err(error) => Err(error.message),
    }
}
