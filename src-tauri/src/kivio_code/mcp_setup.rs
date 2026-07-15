//! MCP integration seam for kivio-code (headless CLI).
//!
//! This module owns ALL MCP wiring for the headless CLI so MCP support can be
//! built out without touching `kivio_code/mod.rs`. `mod.rs` only calls
//! [`collect_mcp_tools`] at startup and the executor calls [`dispatch_mcp_tool`];
//! everything else (server connection, lifecycle, tool naming) lives here.
//!
//! It mirrors the GUI chat path (`chat/commands.rs::list_enabled_tool_catalog` +
//! `call_tool`) but uses the persistent MCP session manager
//! (`AppState::mcp_list_tools` / `AppState::mcp_call_tool`) with the unit
//! [`McpEventSink`] (`()`) — the CLI needs no Tauri UI events, so no `AppHandle`
//! is required. Connecting may spawn stdio child processes / make HTTP calls;
//! the manager already time-boxes those (per `tool_timeout_ms`), and per-server
//! failures are isolated so a broken server never crashes the CLI.

use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;
#[cfg(test)]
use crate::settings::ChatMcpServer;
use crate::settings::Settings;
use crate::state::AppState;
use serde_json::Value;

/// Collect the same enabled tool catalog used by the GUI chat path.
///
/// Runtime eligibility, per-tool settings, concurrent discovery, reconnect
/// backoff, and the matching last-known schema fallback all live in the shared
/// MCP registry. The headless CLI deliberately adds no second timeout or
/// fallback layer.
pub async fn collect_mcp_tools(state: &AppState, settings: &Settings) -> Vec<ChatToolDefinition> {
    crate::mcp::registry::collect_enabled_mcp_tool_defs(state, &(), settings)
        .await
        .0
}

/// Per-server connection status for the interactive `/mcp` command. A read-only
/// view over the configured server list + a live connect probe, so `/mcp` can
/// show each server's transport, enabled flag, connection
/// outcome, and the tool names it advertises (respecting `enabled_tools`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerStatus {
    /// Server id (settings key).
    pub id: String,
    /// Human-readable server name.
    pub name: String,
    /// Transport (`stdio` / `streamable_http`).
    pub transport: String,
    /// Whether the server is enabled in settings.
    pub enabled: bool,
    /// `true` if the connect+list probe succeeded; `false` on error/timeout.
    pub connected: bool,
    /// Tool names the server advertises (after the `enabled_tools` allow-list).
    pub tools: Vec<String>,
    /// Connect/list error message, if the probe failed.
    pub error: Option<String>,
}

/// Probe configured & enabled MCP servers for the interactive `/mcp` command.
///
/// Like [`collect_mcp_tools`] this reads `settings.chat_tools` (MCP only when
/// `chat_tools.enabled`) and connects each enabled server concurrently through
/// the session manager, but returns a per-server status (name, transport,
/// enabled, connected, tool names, error) instead of a flat tool
/// list. Disabled servers are reported too (as `enabled = false`, not probed)
/// so the user sees the full configured set. Returns an empty vec when chat
/// tools are off or no servers are configured.
pub async fn collect_mcp_status(state: &AppState, settings: &Settings) -> Vec<McpServerStatus> {
    if !settings.chat_tools.enabled {
        return Vec::new();
    }
    if settings.chat_tools.servers.is_empty() {
        return Vec::new();
    }

    let probes = settings.chat_tools.servers.iter().map(|server| async move {
        // Runtime-ineligible servers are listed but never connected. This includes disabled
        // settings entries and plugin-backed entries whose plugin is disabled or uninstalled.
        if !crate::mcp::registry::mcp_server_is_runtime_eligible(server) {
            return McpServerStatus {
                id: server.id.clone(),
                name: server.name.clone(),
                transport: server.transport.clone(),
                enabled: server.enabled,
                connected: false,
                tools: Vec::new(),
                error: server
                    .enabled
                    .then(|| "backing plugin is disabled or not installed".to_string()),
            };
        }
        let result = state.mcp_list_tools(&(), server).await;
        match result {
            Ok(server_tools) => {
                let tools = crate::mcp::registry::tools_from_mcp(server, server_tools)
                    .into_iter()
                    .map(|tool| tool.name)
                    .collect();
                McpServerStatus {
                    id: server.id.clone(),
                    name: server.name.clone(),
                    transport: server.transport.clone(),
                    enabled: true,
                    connected: true,
                    tools,
                    error: None,
                }
            }
            Err(err) => McpServerStatus {
                id: server.id.clone(),
                name: server.name.clone(),
                transport: server.transport.clone(),
                enabled: true,
                connected: false,
                tools: Vec::new(),
                error: Some(err),
            },
        }
    });

    futures::future::join_all(probes).await
}

