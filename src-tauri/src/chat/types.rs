use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::model::ModelMessage;
use crate::mcp::types::ChatToolArtifact;

fn default_context_usage_status() -> String {
    "unknown".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextUsageSegment {
    pub id: String,
    pub label: String,
    pub estimated_tokens: usize,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationContextSummary {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub source_message_ids: Vec<String>,
    pub source_until_message_id: String,
    pub token_estimate_before: usize,
    pub token_estimate_after: usize,
    pub created_at: i64,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversationContextState {
    #[serde(default)]
    pub estimated_input_tokens: usize,
    #[serde(default)]
    pub context_window_tokens: Option<usize>,
    #[serde(default)]
    pub context_window_estimated: bool,
    #[serde(default)]
    pub usage_ratio: Option<f32>,
    #[serde(default = "default_context_usage_status")]
    pub status: String,
    #[serde(default)]
    pub segments: Vec<ContextUsageSegment>,
    #[serde(default)]
    pub last_measured_at: i64,
    #[serde(default)]
    pub last_compressed_at: Option<i64>,
    #[serde(default)]
    pub compressed_message_count: usize,
    #[serde(default)]
    pub summary: Option<ConversationContextSummary>,
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl Default for AgentTodoStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AgentTodoItem {
    pub id: String,
    /// 一行 subject（保留字段名 `content` 向后兼容；概念上是任务标题）。
    pub content: String,
    /// 可选的详细描述（subject/description 分离）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub status: AgentTodoStatus,
    /// 本条完成后才能开始的任务 id（正向依赖边）。写侧自动同步对端 blocked_by。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
    /// 必须先完成才能开始本条的任务 id（反向依赖边）。写侧自动同步对端 blocks。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// 认领者（P3 subagent 预留，本期不接消费方）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AgentTodoState {
    #[serde(default)]
    pub items: Vec<AgentTodoItem>,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanMode {
    Act,
    Plan,
    Orchestrate,
}

impl Default for AgentPlanMode {
    fn default() -> Self {
        Self::Act
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanStatus {
    Empty,
    Draft,
    Approved,
}

impl Default for AgentPlanStatus {
    fn default() -> Self {
        Self::Empty
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AgentPlanState {
    #[serde(default)]
    pub mode: AgentPlanMode,
    #[serde(default)]
    pub status: AgentPlanStatus,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
}

/// 工具调用状态（保存在 assistant message metadata 中）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    Running,
    Success,
    Error,
    Cancelled,
    Skipped,
}

/// Chat 工具调用记录。字段保持 snake_case 存储，前端可同时兼容 camelCase 事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub arguments: String,
    pub status: ToolCallStatus,
    #[serde(default)]
    pub result_preview: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub round: u32,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub artifacts: Vec<ChatToolArtifact>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub span_id: Option<String>,
    #[serde(default)]
    pub structured_content: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageSegmentKind {
    Text,
    Reasoning,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageSegmentPhase {
    Auxiliary,
    Plain,
    ToolLoop,
    Synthesis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessageSegment {
    pub id: String,
    pub kind: ChatMessageSegmentKind,
    pub phase: ChatMessageSegmentPhase,
    pub order: u32,
    #[serde(default)]
    pub step_number: Option<u8>,
    #[serde(default)]
    pub round: Option<u32>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

/// 对话消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String, // "user" | "assistant"
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<ChatToolArtifact>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default)]
    pub segments: Vec<ChatMessageSegment>,
    /// Hidden OpenAI-compatible transcript messages produced while answering this UI message.
    ///
    /// Tool calls stay rendered as metadata in `tool_calls`, but strict tool-calling
    /// providers such as DeepSeek need the original assistant `tool_calls` messages
    /// and matching `role: tool` results replayed in later requests.
    #[serde(default)]
    pub api_messages: Vec<Value>,
    /// Canonical provider-agnostic transcript messages produced while answering this UI message.
    ///
    /// New replay paths prefer this field. `api_messages` stays readable for legacy
    /// conversations and OpenAI-compatible debugging.
    #[serde(default)]
    pub model_messages: Vec<ModelMessage>,
    #[serde(default)]
    pub active_skill_id: Option<String>,
    /// How this assistant message was produced: `send` or `regenerate`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_entry: Option<String>,
    /// Final stream outcome for this assistant message: `completed`, `cancelled`, or `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_outcome: Option<String>,
    /// Provider-reported usage accumulated across all model calls of this reply
    /// (planning/synthesis/compaction). None when the provider reports no usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::chat::model::ModelUsage>,
    pub timestamp: i64,
}

/// 附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    #[serde(rename = "type")]
    pub attachment_type: String, // "image" | "file"
    pub name: String,
    pub path: String, // 相对于对话附件目录的路径
}

/// 完整对话数据（存储在 conversations/{id}.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub provider_id: String,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub active_skill_id: Option<String>,
    #[serde(default)]
    pub assistant_id: Option<String>,
    #[serde(default)]
    pub assistant_snapshot: Option<ChatAssistantSnapshot>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub context_state: ConversationContextState,
    #[serde(default)]
    pub agent_todo_state: AgentTodoState,
    #[serde(default)]
    pub agent_plan_state: AgentPlanState,
}

/// 对话列表项（index.json 中的元数据）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationListItem {
    pub id: String,
    pub title: String,
    pub preview: String, // 最后一条消息的前 100 字符
    pub provider_id: String,
    pub model: String,
    pub message_count: usize,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub assistant_id: Option<String>,
    #[serde(default)]
    pub assistant_name: Option<String>,
}

/// 对话索引文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversationIndex {
    pub conversations: Vec<ConversationListItem>,
}

/// Chat 项目。`folder` 保留用于旧对话兼容，真实项目归属使用 `project_id`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatProject {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub root_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 项目索引文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatProjectIndex {
    pub projects: Vec<ChatProject>,
}

/// 可复用 Chat 助手配置。存储字段保持 snake_case，与 Conversation JSON 一致。
/// 重建后只保留：身份(name/desc/icon/color) + 系统提示词 + 模型 + 勾选的 MCP/技能白名单。
/// 旧文件里的 author/tags/quick_commands/data_connectors/knowledge_skills 等字段由 serde 忽略未知字段容错。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAssistant {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    /// 该助手允许使用的 MCP 服务器 id 白名单。空 = 不可用任何 MCP。
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    /// 该助手允许激活的技能 id 白名单。空 = 不可用任何技能。
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub installed: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub built_in: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 对话创建时冻结的助手配置，避免后续编辑助手静默改变旧对话行为。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAssistantSnapshot {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
}

impl From<&ChatAssistant> for ChatAssistantSnapshot {
    fn from(assistant: &ChatAssistant) -> Self {
        Self {
            id: assistant.id.clone(),
            name: assistant.name.clone(),
            description: assistant.description.clone(),
            source: assistant.source.clone(),
            system_prompt: assistant.system_prompt.clone(),
            provider_id: assistant.provider_id.clone(),
            model: assistant.model.clone(),
            mcp_server_ids: assistant.mcp_server_ids.clone(),
            skill_ids: assistant.skill_ids.clone(),
        }
    }
}

/// 助手索引文件结构。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatAssistantIndex {
    pub assistants: Vec<ChatAssistant>,
}

fn default_true() -> bool {
    true
}

impl From<&Conversation> for ConversationListItem {
    fn from(conv: &Conversation) -> Self {
        let preview = conv
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user" || m.role == "assistant")
            .map(|m| {
                let text = m.content.trim();
                truncate_preview(text, 100)
            })
            .unwrap_or_default();

        ConversationListItem {
            id: conv.id.clone(),
            title: conv.title.clone(),
            preview,
            provider_id: conv.provider_id.clone(),
            model: conv.model.clone(),
            message_count: conv.messages.len(),
            created_at: conv.created_at,
            updated_at: conv.updated_at,
            pinned: conv.pinned,
            folder: conv.folder.clone(),
            project_id: conv.project_id.clone(),
            assistant_id: conv.assistant_id.clone(),
            assistant_name: conv
                .assistant_snapshot
                .as_ref()
                .map(|snapshot| snapshot.name.clone()),
        }
    }
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
