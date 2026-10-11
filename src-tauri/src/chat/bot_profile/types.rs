//! Bot profile contract. Disk and IPC share these types.
//! Parent exports them; BotSurface owns the TypeScript adapters.
//!
//! `ChatAssistant` is unchanged. Primary, capability source, approval override,
//! and explicit shared-memory notes live here, keyed by the same assistant id.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::chat::types::BOT_CONVERSATION_PREFIX;

pub const PROFILE_VERSION: u32 = 1;
pub const CHANGED_EVENT: &str = "bot-profile-changed";
pub const MAX_SHARED_NOTES: usize = 20;
pub const MAX_SHARED_NOTE_CHARS: usize = 2_000;

pub const GRANTED_DEFAULTS_EXPLANATION: &str = "来源是当前已授予的全局能力：已启用的 MCP，以及设置里启用且连接器条件满足的技能。不是安装目录里的全部能力，也不是把空白名单当成静默禁用。";
pub const EXPLICIT_EXPLANATION: &str = "来源是这份 Bot 明确保存的名单，并且仍会去掉当前全局未授予的条目。空名单表示明确不使用对应的技能或 MCP。";
pub const LEGACY_EXPLANATION: &str = "来源是这份助手原有的白名单。空名单按历史规则表示没有技能或 MCP；没有改写成新的默认授予，也没有把普通助手批量转成 Bot。";
pub const INHERIT_MODEL_EXPLANATION: &str =
    "模型来源是全局默认。这不是这份 Bot 单独保存的模型，也不会写回全局默认或其他 Bot。";
pub const EXPLICIT_MODEL_EXPLANATION: &str =
    "模型来源是这份 Bot 单独保存的供应商和模型，不会修改全局默认。";
pub const FROZEN_MODEL_EXPLANATION: &str =
    "还没有 Bot 配置文件。模型保持这条固定私聊里已经写下的值，不会改成全局默认，也不会批量迁移。";
pub const INHERIT_APPROVAL_EXPLANATION: &str =
    "审批来源是全局 Chat 审批策略。这份 Bot 没有单独覆盖，也不会修改全局策略。";
pub const OVERRIDE_APPROVAL_EXPLANATION: &str =
    "审批来源是这份 Bot 的覆盖值，只作用于这份 Bot 的运行，不修改全局审批策略。";

