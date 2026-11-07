use crate::model::ModelArtifact;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallResult {
    pub content: String,
    pub is_error: bool,
    pub raw: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<ModelArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
    #[serde(default, rename = "outputSchema")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
}
