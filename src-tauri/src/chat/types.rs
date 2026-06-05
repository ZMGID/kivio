use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::model::{ModelMessage, ModelUsage};
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

/// 工具调用状态（保存在 assistant message metadata 中）
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub round: u8,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub artifacts: Vec<ChatToolArtifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMixerStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMixerLaneRecord {
    pub id: String,
    pub label: String,
    pub provider_id: String,
    #[serde(default)]
    pub provider_name: Option<String>,
    pub model: String,
    pub status: ChatMixerStatus,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub usage: Option<ModelUsage>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMixerAggregatorRecord {
    pub provider_id: String,
    #[serde(default)]
    pub provider_name: Option<String>,
    pub model: String,
    pub status: ChatMixerStatus,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub usage: Option<ModelUsage>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMixerRunRecord {
    pub id: String,
    pub enabled: bool,
    pub synthesized: bool,
    pub min_successful_lanes: u8,
    pub started_at: i64,
    #[serde(default)]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub lanes: Vec<ChatMixerLaneRecord>,
    #[serde(default)]
    pub aggregator: Option<ChatMixerAggregatorRecord>,
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
    pub tool_calls: Vec<ToolCallRecord>,
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
    pub mixer_runs: Vec<ChatMixerRunRecord>,
    #[serde(default)]
    pub active_skill_id: Option<String>,
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
    pub context_state: ConversationContextState,
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
    pub assistant_id: Option<String>,
    #[serde(default)]
    pub assistant_name: Option<String>,
}

/// 对话索引文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversationIndex {
    pub conversations: Vec<ConversationListItem>,
}

/// Chat 项目（MVP 使用 `name` 作为 Conversation.folder 归属键）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatProject {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 项目索引文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatProjectIndex {
    pub projects: Vec<ChatProject>,
}

/// 可复用 Chat 助手配置。存储字段保持 snake_case，与 Conversation JSON 一致。
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
    pub system_prompt: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub skill_id: Option<String>,
    #[serde(default)]
    pub tool_preset: String,
    #[serde(default)]
    pub conversation_starters: Vec<String>,
    #[serde(default)]
    pub greeting: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
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
    pub system_prompt: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub skill_id: Option<String>,
    #[serde(default)]
    pub tool_preset: String,
    #[serde(default)]
    pub conversation_starters: Vec<String>,
    #[serde(default)]
    pub greeting: String,
}

impl From<&ChatAssistant> for ChatAssistantSnapshot {
    fn from(assistant: &ChatAssistant) -> Self {
        Self {
            id: assistant.id.clone(),
            name: assistant.name.clone(),
            description: assistant.description.clone(),
            system_prompt: assistant.system_prompt.clone(),
            provider_id: assistant.provider_id.clone(),
            model: assistant.model.clone(),
            skill_id: assistant.skill_id.clone(),
            tool_preset: assistant.tool_preset.clone(),
            conversation_starters: assistant.conversation_starters.clone(),
            greeting: assistant.greeting.clone(),
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
