use super::content::{AgentToolCall, ContentBlock};
use serde_json::Value;

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
    pub tools: Vec<crate::chat::tools::ToolDefinition>,
    pub blocked_tool_calls: Vec<crate::chat::tools::ToolDefinition>,
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
        tools: Vec<crate::chat::model::ModelTool>,
        tool_choice: crate::chat::model::ToolChoice,
        label: &str,
        delivery: super::provider_runtime::ModelDelivery,
    ) -> super::provider_runtime::ModelCall {
        super::provider_runtime::ModelCall {
            provider_id: self.provider_id.clone(),
            model: self.model.clone(),
            history,
            tools,
            delivery,
            options: crate::chat::model::GenerateOptions {
                thinking_enabled: self.thinking_enabled,
                thinking_level: self.thinking_level.clone(),
                builtin_web_search: self.builtin_web_search,
                tool_choice,
                max_tokens: self.max_output_tokens,
                ..Default::default()
            },
            metadata: crate::chat::model::RequestMetadata {
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
    pub(crate) final_message: Option<crate::chat::model::ModelMessage>,
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_records: Vec<AgentToolCall>,
    pub segments: Vec<ContentBlock>,
    pub api_messages: Vec<Value>,
    pub status: AgentRunStatus,
    /// 本轮全部模型调用（规划/合成/压缩摘要）累计的 provider 真实 usage；
    /// provider 不报告时为 None（前端回落到 chars 估算）。
    pub usage: Option<crate::chat::model::ModelUsage>,
    /// Sum of complete per-request cache telemetry pairs; independent of billing usage.
    pub cache_pairs: Option<(u64, u64)>,
    /// 本轮**最后一次**模型调用的 usage（真实用量锚点，落盘到 `ChatMessage.anchor_usage`）。
    /// 与累计 `usage` 区分——累计是多步之和会虚高，锚点须是单次调用值。None = provider 未报。
    pub last_step_usage: Option<crate::chat::model::ModelUsage>,
    pub history: Vec<super::history::RuntimeMessage>,
    pub compacted: bool,
    /// 模型原生生成的图片（Gemini native image gen，任务 07-24）：跨轮累积的
    /// `GenerateOutput.images`，reply 侧落成 assistant 消息级 artifacts。空 = 未出图。
    pub images: Vec<crate::chat::model::GeneratedImageData>,
}
