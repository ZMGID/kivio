//! Conversation tools. Dispatch matches `scheduled_tasks::tools`.
//! `ENTRIES` is the only metadata table. `enabled` is always false, so these
//! tools are not in the global builtin list. Parent `find_entry` chains this slice.
//! Primary does not change this list: every Bot gets the same tools.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::mcp::native_registry::{NativeToolCall, NativeToolEntry, NativeToolFuture};
use crate::mcp::registry::NativeToolContext;
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};

use super::context::resolve_stable_assistant_id;
use super::lifecycle::{self, is_global_memory_tool};
use super::types::{BotCreateRequest, BotEditRequest};

pub const CREATE_TOOL: &str = "bot_create";
pub const UPDATE_TOOL: &str = "bot_update";
pub const GET_TOOL: &str = "bot_get";
pub const LIST_TOOL: &str = "bot_list";
pub const SET_PRIMARY_TOOL: &str = "bot_set_primary";
pub const MEMORY_READ_TOOL: &str = "bot_memory_read";
pub const MEMORY_UPDATE_TOOL: &str = "bot_memory_update";
pub const MEMORY_CLEAR_TOOL: &str = "bot_memory_clear";

pub static ENTRIES: &[NativeToolEntry] = &[
    NativeToolEntry {
        name: CREATE_TOOL,
        def: create_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: UPDATE_TOOL,
        def: update_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: GET_TOOL,
        def: get_tool,
        enabled: |_, _, _| false,
        parallel_safe: true,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: LIST_TOOL,
        def: list_tool,
        enabled: |_, _, _| false,
        parallel_safe: true,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: SET_PRIMARY_TOOL,
        def: set_primary_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: MEMORY_READ_TOOL,
        def: memory_read_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: MEMORY_UPDATE_TOOL,
        def: memory_update_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: MEMORY_CLEAR_TOOL,
        def: memory_clear_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
        call: NativeToolCall::Conversation(handle_conversation_tool_call),
    },
];

pub fn append_tools(app: &AppHandle, conversation_id: &str, tools: &mut Vec<ChatToolDefinition>) {
    match resolve_stable_assistant_id(app, conversation_id) {
        Ok(Some(_)) => {
            tools.retain(|tool| !is_global_memory_tool(&tool.name));
            tools.extend(ENTRIES.iter().map(|entry| (entry.def)()));
        }
        Ok(None) => {}
        Err(_) => {
            tools.retain(|tool| !is_global_memory_tool(&tool.name));
        }
    }
}

pub fn handle_conversation_tool_call<'a>(
    app: &'a AppHandle,
    ctx: &'a NativeToolContext,
    tool_name: &'a str,
    arguments: Value,
) -> NativeToolFuture<'a> {
    Box::pin(async move {
        Ok(match dispatch(app, ctx, tool_name, arguments).await {
            Ok(value) => tool_result(value, false),
            Err(message) => tool_result(json!({ "error": message }), true),
        })
    })
}

pub fn create_tool() -> ChatToolDefinition {
    tool(
        CREATE_TOOL,
        "创建一个持久 Bot：写入助手档案、Bot 配置和固定私聊 conv_bot_{assistantId}。失败不会留下半成品联系人。主 Bot 标记不增加权限。新 Bot 的能力来源默认是当前已授予的全局技能和已启用 MCP，不是全部安装项，也不是空白禁用。",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "description": { "type": "string" },
                "systemPrompt": { "type": "string" },
                "icon": { "type": "string" },
                "color": { "type": "string" },
                "primary": { "type": "boolean" },
                "capability": capability_schema(),
                "approval": approval_schema(),
                "model": model_schema()
            },
            "required": ["name"]
        }),
        false,
        false,
    )
}

pub fn update_tool() -> ChatToolDefinition {
    tool(
        UPDATE_TOOL,
        "编辑一个已有 Bot 的档案、能力来源、模型来源或审批覆盖。保留固定私聊和消息历史。不能读取对方的私聊或私有记忆。省略的字段保持不变。编辑器保存时带上 bot_get 读到的 profile.revision 作为 expectedRevision；版本不一致则拒绝覆盖。",
        json!({
            "type": "object",
            "properties": {
                "assistantId": { "type": "string" },
                "expectedRevision": { "type": "number" },
                "name": { "type": "string" },
                "description": { "type": "string" },
                "systemPrompt": { "type": "string" },
                "icon": { "type": "string" },
                "color": { "type": "string" },
                "primary": { "type": "boolean" },
                "capability": capability_schema(),
                "approval": approval_schema(),
                "model": model_schema()
            },
            "required": ["assistantId"]
        }),
        false,
        false,
    )
}

pub fn get_tool() -> ChatToolDefinition {
    tool(
        GET_TOOL,
        "查看一个 Bot 的档案、能力来源和模型来源。不返回私聊消息，也不返回私有记忆正文。",
        json!({
            "type": "object",
            "properties": { "assistantId": { "type": "string" } },
            "required": ["assistantId"]
        }),
        true,
        false,
    )
}

pub fn list_tool() -> ChatToolDefinition {
    tool(
        LIST_TOOL,
        "列出 Bot 联系人、主标记和能力/模型/审批来源。不含私聊和私有记忆。主标记只表示协调职责。",
        json!({ "type": "object", "properties": {} }),
        true,
        false,
    )
}

