//! Shared tool configuration wire types and defaults; execution and settings persistence remain their existing owners.
use serde::{Deserialize, Serialize};

/// A mixer role binds its provider, model and reasoning selection together.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubAgentModelSelection {
    pub provider_id: String,
    pub model: String,
    pub thinking_level: Option<String>,
}

pub const DEFAULT_SUB_AGENT_CONCURRENCY: usize = 12;
pub const SUB_AGENT_CONCURRENCY_MIN: usize = 1;
pub const SUB_AGENT_CONCURRENCY_MAX: usize = 64;
fn default_true() -> bool {
    true
}

/**
 * Chat MCP stdio server 配置。
 *
 * settings.json 使用 camelCase；env 与 API keys 一样按本地明文设置策略保存。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatMcpServer {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport: String,
    pub url: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub headers: std::collections::HashMap<String, String>,
    pub cwd: Option<String>,
    pub enabled_tools: Vec<String>,
    /// 连接器目录 id（如 "github"/"notion"/"composio" 或 "custom-xxx"）。
    /// 非空表示这条 server 由「连接器」页管理，不在 MCP 页重复展示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
    /// 连接器认证信息。Phase A 只用 `{ kind: "token" }`；OAuth 字段留待 Phase B。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ConnectorAuth>,
}

impl Default for ChatMcpServer {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: false,
            transport: "stdio".to_string(),
            url: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        }
    }
}

/**
 * 连接器认证信息。与 `providers[].apiKeys` 一样按本地明文设置策略保存。
 *
 * Phase A 仅使用 `kind: "token"` + `access_token`；其余字段（refresh/expires/
 * token_endpoint/client_id/scopes）为 Phase B 的 OAuth 流程预留。
 */
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectorAuth {
    pub kind: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub token_endpoint: Option<String>,
    pub client_id: Option<String>,
    /// Pre-registered applications may require a secret for exchange and refresh.
    /// Uses the same local credential persistence as access_token.
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    /// 真实账户标识（邮箱 / 工作区名 / 用户名），授权时尽力提取，拿不到则 None。
    /// 明文存储，向后兼容（旧设置无此字段时反序列化为 None）。
    #[serde(default)]
    pub account: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatNativeToolsConfig {
    pub web_search: bool,
    #[serde(default)]
    pub web_fetch: bool,
    pub skill_runtime: bool,
    #[serde(default)]
    pub read_file: bool,
    #[serde(default)]
    pub write_file: bool,
    #[serde(default)]
    pub edit_file: bool,
    #[serde(default)]
    pub run_command: bool,
    #[serde(default = "default_true")]
    pub knowledge_search: bool,
    /// Agent tools to list / create / edit / run automations.
    #[serde(default = "default_true")]
    pub automation: bool,
    /// Chat tools that let the agent schedule prompts into the current conversation.
    #[serde(default = "default_true")]
    pub scheduled_tasks: bool,
    /// Default root for ordinary (non-project) conversation workbenches.
    /// Missing legacy configs deserialize to an empty string so sanitize can
    /// migrate `workspace_roots[0]` before falling back to the platform default.
    #[serde(default)]
    pub working_directory: String,
    /// Legacy compatibility only. Runtime code must not use this as a path boundary.
    #[serde(default)]
    pub workspace_roots: Vec<String>,
}

impl ChatNativeToolsConfig {
    pub fn any_enabled(&self) -> bool {
        self.web_search
            || self.web_fetch
            || self.skill_runtime
            || self.read_file
            || self.write_file
            || self.edit_file
            || self.run_command
            || self.knowledge_search
            || self.automation
            || self.scheduled_tasks
    }
}

impl Default for ChatNativeToolsConfig {
    fn default() -> Self {
        // Agentic-app baseline: native tools are ON by default. Reading files and
        // running commands are table stakes for the agent (and its sub-agents),
        // not opt-in extras. Safety lives at execution time in the session-consent
        // gate (chat/agent/execute.rs), which the UI lets cautious users tighten
        // back to per-conversation confirmation. web_search still only surfaces
        // when a provider key is configured.
        Self {
            web_search: true,
            web_fetch: true,
            skill_runtime: true,
            read_file: true,
            write_file: true,
            edit_file: true,
            run_command: true,
            knowledge_search: true,
            automation: true,
            scheduled_tasks: true,
            working_directory: default_chat_working_directory(),
            workspace_roots: Vec::new(),
        }
    }
}

pub fn default_chat_working_directory() -> String {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join("Kivio").join("workspace"))
        .unwrap_or_else(|| std::path::PathBuf::from("Kivio").join("workspace"))
        .to_string_lossy()
        .to_string()
}

fn default_skill_auto_match() -> bool {
    true
}

