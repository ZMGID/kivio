//! Peer and room contracts. Disk and IPC share these types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::chat::types::BOT_CONVERSATION_PREFIX;

pub const CHANGED_EVENT: &str = "bot-collaboration-changed";
/// One request plus one native reply. A further automatic hop is refused.
pub const MAX_COORDINATION_DEPTH: u32 = 1;
pub const MAX_DELIVERIES_PER_EXCHANGE: usize = 8;
pub const MAX_ROOM_BOTS: usize = 8;
/// Recipient runs at once. One bot waiting on approval keeps its own slot.
pub const MAX_PARALLEL_DISPATCH: usize = MAX_ROOM_BOTS;
/// Unaddressed room messages run every other active member, capped by the room size.
pub const BROADCAST_BOUND: usize = MAX_ROOM_BOTS;
pub const MAX_ATTACHMENT_IDS: usize = 8;
pub const MAX_ROOM_TITLE_CHARS: usize = 80;
pub const INTERRUPT_REASON: &str = "应用退出时这次投递已经开始，未重放副作用";
pub const GROUP_STOP_REASON: &str = "群组周期已停止，未发送";
pub const NO_FABRICATED_REPLY: &str = "未写入伪造回复";

pub fn bot_conversation_id(bot_id: &str) -> String {
    format!("{BOT_CONVERSATION_PREFIX}{bot_id}")
}

pub fn bot_id_from_conversation(conversation_id: &str) -> Option<&str> {
    // Member threads are not private chats. Parsing them is the core helper.
    if conversation_id.starts_with(crate::chat::bot_profile::MEMBER_CONVERSATION_MARK) {
        return None;
    }
    let id = conversation_id.strip_prefix(BOT_CONVERSATION_PREFIX)?;
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains('@') {
        None
    } else {
        Some(id)
    }
}

/// Room id of a core member thread. `None` unless
/// [`assistant_id_from_member_conversation`](crate::chat::bot_profile::assistant_id_from_member_conversation)
/// accepts the id.
pub fn member_room_id(conversation_id: &str) -> Option<&str> {
    let assistant_id =
        crate::chat::bot_profile::assistant_id_from_member_conversation(conversation_id)?;
    let rest = conversation_id.strip_prefix(crate::chat::bot_profile::MEMBER_CONVERSATION_MARK)?;
    let (encoded, room_id) = rest.split_once('@')?;
    (encoded == assistant_id).then_some(room_id)
}

pub fn normalize_attachment_ids(ids: &[String]) -> Result<Vec<String>, String> {
    if ids.len() > MAX_ATTACHMENT_IDS {
        return Err(format!("附件最多 {MAX_ATTACHMENT_IDS} 个"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id.trim();
        if id.is_empty()
            || id.len() > 200
            || id.contains('/')
            || id.contains('\\')
            || id.contains('@')
            || id.chars().any(|c| c.is_whitespace())
        {
            return Err(format!("附件 id 无效：{id}"));
        }
        if !seen.insert(id.to_string()) {
            return Err(format!("附件「{id}」重复"));
        }
        out.push(id.to_string());
    }
    Ok(out)
}

pub fn normalize_room_title(title: &str) -> Result<String, String> {
    let trimmed = title.trim();
    let len = trimmed.chars().count();
    if len == 0 || len > MAX_ROOM_TITLE_CHARS {
        return Err(format!("房间标题需要 1 到 {MAX_ROOM_TITLE_CHARS} 个字"));
    }
    Ok(trimmed.to_string())
}

pub fn is_bot_conversation(conversation_id: &str) -> bool {
    bot_id_from_conversation(conversation_id).is_some()
}

/// Assistant id that may own a private memory directory.
/// Ordinary chats and non-assistant bot ids return none.
pub fn private_assistant_id(bot_id: &str) -> Option<&str> {
    let id = bot_id.trim();
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

/// Who a conversation is when it is a Bot thread or one room member's thread.
/// A shared room thread with two different Bots has no single identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MemberConversationIdentity {
    pub bot_id: String,
    pub name: String,
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    /// Set only when this Bot has a private memory directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_id: Option<String>,
}

pub fn identity_for_members(
    conversation_id: &str,
    rooms: &[Room],
) -> Option<MemberConversationIdentity> {
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return None;
    }
    let mut matches = Vec::new();
    for room in rooms {
        for member in &room.members {
            if member.state != MemberState::Active || member.conversation_id != conversation_id {
                continue;
            }
            matches.push(MemberConversationIdentity {
                assistant_id: crate::chat::bot_profile::assistant_id_from_member_conversation(
                    &member.conversation_id,
                )
                .map(str::to_string),
                bot_id: member.bot_id.clone(),
                name: member.name.clone(),
                conversation_id: member.conversation_id.clone(),
                room_id: Some(room.id.clone()),
            });
        }
    }
    let first = matches.first()?.clone();
    if matches.iter().any(|item| item.bot_id != first.bot_id) {
        return None;
    }
    let room_id = (matches.len() == 1)
        .then(|| first.room_id.clone())
        .flatten();
    Some(MemberConversationIdentity { room_id, ..first })
}

pub fn is_room_conversation(conversation_id: &str, rooms: &[Room]) -> bool {
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return false;
    }
    rooms.iter().any(|room| {
        room.members.iter().any(|member| {
            member.state == MemberState::Active && member.conversation_id == conversation_id
        })
    })
}

