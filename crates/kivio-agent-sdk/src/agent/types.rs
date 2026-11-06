use super::content::{AgentToolCall, ContentBlock};

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
pub enum AgentStreamPolicy {
    PlanningNoDoneUntilNoTools,
    SynthesisAlwaysDone,
    SynthesisDeferEmpty,
}

#[derive(Clone)]
pub struct AgentRunConfig<'a> {
    pub provider_runtime: &'a dyn super::provider_runtime::ModelRuntime,
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub generation: u64,
    pub provider_id: String,
    pub provider_label: String,
    pub api_format: String,
    pub model: String,
    pub runtime_messages: Vec<super::history::RuntimeMessage>,
    pub tools: Vec<crate::tools::ToolDefinition>,
    pub blocked_tool_calls: Vec<crate::tools::ToolDefinition>,
    pub thinking_enabled: bool,
    pub thinking_level: Option<String>,
    pub builtin_web_search: bool,
    pub max_output_tokens: u32,
    pub initial_cache_usage: Option<(u64, u64)>,
    pub report_context: bool,
}

impl AgentRunConfig<'_> {
    pub(crate) fn builtin_web_search_active(&self) -> bool {
        self.builtin_web_search
    }

    pub(crate) fn model_call(
        &self,
        history: Vec<super::history::RuntimeMessage>,
        tools: Vec<crate::model::ModelTool>,
        tool_choice: crate::model::ToolChoice,
        label: &str,
        delivery: super::provider_runtime::ModelDelivery,
    ) -> super::provider_runtime::ModelCall {
        super::provider_runtime::ModelCall {
            provider_id: self.provider_id.clone(),
            model: self.model.clone(),
            history,
            tools,
            delivery,
            options: crate::model::GenerateOptions {
                thinking_enabled: self.thinking_enabled,
                thinking_level: self.thinking_level.clone(),
                builtin_web_search: self.builtin_web_search,
                tool_choice,
                max_tokens: self.max_output_tokens,
                ..Default::default()
            },
            metadata: crate::model::RequestMetadata {
                label: label.into(),
                conversation_id: Some(self.conversation_id.clone()),
                message_id: Some(self.message_id.clone()),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRunStatus {
    Completed,
    Recovered,
    CompactionBlocked,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub(crate) final_message: Option<crate::model::ModelMessage>,
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_records: Vec<AgentToolCall>,
    pub segments: Vec<ContentBlock>,
    pub generated_history: Vec<super::history::RuntimeMessage>,
    pub status: AgentRunStatus,
    /// Accumulated reported model usage. Absent when no request reports usage.
    pub usage: Option<crate::model::ModelUsage>,
    /// Sum of complete per-request cache telemetry pairs; independent of billing usage.
    pub cache_pairs: Option<(u64, u64)>,
    /// Usage from the latest model request, separate from accumulated billing usage.
    pub last_step_usage: Option<crate::model::ModelUsage>,
    pub history: Vec<super::history::RuntimeMessage>,
    pub compacted: bool,
    /// Generated image payloads accumulated during this run. The host owns persistence.
    pub images: Vec<crate::model::GeneratedImageData>,
}