pub(crate) fn default_skill_fallback_mode() -> String {
    "progressive".to_string()
}

pub const CHAT_TOOL_MIN_TIMEOUT_MS: u64 = 1_000;
/// 显式 `bash` `timeout_ms` 与 `bash_output` wait 的上限。省略 timeout 的
/// 前台 bash 等到进程退出，不受默认 60s 工具超时约束。
pub const CHAT_TOOL_MAX_TIMEOUT_MS: u64 = 600_000;
/// 单条工具结果字符上限的合法区间。低于下限会把编译错误/测试输出截到没意义；
/// 高于上限则失去"防上下文撑爆"的作用。`None`（不截断）由 sanitize 归一到默认值。
pub const CHAT_TOOL_MIN_OUTPUT_CHARS: usize = 2_000;
pub const CHAT_TOOL_MAX_OUTPUT_CHARS: usize = 200_000;
/// 默认单条工具结果字符上限 ≈ 6K token（头 1/2 + 尾 1/4 保留约 3/4）。
pub const DEFAULT_MAX_TOOL_OUTPUT_CHARS: usize = 24_000;
/// MCP 持久连接空闲超时下限：太小会让长连接频繁回收失去意义。
pub const MCP_IDLE_TIMEOUT_MIN_MS: u64 = 60_000;
/// MCP 持久连接空闲超时上限：避免死连接长期占用子进程。
pub const MCP_IDLE_TIMEOUT_MAX_MS: u64 = 24 * 60 * 60 * 1_000;

fn default_chat_tool_timeout_ms() -> u64 {
    60_000
}

fn default_sub_agent_concurrency() -> usize {
    DEFAULT_SUB_AGENT_CONCURRENCY
}

fn default_mcp_idle_timeout_ms() -> u64 {
    600_000
}

/// 单条工具结果进入上下文前的字符上限（头 1/2 + 尾 1/4 保留，实际约 3/4）。
/// 默认 [`DEFAULT_MAX_TOOL_OUTPUT_CHARS`]：从源头掐住 read_file / bash / grep 等大输出，
/// 避免它们以全量累积进 runtime_messages 撑爆上下文。`None` = 不截断（旧行为，sanitize 会归一到默认）。
fn default_max_tool_output_chars() -> Option<usize> {
    Some(DEFAULT_MAX_TOOL_OUTPUT_CHARS)
}

pub(crate) fn default_chat_approval_policy() -> String {
    // Commands, file writes and remote tools require explicit user approval by default.
    "always_confirm".to_string()
}

/// Hook 超时的合法区间与默认值。
pub const HOOK_MIN_TIMEOUT_MS: u64 = 1_000;
pub const HOOK_MAX_TIMEOUT_MS: u64 = 600_000;
pub const HOOK_DEFAULT_TIMEOUT_MS: u64 = 60_000;

fn default_hook_timeout_ms() -> u64 {
    HOOK_DEFAULT_TIMEOUT_MS
}

pub(crate) fn default_hook_method() -> String {
    "POST".to_string()
}

/// 对话生命周期 Hook：在 agent loop 的某个事件点执行 Shell 脚本或发一个 HTTP 请求。
/// 一律 fire-and-forget —— 不阻断、不改写工具调用（见 07-28-hooks PRD 的「非目标」）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HookDef {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 8 个生命周期事件之一（见 `chat::hooks::HookEvent`）。未知值在 sanitize 时丢弃。
    pub event: String,
    pub enabled: bool,
    /// "command" | "http"
    #[serde(rename = "type")]
    pub kind: String,
    /// kind == "command" 时的脚本正文。
    pub script: String,
    /// kind == "http" 时的目标。
    pub url: String,
    #[serde(default = "default_hook_method")]
    pub method: String,
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_hook_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for HookDef {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            event: String::new(),
            enabled: false,
            kind: "command".to_string(),
            script: String::new(),
            url: String::new(),
            method: default_hook_method(),
            headers: std::collections::BTreeMap::new(),
            timeout_ms: default_hook_timeout_ms(),
        }
    }
}

