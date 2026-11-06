use serde_json::Value;

use crate::model::{ModelArtifact, WebCitation};

use super::steering::SteeringMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentPhase {
    Plain,
    ToolLoop,
    Synthesis,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContentKind {
    Text,
    Reasoning,
    Tool,
    Search(ProviderSearch),
    Input {
        kind: InputKind,
        message: SteeringMessage,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Steer,
    FollowUp,
}

/// Stable ordinal reserved by the producer, including empty blocks and gaps.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentBlock {
    pub id: u32,
    pub kind: ContentKind,
    pub phase: ContentPhase,
    pub step_number: Option<u8>,
    pub round: Option<u32>,
    pub text: Option<String>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSearch {
    pub id: String,
    pub provider: String,
    pub queries: Vec<String>,
    pub citations: Vec<WebCitation>,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentToolStatus {
    Pending,
    Running,
    Success,
    Error,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct ToolArgumentState {
    pub model_name: String,
    pub complete: bool,
}

/// Execution facts. Output is actual tool content; the projection applies its display limit.
#[derive(Debug, Clone)]
pub struct AgentToolCall {
    pub id: String,
    pub name: String,
    pub source: String,
    pub server_id: Option<String>,
    pub arguments: String,
    pub status: AgentToolStatus,
    pub output: Option<String>,
    pub output_limit: Option<usize>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub round: u32,
    pub sensitive: bool,
    pub artifacts: Vec<ModelArtifact>,
    pub structured_content: Option<Value>,
    pub draft: Option<ToolArgumentState>,
}
