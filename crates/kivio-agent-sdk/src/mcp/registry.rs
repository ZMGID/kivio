use super::{McpClient, McpTool};
use crate::{
    agent::{
        content::{AgentToolCall, AgentToolStatus},
        execute::{PreparedToolCall, ToolPreparation, ToolProgress},
        ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    },
    model::{ModelTool, PendingToolCall},
    tools::{
        schema::validate_arguments, ToolAuthorization, ToolDefinition, ToolError, ToolFuture,
        ToolOutput, ToolRegistry,
    },
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

const MAX_WIRE_TOOL_NAME: usize = 64;
const WIRE_NAME_HASH_LEN: usize = 8;
const LONG_MCP_SERVER_ID: usize = 16;

pub fn wire_mcp_tool_id(id: &str, server_id: Option<&str>) -> String {
    sanitize_openai_tool_name(&compact_mcp_tool_id(id, server_id))
}

pub fn compact_mcp_tool_id(id: &str, server_id: Option<&str>) -> String {
    let Some(server_id) =
        server_id.filter(|s| !s.is_empty() && s.chars().count() > LONG_MCP_SERVER_ID)
    else {
        return id.to_string();
    };
    let prefix = format!("mcp__{server_id}__");
    let Some(tool) = id.strip_prefix(&prefix).filter(|t| !t.is_empty()) else {
        return id.to_string();
    };
    format!("mcp__{}__{tool}", short_stable_hash(server_id))
}

fn short_stable_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    digest
        .iter()
        .flat_map(|byte| [hex_digit(byte >> 4), hex_digit(byte & 0x0f)])
        .take(WIRE_NAME_HASH_LEN)
        .collect()
}

fn hex_digit(nibble: u8) -> char {
    b"0123456789abcdef"[nibble as usize] as char
}

pub fn sanitize_openai_tool_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        return "tool".to_string();
    }
    if trimmed.chars().count() <= MAX_WIRE_TOOL_NAME {
        return trimmed.to_string();
    }
    // Left-truncate used to collide (issue #33). Keep a stable unique suffix:
    // `<prefix>_<8-hex>` always 64 chars, deterministic, legal on the wire.
    let digest = short_stable_hash(trimmed);
    let prefix_len = MAX_WIRE_TOOL_NAME - 1 - WIRE_NAME_HASH_LEN;
    let prefix: String = trimmed.chars().take(prefix_len).collect();
    format!("{prefix}_{digest}")
}

pub fn mcp_tool_definition(server_id: &str, tool: &McpTool) -> ToolDefinition {
    let key = format!("mcp__{server_id}__{}", tool.name);
    let wire = wire_mcp_tool_id(&key, Some(server_id));
    let mut definition = ToolDefinition::new(ModelTool {
        name: wire.clone(),
        description: if tool.description.trim().is_empty() {
            format!("MCP tool {}", tool.name)
        } else {
            tool.description.clone()
        },
        input_schema: if tool.input_schema.is_null() {
            serde_json::json!({"type":"object", "properties":{}})
        } else {
            tool.input_schema.clone()
        },
    });
    definition.key = key;
    definition.label.name = wire;
    definition.label.source = "mcp".into();
    definition.label.server_id = Some(server_id.into());
    definition.label.sensitive = true;
    definition
}

impl McpClient {
    /// Atomically register this catalog. Raw server names are retained for dispatch.
    /// Server annotations never bypass authorization or enable parallel execution.
    pub async fn register_tools(
        self: &Arc<Self>,
        registry: &mut ToolRegistry,
        server_id: &str,
        authorization: Arc<dyn ToolAuthorization>,
        timeout: Duration,
    ) -> Result<(), ToolError> {
        if server_id.trim().is_empty() {
            return Err(ToolError::InvalidArguments(
                "MCP server id must not be empty".into(),
            ));
        }
        let tools = self.list_tools().await.map_err(ToolError::Execution)?;
        let mut updated = registry.snapshot();
        for tool in tools {
            if tool.name.trim().is_empty() {
                return Err(ToolError::InvalidArguments(
                    "MCP tool name must not be empty".into(),
                ));
            }
            updated.register_dynamic(
                mcp_tool_definition(server_id, &tool),
                Arc::new(RegisteredMcp {
                    client: self.clone(),
                    raw_name: tool.name,
                    authorization: authorization.clone(),
                    timeout,
                }),
            )?;
        }
        *registry = updated;
        Ok(())
    }
}