/// Dispatch a tool call to its MCP server.
///
/// Returns `Err(msg)` ONLY when the tool is not an MCP tool (so the executor's
/// unknown-tool error still fires for truly-unknown tools): a non-`mcp` source,
/// a missing `server_id`, or a `server_id` that does not match an enabled
/// configured server. When the MCP server handled the call, returns
/// `Ok(result)` — including tool-level errors, which are represented inside the
/// returned `McpToolCallResult` (`is_error = true`), not as a Rust `Err`.
pub async fn dispatch_mcp_tool(
    state: &AppState,
    tool: &ChatToolDefinition,
    arguments: &Value,
) -> Result<McpToolCallResult, String> {
    // Not an MCP tool → let the executor fall through to its unknown-tool error.
    if tool.source != "mcp" {
        return Err(format!("tool '{}' is not an MCP tool", tool.name));
    }
    let server_id = tool
        .server_id
        .as_deref()
        .ok_or_else(|| "MCP tool has no server id".to_string())?;

    // Resolve the target server from settings (must be present AND enabled),
    // matching the GUI's `call_tool`.
    let settings = state.settings_read().clone();
    let server = settings
        .chat_tools
        .servers
        .iter()
        .find(|server| {
            server.id == server_id && crate::mcp::registry::mcp_server_is_runtime_eligible(server)
        })
        .cloned()
        .ok_or_else(|| format!("MCP server '{server_id}' is disabled or missing"))?;

    // Use the actual MCP tool name (`tool.name`), NOT the openai-facing id
    // (`mcp__{server}__{tool}`). Route through the persistent session pool:
    // get-or-connect, liveness probe, transparent reconnect, per-server
    // isolation. The unit sink means no Tauri events are emitted.
    state
        .mcp_call_tool(&(), &server, &tool.name, arguments.clone())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn headless_state(settings: Settings) -> AppState {
        let dir = std::env::temp_dir().join(format!("kivio-code-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        AppState::new_headless(settings, dir.join("usage"))
    }

    fn mcp_tool(server_id: &str, name: &str) -> ChatToolDefinition {
        ChatToolDefinition {
            id: format!("mcp__{server_id}__{name}"),
            name: name.to_string(),
            description: String::new(),
            source: "mcp".to_string(),
            server_id: Some(server_id.to_string()),
            server_name: Some("Test".to_string()),
            input_schema: serde_json::json!({ "type": "object" }),
            sensitive: false,
            annotations: None,
            output_schema: None,
        }
    }

    fn stdio_server(id: &str, enabled: bool) -> ChatMcpServer {
        ChatMcpServer {
            id: id.to_string(),
            name: format!("Server {id}"),
            enabled,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "true".to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            headers: HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        }
    }

    fn unavailable_plugin_server(id: &str) -> ChatMcpServer {
        let mut server = stdio_server(id, true);
        server.connector_id = Some("plugin:__missing_mcp_test_plugin__".to_string());
        server
    }

    #[tokio::test]
    async fn collect_returns_empty_when_chat_tools_disabled() {
        let mut settings = Settings::default();
        settings.chat_tools.enabled = false;
        settings.chat_tools.servers = vec![stdio_server("srv", true)];
        let state = headless_state(settings.clone());
        let tools = collect_mcp_tools(&state, &settings).await;
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn collect_returns_empty_when_no_servers_configured() {
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = Vec::new();
        let state = headless_state(settings.clone());
        let tools = collect_mcp_tools(&state, &settings).await;
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn collect_status_empty_when_chat_tools_disabled() {
        let mut settings = Settings::default();
        settings.chat_tools.enabled = false;
        settings.chat_tools.servers = vec![stdio_server("srv", true)];
        let state = headless_state(settings.clone());
        let status = collect_mcp_status(&state, &settings).await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn collect_status_empty_when_no_servers() {
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = Vec::new();
        let state = headless_state(settings.clone());
        let status = collect_mcp_status(&state, &settings).await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn collect_status_lists_disabled_server_without_probing() {
        // A disabled server is still surfaced (so the user sees the full set),
        // but is reported as disabled + not connected, never probed.
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = vec![stdio_server("srv", false)];
        let state = headless_state(settings.clone());
        let status = collect_mcp_status(&state, &settings).await;
        assert_eq!(status.len(), 1);
        let s = &status[0];
        assert_eq!(s.id, "srv");
        assert_eq!(s.name, "Server srv");
        assert_eq!(s.transport, "stdio");
        assert!(!s.enabled);
        assert!(!s.connected);
        assert!(s.tools.is_empty());
        assert!(s.error.is_none());
    }

    #[tokio::test]
    async fn collect_status_reports_runtime_ineligible_plugin_without_probing() {
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = vec![unavailable_plugin_server("plugin-srv")];
        let state = headless_state(settings.clone());

        let status = collect_mcp_status(&state, &settings).await;

        assert_eq!(status.len(), 1);
        assert!(status[0].enabled);
        assert!(!status[0].connected);
        assert_eq!(
            status[0].error.as_deref(),
            Some("backing plugin is disabled or not installed")
        );
    }

    #[tokio::test]
    async fn collect_ignores_disabled_servers() {
        // Only a disabled server is configured → no connection attempt, empty.
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = vec![stdio_server("srv", false)];
        let state = headless_state(settings.clone());
        let tools = collect_mcp_tools(&state, &settings).await;
        assert!(tools.is_empty());
    }

    #[test]
    fn tools_from_mcp_filters_by_enabled_tools_and_sets_shape() {
        let mut server = stdio_server("srv", true);
        server.enabled_tools = vec!["echo".to_string()];
        let mcp_tools = vec![
            crate::mcp::types::McpTool {
                name: "echo".to_string(),
                description: "Echo back".to_string(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: None,
                annotations: None,
            },
            crate::mcp::types::McpTool {
                name: "hidden".to_string(),
                description: String::new(),
                input_schema: Value::Null,
                output_schema: None,
                annotations: None,
            },
        ];
        let defs = crate::mcp::registry::tools_from_mcp(&server, mcp_tools);
        assert_eq!(defs.len(), 1, "non-enabled tool must be filtered out");
        let def = &defs[0];
        assert_eq!(def.name, "echo");
        assert_eq!(def.source, "mcp");
        assert_eq!(def.server_id.as_deref(), Some("srv"));
        assert_eq!(def.id, "mcp__srv__echo");
        // openai-facing name is the sanitized id, not the bare tool name.
        assert_eq!(def.openai_tool_name(), "mcp__srv__echo");
    }

    #[test]
    fn tools_from_mcp_allows_all_when_enabled_tools_empty() {
        let server = stdio_server("srv", true);
        let mcp_tools = vec![
            crate::mcp::types::McpTool {
                name: "a".to_string(),
                description: String::new(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: None,
                annotations: None,
            },
            crate::mcp::types::McpTool {
                name: "b".to_string(),
                description: String::new(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: None,
                annotations: None,
            },
        ];
        let defs = crate::mcp::registry::tools_from_mcp(&server, mcp_tools);
        assert_eq!(defs.len(), 2);
    }

    #[tokio::test]
    async fn dispatch_errs_for_non_mcp_tool() {
        let state = headless_state(Settings::default());
        let mut tool = mcp_tool("srv", "echo");
        tool.source = "native".to_string();
        let err = dispatch_mcp_tool(&state, &tool, &serde_json::json!({}))
            .await
            .expect_err("non-mcp tool must Err so the executor errors as unknown");
        assert!(err.contains("not an MCP tool"), "got: {err}");
    }

    #[tokio::test]
    async fn dispatch_errs_for_unknown_server() {
        // MCP tool whose server_id is not in settings → Err (so the unknown-tool
        // path still fires); never panics, never hangs trying to connect.
        let state = headless_state(Settings::default());
        let tool = mcp_tool("does-not-exist", "echo");
        let err = dispatch_mcp_tool(&state, &tool, &serde_json::json!({}))
            .await
            .expect_err("unknown server must Err");
        assert!(err.contains("disabled or missing"), "got: {err}");
    }

    #[tokio::test]
    async fn dispatch_errs_for_disabled_server() {
        // Server exists but is disabled → treated as missing.
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = vec![stdio_server("srv", false)];
        let state = headless_state(settings);
        let tool = mcp_tool("srv", "echo");
        let err = dispatch_mcp_tool(&state, &tool, &serde_json::json!({}))
            .await
            .expect_err("disabled server must Err");
        assert!(err.contains("disabled or missing"), "got: {err}");
    }

    #[tokio::test]
    async fn dispatch_errs_for_runtime_ineligible_plugin_server() {
        let mut settings = Settings::default();
        settings.chat_tools.enabled = true;
        settings.chat_tools.servers = vec![unavailable_plugin_server("plugin-srv")];
        let state = headless_state(settings);
        let tool = mcp_tool("plugin-srv", "echo");

        let err = dispatch_mcp_tool(&state, &tool, &serde_json::json!({}))
            .await
            .expect_err("runtime-ineligible plugin server must Err");

        assert!(err.contains("disabled or missing"), "got: {err}");
    }

    #[tokio::test]
    async fn dispatch_errs_for_missing_server_id() {
        let state = headless_state(Settings::default());
        let mut tool = mcp_tool("srv", "echo");
        tool.server_id = None;
        let err = dispatch_mcp_tool(&state, &tool, &serde_json::json!({}))
            .await
            .expect_err("missing server id must Err");
        assert!(err.contains("no server id"), "got: {err}");
    }
}
