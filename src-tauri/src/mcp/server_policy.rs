use crate::{settings::ChatMcpServer,mcp::{ChatToolDefinition,types::tool_definition_from_mcp}};
pub(crate) fn mcp_server_is_runtime_eligible(server: &ChatMcpServer) -> bool {
    if !server.enabled {
        return false;
    }
    // Package ownership must be resolved before the broader catalog-plugin prefix.
    // Use the same owner switch as package skills and lifecycle reconciliation.
    if let Some(package_id) = server
        .connector_id
        .as_deref()
        .and_then(|connector_id| connector_id.strip_prefix("plugin:package:"))
    {
        return crate::plugins::packages::owner_enabled(package_id);
    }
    if let Some(plugin_id) = server
        .connector_id
        .as_deref()
        .and_then(|connector_id| connector_id.strip_prefix("plugin:"))
    {
        return crate::plugins::is_enabled(plugin_id) && crate::plugins::is_installed(plugin_id);
    }
    true
}

pub(crate) fn eligible_mcp_servers(settings: &crate::settings::Settings) -> Vec<&ChatMcpServer> {
    if !settings.chat_tools.enabled {
        return Vec::new();
    }
    settings
        .chat_tools
        .servers
        .iter()
        .filter(|server| mcp_server_is_runtime_eligible(server))
        .collect()
}

pub(crate) fn tools_from_mcp(
    server: &ChatMcpServer,
    tools: Vec<crate::mcp::types::McpTool>,
) -> Vec<ChatToolDefinition> {
    let allowed = server
        .enabled_tools
        .iter()
        .map(|tool| tool.as_str())
        .collect::<Vec<_>>();
    tools
        .into_iter()
        .filter(|tool| allowed.is_empty() || allowed.contains(&tool.name.as_str()))
        .map(|tool| tool_definition_from_mcp(server, tool))
        .collect()
}
