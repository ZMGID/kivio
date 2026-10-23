//! Shared model-facing tool contracts and wire-name policy.
use serde::{Deserialize, Serialize};
/// 保留名规避（wire 层别名）：部分上游把特定工具名当作**内部保留工具**拦截消化——
/// 实测 Cursor 系上游（grok-composer 等经 cursor2api 类代理）会把名为 `web_search` 的
/// 工具调用吞掉（模型明确决定调用，流里却什么都不返回，表现为空响应）。全局把这类
/// 内置工具名映射成无歧义的 wire 别名：请求里声明别名、系统提示词渲染别名，收到
/// 别名调用后经 `match_tool_call`（匹配 `openai_tool_name()`）自然映射回内部工具执行。
/// 对正经 provider 无副作用（别名同样是合法工具名）。
const RESERVED_WIRE_ALIASES: &[(&str, &str)] = &[("web_search", "search_web")];

/// 内部工具名 → wire 别名（无命中原样返回）。
pub fn apply_reserved_wire_alias(name: &str) -> String {
    RESERVED_WIRE_ALIASES
        .iter()
        .find(|(from, _)| *from == name)
        .map(|(_, to)| (*to).to_string())
        .unwrap_or_else(|| name.to_string())
}

/// wire 别名 → 内部工具名（无命中原样返回）。供按模型侧函数名反查内部工具的路径使用
/// （如 disabled-tool 反馈）。
pub fn resolve_reserved_wire_alias(name: &str) -> &str {
    RESERVED_WIRE_ALIASES
        .iter()
        .find(|(_, to)| *to == name)
        .map(|(from, _)| *from)
        .unwrap_or(name)
}

/// 旧工具名 → 现工具名。工具被移除/合并/改名后，旧名仍需能路由到现工具，覆盖两条输入
/// 路径：① 模型发出的旧名工具调用（`match_tool_call`），② persona/skill 存储的工具白名单
/// （`tool_matches_recommended_name`）。
///
/// **方向与 `RESERVED_WIRE_ALIASES` 相反**：wire 别名是"内部名 → 模型可见名"（改对外暴露）；
/// 这里是"旧输入名 → 现内部名"（把历史输入规整到现工具），**不参与**工具声明/提示词渲染。
const LEGACY_TOOL_ALIASES: &[(&str, &str)] = &[
    ("read_file", "read"), // 对外名缩短前的文件工具
    ("write_file", "write"),
    ("edit_file", "edit"),
    ("list_dir", "read"), // list_dir / ls 并入 read（read 现在可读目录）
    ("ls", "read"),
    ("search_files", "grep"),
    ("glob_files", "glob"),
    ("find", "glob"), // find 改名 glob
    ("run_command", "bash"),
    ("list_background", "bash_output"), // list_background 并入 bash_output（无 job_id=列表）
    ("todo_update", "todo_write"),      // todo_update 并入 todo_write（整表替换）
    ("skill_activate", "skill"),        // skill_activate 改名 skill（合并 read_file/run_script 后）
];