pub fn set_primary_tool() -> ChatToolDefinition {
    tool(
        SET_PRIMARY_TOOL,
        "指定或取消主 Bot。这只改变协调标记，不增加工具、审批豁免或阅读他人私聊的能力。",
        json!({
            "type": "object",
            "properties": {
                "assistantId": { "type": "string" },
                "primary": { "type": "boolean" }
            },
            "required": ["assistantId", "primary"]
        }),
        false,
        false,
    )
}

pub fn memory_read_tool() -> ChatToolDefinition {
    tool(
        MEMORY_READ_TOOL,
        "读取当前 Bot 的私有记忆。不会读取全局记忆或其他 Bot 的记忆。",
        json!({ "type": "object", "properties": {} }),
        true,
        false,
    )
}

pub fn memory_update_tool() -> ChatToolDefinition {
    tool(
        MEMORY_UPDATE_TOOL,
        "改写当前 Bot 的私有记忆层 l1 或 l2。不能写入全局记忆或其他 Bot。",
        json!({
            "type": "object",
            "properties": {
                "layer": { "type": "string", "enum": ["l1", "l2"] },
                "content": { "type": "string" }
            },
            "required": ["layer", "content"]
        }),
        false,
        false,
    )
}

pub fn memory_clear_tool() -> ChatToolDefinition {
    tool(
        MEMORY_CLEAR_TOOL,
        "把当前 Bot 的一层私有记忆恢复成空模板。不删除全局记忆，也不删除其他 Bot 的记忆。",
        json!({
            "type": "object",
            "properties": { "layer": { "type": "string", "enum": ["l1", "l2"] } },
            "required": ["layer"]
        }),
        false,
        true,
    )
}

async fn dispatch(
    app: &AppHandle,
    ctx: &NativeToolContext,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, String> {
    if ctx.depth > 0 {
        return Err("子代理不能创建、编辑或读取 Bot".into());
    }
    let self_id = resolve_stable_assistant_id(app, &ctx.conversation_id)?
        .ok_or("只有 Bot 会话可以管理 Bot")?;
    let state = app.state::<crate::state::AppState>();
    match tool_name {
        CREATE_TOOL => {
            let request: BotCreateRequest = parse(arguments)?;
            let detail = lifecycle::create_bot(app, state.inner(), request).await?;
            serde_json::to_value(detail).map_err(|err| err.to_string())
        }
        UPDATE_TOOL => {
            let request: BotEditRequest = parse(arguments)?;
            let detail = lifecycle::update_bot(app, state.inner(), request).await?;
            serde_json::to_value(detail).map_err(|err| err.to_string())
        }
        GET_TOOL => {
            let args: IdArgs = parse(arguments)?;
            let settings = state.settings_read().clone();
            let detail = lifecycle::profile_detail(app, &settings, &args.assistant_id)?;
            serde_json::to_value(detail).map_err(|err| err.to_string())
        }
        LIST_TOOL => {
            let items = lifecycle::list_bots(app).await?;
            serde_json::to_value(json!({ "bots": items })).map_err(|err| err.to_string())
        }
        SET_PRIMARY_TOOL => {
            let args: PrimaryArgs = parse(arguments)?;
            let profile = lifecycle::set_primary_bot(app, &args.assistant_id, args.primary).await?;
            serde_json::to_value(profile).map_err(|err| err.to_string())
        }
        MEMORY_READ_TOOL => {
            let view = lifecycle::memory_view(app, &self_id)?;
            serde_json::to_value(view).map_err(|err| err.to_string())
        }
        MEMORY_UPDATE_TOOL => {
            let args: MemoryWriteArgs = parse(arguments)?;
            let view = lifecycle::save_bot_memory(app, &self_id, &args.layer, &args.content)?;
            serde_json::to_value(view).map_err(|err| err.to_string())
        }
        MEMORY_CLEAR_TOOL => {
            let args: LayerArgs = parse(arguments)?;
            let view = lifecycle::clear_bot_memory(app, &self_id, &args.layer)?;
            serde_json::to_value(view).map_err(|err| err.to_string())
        }
        other => Err(format!("unknown bot tool: {other}")),
    }
}

fn capability_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source": { "type": "string", "enum": ["granted_defaults", "explicit"] },
            "mcpServerIds": { "type": "array", "items": { "type": "string" } },
            "skillIds": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["source"]
    })
}

fn model_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source": { "type": "string", "enum": ["inherit_global", "explicit"] },
            "providerId": { "type": "string" },
            "model": { "type": "string" }
        },
        "required": ["source"]
    })
}

fn approval_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source": { "type": "string", "enum": ["inherit_global", "override"] },
            "policy": {
                "type": "string",
                "enum": ["always_confirm", "readonly_auto_sensitive_confirm", "auto"]
            }
        },
        "required": ["source"]
    })
}

fn tool(
    name: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
    destructive: bool,
) -> ChatToolDefinition {
    ChatToolDefinition {
        id: format!("native__{name}"),
        name: name.into(),
        description: description.into(),
        source: "native".into(),
        server_id: None,
        server_name: Some("Kivio".into()),
        input_schema,
        sensitive: destructive,
        annotations: Some(json!({
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "openWorldHint": false,
        })),
        output_schema: None,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdArgs {
    assistant_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimaryArgs {
    assistant_id: String,
    primary: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayerArgs {
    layer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryWriteArgs {
    layer: String,
    content: String,
}
