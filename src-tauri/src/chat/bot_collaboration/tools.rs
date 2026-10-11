//! Conversation tools. Dispatch entry matches `scheduled_tasks::tools`.
//! `ENTRIES` is the only metadata table. `enabled` is always false, so these
//! tools are not in the global builtin list.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::native_registry::{NativeToolCall, NativeToolEntry, NativeToolFuture};
use crate::mcp::registry::NativeToolContext;
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};
use tauri::AppHandle;

use super::flow::{actor_from_tool, broadcast, request_members, submit};
use super::runtime::{now_secs, publish, try_service};
use super::types::DeliveryKind;

pub const PEER_SEND: &str = "bot_peer_send";
pub const PEER_REPLY: &str = "bot_peer_reply";
pub const EXCHANGE_QUERY: &str = "bot_exchange_query";
pub const ROOM_MENTION: &str = "bot_room_mention";
pub const ROOM_HANDOFF: &str = "bot_room_handoff";
pub const ROOM_MEMBERS: &str = "bot_room_members";
pub const ROOM_LOG: &str = "bot_room_log";
pub const ROOM_PAUSE: &str = "bot_room_pause";
pub const ROOM_CONTINUE: &str = "bot_room_continue";
pub const ROOM_BROADCAST: &str = "bot_room_broadcast";

pub static ENTRIES: &[NativeToolEntry] = &[
    NativeToolEntry {
        name: PEER_SEND,
        def: peer_send_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: PEER_REPLY,
        def: peer_reply_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: EXCHANGE_QUERY,
        def: exchange_query_tool,
        enabled: |_, _, _| false,
        parallel_safe: true,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: ROOM_MENTION,
        def: room_mention_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: ROOM_HANDOFF,
        def: room_handoff_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: ROOM_MEMBERS,
        def: room_members_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: ROOM_LOG,
        def: room_log_tool,
        enabled: |_, _, _| false,
        parallel_safe: true,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: ROOM_PAUSE,
        def: room_pause_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: ROOM_CONTINUE,
        def: room_continue_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: ROOM_BROADCAST,
        def: room_broadcast_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
];

pub fn is_coordination_mutation(name: &str) -> bool {
    matches!(
        name,
        PEER_SEND
            | PEER_REPLY
            | ROOM_MENTION
            | ROOM_HANDOFF
            | ROOM_MEMBERS
            | ROOM_PAUSE
            | ROOM_CONTINUE
            | ROOM_BROADCAST
    )
}

pub fn append_tools(app: &AppHandle, conversation_id: &str, tools: &mut Vec<ChatToolDefinition>) {
    if !super::collaboration_conversation(app, conversation_id) {
        return;
    }
    tools.extend(ENTRIES.iter().map(|entry| (entry.def)()));
}

pub fn handle_conversation_tool_call<'a>(
    app: &'a AppHandle,
    ctx: &'a NativeToolContext,
    tool_name: &'a str,
    arguments: Value,
) -> NativeToolFuture<'a> {
    Box::pin(async move {
        let result = call(app, ctx, tool_name, arguments);
        if !result.is_error {
            publish(app);
        }
        Ok(result)
    })
}

fn call(
    app: &AppHandle,
    ctx: &NativeToolContext,
    tool_name: &str,
    arguments: Value,
) -> McpToolCallResult {
    match dispatch(app, ctx, tool_name, arguments) {
        Ok(value) => tool_result(value, false),
        Err(message) => tool_result(json!({ "error": message }), true),
    }
}

