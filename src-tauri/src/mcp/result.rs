use super::types::McpToolCallResult;
use serde_json::Value;

pub(crate) fn parse_tool_result(value: Value) -> McpToolCallResult {
    kivio_agent_sdk::mcp::parse_tool_result(value).into()
}