pub const APPROVAL_POLICIES: &[&str] =
    &["always_confirm", "readonly_auto_sensitive_confirm", "auto"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CapabilitySource {
    GrantedDefaults,
    Explicit,
    LegacyWhitelist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ModelSource {
    InheritGlobal,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ApprovalSource {
    InheritGlobal,
    Override,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ResolvedModelSource {
    InheritGlobal,
    Explicit,
    /// No profile file. The fixed chat already has a model; do not rewrite it.
    ConversationFrozen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotCapabilityPolicy {
    pub source: CapabilitySource,
    /// Raw saved ids. Absent for granted defaults so they are not a private snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_ids: Option<Vec<String>>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotModelPolicy {
    pub source: ModelSource,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotApprovalPolicy {
    pub source: ApprovalSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SharedMemoryNote {
    pub id: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotProfile {
    pub version: u32,
    pub assistant_id: String,
    /// Coordination duty only. Does not add or remove tools.
    pub primary: bool,
    pub capability: BotCapabilityPolicy,
    pub model: BotModelPolicy,
    pub approval: BotApprovalPolicy,
    #[serde(default)]
    pub shared_memory: Vec<SharedMemoryNote>,
    /// Per-Bot save counter. Old files omit it and load as 0.
    /// An editor save sends this value back as `expectedRevision`.
    #[serde(default)]
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ResolvedCapability {
    pub source: CapabilitySource,
    pub explanation: String,
    pub mcp_server_ids: Vec<String>,
    pub skill_ids: Vec<String>,
    pub dropped_mcp_server_ids: Vec<String>,
    pub dropped_skill_ids: Vec<String>,
    /// False for a legacy whitelist: reply keeps the historical snapshot lists.
    pub rewrite_snapshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotRuntimeScope {
    pub assistant_id: String,
    pub conversation_id: String,
    /// Coordination marker only. Same tools and authority. No autonomous loop.
    /// `memory_prompt` does not include this text. `reply.rs` appends it when true.
    pub primary: bool,
    pub persisted_profile: bool,
    pub provider_id: String,
    pub model: String,
    pub model_source: ResolvedModelSource,
    pub model_explanation: String,
    /// Parent rewrites the in-memory conversation model only when this is true.
    pub rewrite_model: bool,
    pub capability: ResolvedCapability,
    /// None means keep the cloned global approval policy for this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_override: Option<String>,
    pub approval_source: ApprovalSource,
    pub approval_explanation: String,
    pub memory_prompt: Option<String>,
    pub memory_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotProfileDetail {
    pub assistant_id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub color: String,
    pub system_prompt: String,
    pub provider_id: String,
    pub model: String,
    pub conversation_id: String,
    pub persisted_profile: bool,
    pub profile: BotProfile,
    pub resolved: ResolvedCapability,
    pub resolved_provider_id: String,
    pub resolved_model: String,
    pub model_source: ResolvedModelSource,
    pub model_explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotListItem {
    pub assistant_id: String,
    pub name: String,
    pub conversation_id: String,
    pub primary: bool,
    pub persisted_profile: bool,
    pub capability_source: Option<CapabilitySource>,
    pub capability_explanation: String,
    pub model_source: Option<ResolvedModelSource>,
    pub approval_source: Option<ApprovalSource>,
    /// A corrupt profile is reported here. The file is not replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotMemoryView {
    pub assistant_id: String,
    pub private_directory: String,
    /// Global L1 is not copied here and is not injected for this Bot.
    pub global_injection: &'static str,
    pub l1: Option<String>,
    pub l2: Option<String>,
    pub shared_memory: Vec<SharedMemoryNote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotCapabilityWrite {
    pub source: CapabilitySource,
    /// Omitted and an empty list are the same explicit empty set. Not granted defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mcp_server_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub skill_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotModelWrite {
    pub source: ModelSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotApprovalWrite {
    pub source: ApprovalSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub policy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotCreateRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<String>,
    /// Omitted means not primary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub primary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capability: Option<BotCapabilityWrite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approval: Option<BotApprovalWrite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<BotModelWrite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BotEditRequest {
    pub assistant_id: String,
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub icon: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub color: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub primary: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub capability: Option<BotCapabilityWrite>,
    #[serde(default)]
    #[ts(optional)]
    pub approval: Option<BotApprovalWrite>,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<BotModelWrite>,
    /// When set, the save is rejected if this Bot's profile revision changed.
    /// Omitted means: merge the latest profile inside the lifecycle lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PreparedBot {
    pub assistant: crate::chat::ChatAssistant,
    pub profile: BotProfile,
}

pub fn bot_conversation_id(assistant_id: &str) -> String {
    format!("{BOT_CONVERSATION_PREFIX}{assistant_id}")
}

pub fn assistant_id_from_conversation(conversation_id: &str) -> Option<&str> {
    let id = conversation_id.strip_prefix(BOT_CONVERSATION_PREFIX)?;
    if id.starts_with("asst_")
        && id.len() > "asst_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Some(id)
    } else {
        None
    }
}

pub fn validate_assistant_id(assistant_id: &str) -> Result<&str, String> {
    let assistant_id = assistant_id.trim();
    if assistant_id_from_conversation(&format!("{BOT_CONVERSATION_PREFIX}{assistant_id}")).is_some()
    {
        Ok(assistant_id)
    } else {
        Err(format!("Bot 助手 id 无效：{assistant_id}"))
    }
}

pub fn is_bot_conversation(conversation_id: &str) -> bool {
    assistant_id_from_conversation(conversation_id).is_some()
}

/// Dedicated group-member thread. Not the private chat `conv_bot_{assistantId}`.
/// BotCollaboration should create these with [`member_conversation_id`] and set
/// `Conversation.assistant_id` to the same assistant id.
pub const MEMBER_CONVERSATION_MARK: &str = "conv_bot_member_";

pub fn member_conversation_id(assistant_id: &str, room_id: &str) -> Result<String, String> {
    let assistant_id = validate_assistant_id(assistant_id)?;
    let room_id = validate_room_id(room_id)?;
    Ok(format!(
        "{MEMBER_CONVERSATION_MARK}{assistant_id}@{room_id}"
    ))
}

pub fn assistant_id_from_member_conversation(conversation_id: &str) -> Option<&str> {
    let rest = conversation_id.strip_prefix(MEMBER_CONVERSATION_MARK)?;
    let (assistant_id, room_id) = rest.split_once('@')?;
    if validate_assistant_id(assistant_id).is_err() || validate_room_id(room_id).is_err() {
        return None;
    }
    Some(assistant_id)
}

fn validate_room_id(room_id: &str) -> Result<&str, String> {
    let room_id = room_id.trim();
    if room_id.is_empty()
        || room_id.contains('@')
        || room_id.contains('/')
        || room_id.contains('\\')
        || room_id
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    {
        return Err(format!("群成员会话的房间 id 无效：{room_id}"));
    }
    Ok(room_id)
}
