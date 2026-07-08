use serde_json::Value;

use crate::chat::types::{
    ChatAssistantSnapshot, ChatMessageSegment, CompactionBoundaryRecord, ToolCallRecord,
};
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
    /// Conversation that conversation-scoped tools (todo / native workspace)
    /// target. Equals `conversation_id` for a normal chat run; for a sub-agent
    /// run it is the parent conversation (see `ToolExecutionContext`).
    pub tool_conversation_id: String,
    /// Sub-agent nesting depth (0 = top-level chat run).
    pub depth: u8,
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
    /// 每对话「思考等级」(`Some("low"|"medium"|"high")`)。`None` = 未设置，维持现状。
    /// 仅作用于答案生成（planning/synthesis），不作用于压缩摘要。
    pub thinking_level: Option<String>,
    pub stream_enabled: bool,
    pub max_output_tokens: u32,
    pub retry_attempts: usize,
    pub skill_registry: skills::SkillRegistry,
    pub active_skill_id: Option<String>,
    pub active_skill_detail: Option<skills::SkillDetail>,
    pub assistant_snapshot: Option<ChatAssistantSnapshot>,
    pub custom_system_prompt: String,
    pub provider_tools_fallback_system_prompt: String,
    /// 真实用量锚点（来自会话最后一条带 `anchor_usage` 的 assistant 消息，由 commands.rs 解析）：
    /// run 首次压缩检查前用它把上下文占用锚定到 provider 实报值。值为「上次 prompt + 该次响应」的
    /// token **总数**（含 output，见 `context_estimate::anchor_total_tokens`）。`None` = 无可用锚点
    /// （新对话 / 旧数据无 usage / 切换过 provider / 压缩边界之后），回落纯字符估算。
    pub initial_anchor_total_tokens: Option<u64>,
    /// 锚点响应**之后**（不含响应本身，响应用 output 计入锚点）新增消息的字符估算，由 commands.rs
    /// 组装 runtime_messages 时算好。与 `initial_anchor_total_tokens` 配对：`effective = 锚点 + 该 trailing`。
    pub initial_anchor_trailing_estimate: usize,
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
    /// 本轮全部模型调用（规划/合成/压缩摘要）累计的 provider 真实 usage；
    /// provider 不报告时为 None（前端回落到 chars 估算）。
    pub usage: Option<crate::chat::model::ModelUsage>,
    /// 本轮**最后一次**模型调用的 usage（真实用量锚点，落盘到 `ChatMessage.anchor_usage`）。
    /// 与累计 `usage` 区分——累计是多步之和会虚高，锚点须是单次调用值。None = provider 未报。
    pub last_step_usage: Option<crate::chat::model::ModelUsage>,
    /// 本轮发生了上下文压缩（L2 摘要）时，这里携带压缩后的**完整历史**
    /// （system + 摘要 + 受保护尾段 + 本轮后续消息 + 最终 assistant 回答）。
    /// 跨轮调用方（kivio-code 交互模式）据此**替换**自己累积的 runtime_messages，
    /// 让压缩真正跨轮生效；为 None 时维持"追加 api_messages"的旧行为。
    pub compacted_history: Option<Vec<Value>>,
    /// Agent-loop L2 compaction boundary for timeline UI persistence.
    pub compaction_boundary: Option<CompactionBoundaryRecord>,
    /// Agent-loop L2 compaction summary for `context_state.summary` persistence
    /// (L2 不再只 push boundary，run 结束时由 commands.rs 写回 summary + compression_count）。
    pub compaction_summary: Option<crate::chat::types::ConversationContextSummary>,
}