pub fn validate_bot_id(bot_id: &str) -> Result<&str, String> {
    let id = bot_id.trim();
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.starts_with(BOT_CONVERSATION_PREFIX)
    {
        return Err(format!("Bot id 无效：{bot_id}"));
    }
    Ok(id)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    User,
    Bot {
        id: String,
        name: String,
        // Appearance at message creation; old records retain the default avatar.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
}

impl Actor {
    pub fn bot(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self::Bot {
            id: id.into(),
            name: name.into(),
            icon: None,
            color: None,
        }
    }

    pub fn from_assistant(assistant: &crate::chat::types::ChatAssistant) -> Self {
        Self::Bot {
            id: assistant.id.clone(),
            name: assistant.name.clone(),
            icon: (!assistant.icon.is_empty()).then(|| assistant.icon.clone()),
            color: (!assistant.color.is_empty()).then(|| assistant.color.clone()),
        }
    }

    pub fn bot_id(&self) -> Option<&str> {
        match self {
            Self::Bot { id, .. } => Some(id),
            Self::User => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DeliveryKind {
    Peer,
    Mention,
    Reply,
    Handoff,
    Broadcast,
}

/// Provenance for one peer delivery. The chat message stores the clean body
/// separately; provider replay uses this record to rebuild the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PeerMessage {
    pub delivery_id: String,
    pub exchange_id: String,
    pub from: Actor,
    pub to_bot_id: String,
    pub to_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    pub kind: DeliveryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Queued,
    AwaitingUser,
    Claimed,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    Unavailable,
}

impl DeliveryStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::Interrupted
                | Self::Unavailable
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReceiptState {
    Accepted,
    Completed,
    Rejected,
    Interrupted,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct Receipt {
    pub at: i64,
    pub state: ReceiptState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct Delivery {
    pub id: String,
    pub exchange_id: String,
    pub entry_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    pub room_cycle: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_delivery_id: Option<String>,
    pub from: Actor,
    pub to_bot_id: String,
    pub to_name: String,
    pub to_conversation_id: String,
    pub kind: DeliveryKind,
    pub body: String,
    /// Existing chat attachment ids. Empty for older queued items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment_ids: Vec<String>,
    /// Conversations allowed to supply those attachments. Ids only, not transcripts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment_sources: Vec<String>,
    pub depth: u32,
    pub status: DeliveryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_token: Option<String>,
    #[serde(default)]
    pub admitted: bool,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub receipts: Vec<Receipt>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
}

impl From<&Delivery> for PeerMessage {
    fn from(delivery: &Delivery) -> Self {
        Self {
            delivery_id: delivery.id.clone(),
            exchange_id: delivery.exchange_id.clone(),
            from: delivery.from.clone(),
            to_bot_id: delivery.to_bot_id.clone(),
            to_name: delivery.to_name.clone(),
            room_id: delivery.room_id.clone(),
            kind: delivery.kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ExchangeEntry {
    pub id: String,
    pub at: i64,
    pub actor: Actor,
    pub kind: DeliveryKind,
    pub body: String,
    pub delivery_id: String,
    pub status: DeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct Exchange {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    pub entries: Vec<ExchangeEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RoomStatus {
    Active,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum MemberState {
    Active,
    PendingRemoval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RoomMember {
    pub bot_id: String,
    pub name: String,
    pub conversation_id: String,
    pub state: MemberState,
    pub joined_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum LogKind {
    Message,
    Status,
    Handoff,
    Membership,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RoomLogEntry {
    pub id: String,
    pub at: i64,
    pub actor: Actor,
    pub kind: LogKind,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct Room {
    pub id: String,
    pub title: String,
    pub status: RoomStatus,
    pub cycle: u64,
    pub members: Vec<RoomMember>,
    pub log: Vec<RoomLogEntry>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum PendingKind {
    Handoff,
    AddMember,
    RemoveMember,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum PendingStatus {
    AwaitingUser,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PendingAction {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    pub kind: PendingKind,
    pub bot_id: String,
    pub bot_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    pub status: PendingStatus,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CollaborationSnapshot {
    pub deliveries: Vec<Delivery>,
    pub rooms: Vec<Room>,
    pub exchanges: Vec<Exchange>,
    pub pending: Vec<PendingAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiskState {
    pub deliveries: Vec<Delivery>,
    pub rooms: Vec<Room>,
    pub exchanges: Vec<Exchange>,
    pub pending: Vec<PendingAction>,
}

/// Result of one native recipient run. `native_output` is copied from the
/// assistant message; it is never a canned echo of the inbound body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFinish {
    Succeeded {
        native_output: String,
        model_enqueued: bool,
    },
    Failed {
        message: String,
    },
    Cancelled {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct FinishReport {
    pub delivery: Delivery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<Delivery>,
    pub already_terminal: bool,
}

#[derive(Debug, Clone)]
pub struct Enqueue {
    pub from: Actor,
    pub to_bot_id: String,
    pub to_name: String,
    pub room_id: Option<String>,
    pub kind: DeliveryKind,
    pub body: String,
    pub exchange_id: Option<String>,
    pub parent_delivery_id: Option<String>,
    pub sensitive: bool,
    /// User-confirmed handoff/send. Model tools pass false and wait.
    pub approved: bool,
    pub attachment_ids: Vec<String>,
    pub attachment_sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MemberSession {
    pub room_id: String,
    pub bot_id: String,
    pub name: String,
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_id: Option<String>,
    /// True when this call created the member thread.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BroadcastReceipt {
    pub room_id: String,
    pub delivery_ids: Vec<String>,
    pub skipped_bot_ids: Vec<String>,
    pub bound: u32,
}

#[derive(Debug, Clone)]
pub struct MemberInput {
    pub bot_id: String,
    pub name: String,
}