struct RegisteredMcp {
    client: Arc<McpClient>,
    raw_name: String,
    authorization: Arc<dyn ToolAuthorization>,
    timeout: Duration,
}

impl ToolExecutor for RegisteredMcp {
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            if let Some(error) = call.arguments_parse_error {
                return Ok(complete(record, AgentToolStatus::Error, error));
            }
            if let Err(error) = validate_arguments(&tool.model.input_schema, &call.arguments) {
                return Ok(complete(record, AgentToolStatus::Error, error));
            }
            if let Err(error) = self
                .authorization
                .authorize(progress, ctx, tool, &call.arguments)
                .await
            {
                return Ok(complete(
                    record,
                    AgentToolStatus::Skipped,
                    error.to_string(),
                ));
            }
            Ok(ToolPreparation::Ready(PreparedToolCall {
                record,
                arguments: call.arguments,
                timeout_ms: None,
                timeout_message: String::new(),
                output_limit: None,
                drain_on_cancel: true,
                feedback: Vec::new(),
            }))
        })
    }

    fn call<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        _ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            validate_arguments(&tool.model.input_schema, &arguments)?;
            let result = self
                .client
                .call_tool(
                    &self.raw_name,
                    arguments,
                    self.timeout,
                    progress.control.wait_inactive(),
                )
                .await
                .map_err(|error| error.message)?;
            Ok(ToolOutput {
                content: result.content,
                structured_content: result.structured_content,
                host_data: result.artifacts,
                follow_up_messages: Vec::new(),
                is_error: result.is_error,
            })
        })
    }
}

fn complete(mut record: AgentToolCall, status: AgentToolStatus, error: String) -> ToolPreparation {
    record.status = status;
    record.duration_ms = Some(0);
    record.completed_at = Some(chrono::Local::now().timestamp());
    record.error = Some(error.clone());
    ToolPreparation::Complete(record, error, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> McpTool {
        McpTool {
            name: name.into(),
            description: String::new(),
            input_schema: Value::Null,
            output_schema: None,
            annotations: Some(
                serde_json::json!({ "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false }),
            ),
        }
    }

    #[test]
    fn names_are_namespaced_and_remote_annotations_do_not_grant_capabilities() {
        let first = mcp_tool_definition("first", &tool("read"));
        let second = mcp_tool_definition("second", &tool("read"));
        assert_ne!(first.model.name, second.model.name);
        assert_ne!(first.label.name, second.label.name);
        assert!(first.label.sensitive);
        assert!(!first.parallel_safe);
        assert_eq!(
            first.model.input_schema,
            serde_json::json!({"type":"object","properties":{}})
        );
    }

    #[test]
    fn wire_names_keep_long_tool_suffixes_distinct() {
        let server = "mcp-d1ad400e-e2c7-4f34-9cd0-4e4cf1afa728";
        let a = mcp_tool_definition(server, &tool("zai-mcp-server-ui_to_artifact"));
        let b = mcp_tool_definition(server, &tool("zai-mcp-server-ui_diff_check"));
        assert_ne!(a.model.name, b.model.name);
        for name in [&a.model.name, &b.model.name] {
            assert!(name.len() <= 64);
        }
        let long = "a".repeat(80);
        assert_eq!(sanitize_openai_tool_name(&long).len(), 64);
        assert_ne!(
            sanitize_openai_tool_name(&long),
            sanitize_openai_tool_name(&format!("{long}b"))
        );
    }

    #[test]
    fn sanitized_name_collisions_remain_visible_to_registry_validation() {
        let a = mcp_tool_definition("one", &tool("read.file"));
        let b = mcp_tool_definition("one", &tool("read/file"));
        assert_eq!(a.model.name, b.model.name);
        assert_ne!(a.key, b.key);
        assert_eq!(
            compact_mcp_tool_id("unexpected", Some(&"x".repeat(40))),
            "unexpected"
        );
    }
}