/// 旧工具名规整为现工具名（无命中原样返回）。
pub fn canonical_tool_name(name: &str) -> &str {
    LEGACY_TOOL_ALIASES
        .iter()
        .find(|(from, _)| *from == name)
        .map(|(_, to)| *to)
        .unwrap_or(name)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub input_schema: serde_json::Value,
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

/// OpenAI / Gemini / Anthropic function names share a 64-char cap
/// (`^[a-zA-Z0-9_-]{1,64}$`). Naive left-truncate turns
/// `mcp__<uuid>__ui_to_artifact` and `mcp__<uuid>__ui_diff_check` into the
/// same `...-ui` and Gemini silently drops the rest.
pub(super) const MAX_WIRE_TOOL_NAME: usize = 64;
const WIRE_NAME_HASH_LEN: usize = 8;
/// Server ids longer than this eat the 64-char budget; replace with a hash.
const LONG_MCP_SERVER_ID: usize = 16;

/// Shared wire-name builder for [`ChatToolDefinition`] and `ModelTool`.
pub fn wire_tool_name(source: &str, name: &str, id: &str, server_id: Option<&str>) -> String {
    match source {
        // Native and Skill tools are model-facing APIs owned by Kivio. Keep their names
        // aligned with the system prompt so models can call exactly what we instruct.
        "native" | "skill" | "mixer" => apply_reserved_wire_alias(&sanitize_openai_tool_name(name)),
        _ => sanitize_openai_tool_name(&compact_mcp_tool_id(id, server_id)),
    }
}

/// `mcp__<very-long-server-id>__<tool>` → `mcp__<8-hex>__<tool>` so the
/// distinguishing tool suffix survives the 64-char cap. Short / already-compact
/// ids are left alone. `sanitize_openai_tool_name` still hash-suffixes if the
/// compact form itself exceeds 64.
fn compact_mcp_tool_id(id: &str, server_id: Option<&str>) -> String {
    let Some(server_id) =
        server_id.filter(|s| !s.is_empty() && s.chars().count() > LONG_MCP_SERVER_ID)
    else {
        return id.to_string();
    };
    let prefix = format!("mcp__{server_id}__");
    let Some(tool) = id.strip_prefix(&prefix).filter(|t| !t.is_empty()) else {
        return id.to_string();
    };
    format!("mcp__{}__{tool}", short_stable_hash(server_id))
}

fn short_stable_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    digest
        .iter()
        .flat_map(|byte| [hex_digit(byte >> 4), hex_digit(byte & 0x0f)])
        .take(WIRE_NAME_HASH_LEN)
        .collect()
}

fn hex_digit(nibble: u8) -> char {
    b"0123456789abcdef"[nibble as usize] as char
}

pub fn sanitize_openai_tool_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        return "tool".to_string();
    }
    if trimmed.chars().count() <= MAX_WIRE_TOOL_NAME {
        return trimmed.to_string();
    }
    // Left-truncate used to collide (issue #33). Keep a stable unique suffix:
    // `<prefix>_<8-hex>` always 64 chars, deterministic, legal on the wire.
    let digest = short_stable_hash(trimmed);
    let prefix_len = MAX_WIRE_TOOL_NAME - 1 - WIRE_NAME_HASH_LEN;
    let prefix: String = trimmed.chars().take(prefix_len).collect();
    format!("{prefix}_{digest}")
}

impl ChatToolDefinition {
    pub fn openai_tool_name(&self) -> String {
        wire_tool_name(
            &self.source,
            &self.name,
            &self.id,
            self.server_id.as_deref(),
        )
    }

    pub fn to_openai_tool(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.openai_tool_name(),
                "description": self.description,
                "parameters": self.input_schema,
            }
        })
    }

    pub fn read_only_hint(&self) -> Option<bool> {
        annotation_bool(self.annotations.as_ref(), "readOnlyHint")
    }

    pub fn destructive_hint(&self) -> Option<bool> {
        annotation_bool(self.annotations.as_ref(), "destructiveHint")
    }

    pub fn open_world_hint(&self) -> Option<bool> {
        annotation_bool(self.annotations.as_ref(), "openWorldHint")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallResult {
    pub content: String,
    pub is_error: bool,
    pub raw: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<ChatToolArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<serde_json::Value>,
    /// Extra user-role messages (OpenAI wire shape) to append to the
    /// conversation right after this tool's result message. Used by `read` to
    /// feed an image to a vision-capable model: a tool result can only carry
    /// text, so the actual image rides here as a follow-up user message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_up_user_messages: Vec<serde_json::Value>,
}

pub use crate::chat::conversation_data::ChatToolArtifact;

fn annotation_bool(annotations: Option<&serde_json::Value>, key: &str) -> Option<bool> {
    let annotations = annotations?;
    let snake_key = to_snake_case(key);
    annotations
        .get(key)
        .or_else(|| annotations.get(&snake_key))
        .and_then(|value| value.as_bool())
}

fn to_snake_case(value: &str) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
