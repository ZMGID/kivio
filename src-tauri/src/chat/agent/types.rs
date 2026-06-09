use serde_json::Value;

use crate::chat::types::{ChatAssistantSnapshot, ChatMessageSegment, ToolCallRecord};
use crate::mcp::ChatToolDefinition;
use crate::settings::{ChatToolsConfig, ModelProvider, Settings};
use crate::skills;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunEntry {
    Send,
    Regenerate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPhase {
    ToolLoop,
    Synthesis,
    Plain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStopReason {
    Natural,
    StepLimit,
    Cancelled,
    ProviderToolsUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStreamPolicy {
    PlanningNoDoneUntilNoTools,
    SynthesisAlwaysDone,
    SynthesisDeferEmpty,
}

pub struct AgentRunConfig<'a> {
    pub entry: AgentRunEntry,
    pub state: &'a AppState,
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub generation: u64,
    pub provider: ModelProvider,
    pub model: String,
    pub runtime_messages: Vec<Value>,
    pub tools: Vec<ChatToolDefinition>,
    pub blocked_tool_calls: Vec<ChatToolDefinition>,
    pub settings: Settings,
    pub effective_chat_tools: ChatToolsConfig,
    pub language: String,
    pub has_image: bool,
    pub thinking_enabled: bool,
    pub stream_enabled: bool,
    pub max_output_tokens: u32,
    pub retry_attempts: usize,
    pub skill_registry: skills::SkillRegistry,
    pub active_skill_id: Option<String>,
    pub active_skill_detail: Option<skills::SkillDetail>,
    pub assistant_snapshot: Option<ChatAssistantSnapshot>,
    pub custom_system_prompt: String,
    pub provider_tools_fallback_system_prompt: String,
}

#[derive(Debug, Clone)]
pub struct AgentStepResult {
    pub step_number: u8,
    pub phase: AgentPhase,
    pub response_messages: Vec<Value>,
    pub tool_records: Vec<ToolCallRecord>,
    pub segments: Vec<ChatMessageSegment>,
    pub streamed: bool,
    pub stop_reason: Option<AgentStopReason>,
}

#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_records: Vec<ToolCallRecord>,
    pub segments: Vec<ChatMessageSegment>,
    pub api_messages: Vec<Value>,
    pub steps: Vec<AgentStepResult>,
    pub stream_outcome: String,
}