/// 归一 Hook 列表：丢弃事件名/类型非法或目标为空的条目，补空 id，钳制超时。
/// 无效条目直接丢弃而非修正——一个 event 打错字的 Hook 没有合理的「就近」事件可猜。
/// 把 JSON `null` 当成空 `Vec` 收下，而不是报类型错误。
/// 见 `ChatToolsConfig::hooks` 上的注释：一个字段被前端漏传就会在磁盘上留下 null，
/// 而 serde 的 `default` 兜不住 null，会让整个父结构解析失败。
fn null_tolerant_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/**
 * Chat 工具与 Skill 配置。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatToolsConfig {
    pub enabled: bool,
    pub servers: Vec<ChatMcpServer>,
    pub skill_scan_paths: Vec<String>,
    #[serde(default = "default_skill_auto_match")]
    pub skill_auto_match: bool,
    #[serde(default = "default_skill_fallback_mode")]
    pub skill_fallback_mode: String,
    /// Skill ids the user turned off in Settings. Omitted ids are enabled.
    #[serde(default)]
    pub disabled_skill_ids: Vec<String>,
    #[serde(default = "default_chat_tool_timeout_ms")]
    pub tool_timeout_ms: u64,
    /// MCP 持久连接空闲超时（ms）：会话 last_used 超过此值后被 reaper 回收，下次调用透明重连。
    #[serde(default = "default_mcp_idle_timeout_ms")]
    pub mcp_idle_timeout_ms: u64,
    #[serde(default = "default_max_tool_output_chars")]
    pub max_tool_output_chars: Option<usize>,
    #[serde(default = "default_chat_approval_policy")]
    pub approval_policy: String,
    /// 同一时刻最多并行运行的子 agent 数。受 [`SUB_AGENT_CONCURRENCY_MIN`]..[`MAX`] 钳制。
    #[serde(default = "default_sub_agent_concurrency")]
    pub sub_agent_concurrency: usize,
    /// Legacy global override, consumed into sub_agent_models.task during normalization.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sub_agent_provider_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sub_agent_model: String,
    /// SMOL / SLOW / TASK model assignments. Empty roles inherit TASK, then the parent.
    #[serde(default)]
    pub sub_agent_models: std::collections::BTreeMap<String, SubAgentModelSelection>,
    /// 开发者「请求调试」总开关：开启后每次 provider 调用被记录到内存环形缓冲（脱敏）。
    /// 默认关闭；关闭时 adapter 零开销（不构造记录）。仅内存、不落盘。
    #[serde(default)]
    pub request_debug_enabled: bool,
    /// Local browser extension credential; never include its value in agent prompts.
    pub playwright_extension_token: String,
    /// 对话生命周期 Hooks（07-28-hooks）。空数组 = 无 Hook = agent loop 零开销。
    ///
    /// `deserialize_with` 而非光 `default`：字段刚上线时前端漏传，`invoke` 把缺失字段
    /// 序列化成 **null** 落进了 settings.json，而 `default` 只兜「键不存在」，遇到
    /// null 会以 `invalid type: null, expected a sequence` 让**整个 chatTools 解析失败**
    /// （连 MCP 服务器、原生工具开关一起丢）。null 归一到空数组。
    #[serde(default, deserialize_with = "null_tolerant_vec")]
    pub hooks: Vec<HookDef>,
    pub native_tools: ChatNativeToolsConfig,
}

impl Default for ChatToolsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            servers: Vec::new(),
            skill_scan_paths: Vec::new(),
            skill_auto_match: default_skill_auto_match(),
            skill_fallback_mode: default_skill_fallback_mode(),
            disabled_skill_ids: Vec::new(),
            tool_timeout_ms: default_chat_tool_timeout_ms(),
            mcp_idle_timeout_ms: default_mcp_idle_timeout_ms(),
            max_tool_output_chars: default_max_tool_output_chars(),
            approval_policy: default_chat_approval_policy(),
            sub_agent_concurrency: default_sub_agent_concurrency(),
            sub_agent_provider_id: String::new(),
            sub_agent_model: String::new(),
            sub_agent_models: Default::default(),
            request_debug_enabled: false,
            playwright_extension_token: String::new(),
            hooks: Vec::new(),
            native_tools: ChatNativeToolsConfig::default(),
        }
    }
}

pub fn sanitize_hooks(hooks: &mut Vec<HookDef>) {
    hooks.retain_mut(|hook| {
        hook.event = hook.event.trim().to_string();
        hook.kind = hook.kind.trim().to_lowercase();
        if crate::chat::hooks::HookEvent::parse(&hook.event).is_none() {
            return false;
        }
        match hook.kind.as_str() {
            "command" => {
                if hook.script.trim().is_empty() {
                    return false;
                }
            }
            "http" => {
                if hook.url.trim().is_empty() {
                    return false;
                }
                hook.method = hook.method.trim().to_uppercase();
                if hook.method.is_empty() {
                    hook.method = default_hook_method();
                }
            }
            _ => return false,
        }
        if hook.id.trim().is_empty() {
            hook.id = uuid::Uuid::new_v4().to_string();
        }
        hook.timeout_ms = hook
            .timeout_ms
            .clamp(HOOK_MIN_TIMEOUT_MS, HOOK_MAX_TIMEOUT_MS);
        true
    });
}