fn dispatch(
    app: &AppHandle,
    ctx: &NativeToolContext,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, String> {
    if ctx.depth > 0 {
        return Err("子代理不能代表 Bot 投递或加入协作房间".into());
    }
    let from = actor_from_tool(app, &ctx.conversation_id)?;
    let (exchange_id, parent_delivery_id) = model_lineage(app, &ctx.conversation_id)?;
    match tool_name {
        PEER_SEND => {
            let args: PeerSendArgs = parse(arguments)?;
            let delivery = submit(
                app,
                from,
                &args.to_bot_id,
                None,
                args.room_id,
                DeliveryKind::Peer,
                args.body,
                exchange_id,
                parent_delivery_id,
                true,
                args.attachment_ids,
                args.source_conversation_id.as_deref(),
            )?;
            delivery_value(delivery)
        }
        PEER_REPLY => {
            let args: ReplyArgs = parse(arguments)?;
            let parent = match try_service(app)?.active_inbound(&ctx.conversation_id) {
                Some(delivery) => delivery,
                None => try_service(app)?
                    .inbound_for(&ctx.conversation_id, args.delivery_id.as_deref())?,
            };
            let Some(to_bot_id) = parent.from.bot_id().map(str::to_string) else {
                return Err("对方是用户，直接在回复里写答案，不要再投递".into());
            };
            let delivery = submit(
                app,
                from,
                &to_bot_id,
                None,
                parent.room_id,
                DeliveryKind::Reply,
                args.body,
                Some(parent.exchange_id),
                Some(parent.id),
                true,
                Vec::new(),
                None,
            )?;
            delivery_value(delivery)
        }
        EXCHANGE_QUERY => {
            let args: IdArgs = parse(arguments)?;
            Ok(serde_json::to_value(try_service(app)?.exchange(&args.id)?).unwrap_or(Value::Null))
        }
        ROOM_MENTION => {
            let args: RoomBodyArgs = parse(arguments)?;
            let delivery = submit(
                app,
                from,
                &args.to_bot_id,
                None,
                Some(args.room_id),
                DeliveryKind::Mention,
                args.body,
                exchange_id,
                parent_delivery_id,
                true,
                args.attachment_ids,
                args.source_conversation_id.as_deref(),
            )?;
            delivery_value(delivery)
        }
        ROOM_HANDOFF => {
            let args: RoomBodyArgs = parse(arguments)?;
            let delivery = submit(
                app,
                from,
                &args.to_bot_id,
                None,
                Some(args.room_id),
                DeliveryKind::Handoff,
                args.body,
                exchange_id,
                parent_delivery_id,
                false,
                args.attachment_ids,
                args.source_conversation_id.as_deref(),
            )?;
            delivery_value(delivery)
        }
        ROOM_BROADCAST => {
            let args: BroadcastArgs = parse(arguments)?;
            let receipt = broadcast(
                app,
                from,
                &args.room_id,
                args.body,
                args.attachment_ids,
                exchange_id,
                parent_delivery_id,
                args.source_conversation_id.as_deref(),
            )?;
            Ok(serde_json::to_value(receipt).unwrap_or(Value::Null))
        }
        ROOM_MEMBERS => {
            let args: MemberArgs = parse(arguments)?;
            let actions = request_members(app, &args.room_id, &args.add, &args.remove)?;
            Ok(json!({ "pending": actions, "status": "awaiting_user" }))
        }
        ROOM_LOG => {
            let args: IdArgs = parse(arguments)?;
            Ok(serde_json::to_value(try_service(app)?.room(&args.id)?.log).unwrap_or(Value::Null))
        }
        ROOM_PAUSE => {
            let args: IdArgs = parse(arguments)?;
            let room = try_service(app)?.pause_room(&args.id, now_secs())?;
            Ok(serde_json::to_value(room).unwrap_or(Value::Null))
        }
        ROOM_CONTINUE => {
            let args: IdArgs = parse(arguments)?;
            let room = try_service(app)?.continue_room(&args.id, now_secs())?;
            Ok(serde_json::to_value(room).unwrap_or(Value::Null))
        }
        other => Err(format!("unknown bot collaboration tool: {other}")),
    }
}

fn model_lineage(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<(Option<String>, Option<String>), String> {
    Ok(match try_service(app)?.active_inbound(conversation_id) {
        Some(delivery) => (Some(delivery.exchange_id), Some(delivery.id)),
        None => (None, None),
    })
}

fn delivery_value(delivery: super::types::Delivery) -> Result<Value, String> {
    let unavailable = delivery.status == super::types::DeliveryStatus::Unavailable;
    let value = serde_json::to_value(&delivery).unwrap_or(Value::Null);
    if unavailable {
        Err(delivery.error.unwrap_or_else(|| "目标 Bot 不可用".into()))
    } else {
        Ok(value)
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, String> {
    serde_json::from_value(arguments).map_err(|err| format!("参数无效：{err}"))
}

fn tool_result(value: Value, is_error: bool) -> McpToolCallResult {
    McpToolCallResult {
        content: serde_json::to_string_pretty(&value).unwrap_or_default(),
        is_error,
        raw: value,
        ..Default::default()
    }
}

fn tool(name: &str, description: &str, input_schema: Value, read_only: bool) -> ChatToolDefinition {
    ChatToolDefinition {
        id: format!("native__{name}"),
        name: name.into(),
        description: description.into(),
        source: "native".into(),
        server_id: None,
        server_name: Some("Kivio".into()),
        input_schema,
        sensitive: name == ROOM_HANDOFF || name == ROOM_MEMBERS,
        annotations: Some(json!({
            "readOnlyHint": read_only,
            "destructiveHint": name == ROOM_MEMBERS,
            "openWorldHint": false,
        })),
        output_schema: None,
    }
}

pub fn peer_send_tool() -> ChatToolDefinition {
    tool(
        PEER_SEND,
        "把一条消息投递给另一个持久 Bot。对方用自己的会话和身份运行，不会占用子代理。对方正忙时进入队列，空闲后再发送。",
        json!({
            "type": "object",
            "properties": {
                "to_bot_id": { "type": "string" },
                "body": { "type": "string" },
                "room_id": { "type": "string" },
                "attachment_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 8 },
                "source_conversation_id": { "type": "string" }
            },
            "required": ["to_bot_id", "body"]
        }),
        false,
    )
}

pub fn peer_reply_tool() -> ChatToolDefinition {
    tool(
        PEER_REPLY,
        "回复当前这条同伴投递。只回给发送方，并且一次交流最多自动往返一轮。",
        json!({
            "type": "object",
            "properties": {
                "body": { "type": "string" },
                "delivery_id": { "type": "string" }
            },
            "required": ["body"]
        }),
        false,
    )
}

pub fn exchange_query_tool() -> ChatToolDefinition {
    tool(
        EXCHANGE_QUERY,
        "查看一次同伴交流的时间线、状态和回执。",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "exchange id" } },
            "required": ["id"]
        }),
        true,
    )
}

pub fn room_mention_tool() -> ChatToolDefinition {
    tool(
        ROOM_MENTION,
        "在协作房间里点名一个成员 Bot。只投递给该成员。",
        json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string" },
                "to_bot_id": { "type": "string" },
                "body": { "type": "string" },
                "attachment_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 8 },
                "source_conversation_id": { "type": "string" }
            },
            "required": ["room_id", "to_bot_id", "body"]
        }),
        false,
    )
}

pub fn room_handoff_tool() -> ChatToolDefinition {
    tool(
        ROOM_HANDOFF,
        "把房间里的这件事交接给另一个成员。交接要等用户确认后才会发送。",
        json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string" },
                "to_bot_id": { "type": "string" },
                "body": { "type": "string" },
                "attachment_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 8 },
                "source_conversation_id": { "type": "string" }
            },
            "required": ["room_id", "to_bot_id", "body"]
        }),
        false,
    )
}

pub fn room_broadcast_tool() -> ChatToolDefinition {
    tool(
        ROOM_BROADCAST,
        "房间里没有点名对象时，按成员顺序给其他在场成员各发一条，同一轮，不再转发。没有目标也不会拒绝。",
        json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string" },
                "body": { "type": "string" },
                "attachment_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 8 },
                "source_conversation_id": { "type": "string" }
            },
            "required": ["room_id", "body"]
        }),
        false,
    )
}

pub fn room_members_tool() -> ChatToolDefinition {
    tool(
        ROOM_MEMBERS,
        "请求增减房间成员。变更不会立刻生效，要等用户确认。",
        json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string" },
                "add": { "type": "array", "items": { "type": "string" } },
                "remove": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["room_id"]
        }),
        false,
    )
}

pub fn room_log_tool() -> ChatToolDefinition {
    tool(
        ROOM_LOG,
        "读取协作房间的成员和日志。",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "room id" } },
            "required": ["id"]
        }),
        true,
    )
}

pub fn room_pause_tool() -> ChatToolDefinition {
    tool(
        ROOM_PAUSE,
        "暂停这个房间的自动投递。已经开始的原生运行会跑完，但不会再排队新的自动回复。",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        }),
        false,
    )
}

pub fn room_continue_tool() -> ChatToolDefinition {
    tool(
        ROOM_CONTINUE,
        "继续这个房间。已取消的周期不会重放。",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        }),
        false,
    )
}

#[derive(Deserialize)]
struct PeerSendArgs {
    to_bot_id: String,
    body: String,
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    attachment_ids: Vec<String>,
    #[serde(default)]
    source_conversation_id: Option<String>,
}

#[derive(Deserialize)]
struct ReplyArgs {
    body: String,
    #[serde(default)]
    delivery_id: Option<String>,
}

#[derive(Deserialize)]
struct RoomBodyArgs {
    room_id: String,
    to_bot_id: String,
    body: String,
    #[serde(default)]
    attachment_ids: Vec<String>,
    #[serde(default)]
    source_conversation_id: Option<String>,
}

#[derive(Deserialize)]
struct BroadcastArgs {
    room_id: String,
    body: String,
    #[serde(default)]
    attachment_ids: Vec<String>,
    #[serde(default)]
    source_conversation_id: Option<String>,
}

#[derive(Deserialize)]
struct MemberArgs {
    room_id: String,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

#[derive(Deserialize)]
struct IdArgs {
    id: String,
}
