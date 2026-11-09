pub use crate::chat::model::ModelArtifact as ChatToolArtifact;
use crate::chat::model::ModelTool;
use crate::chat::tools::{ArgumentPolicy, ToolTimeout};

use serde::{Deserialize, Serialize};

use crate::settings::{ChatMcpServer, ChatNativeToolsConfig};

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
    ("schedule_create", "schedule"),
    ("schedule_list", "schedule"),
    ("schedule_update", "schedule"),
    ("schedule_delete", "schedule"),
    ("automation_list", "automation"),
    ("automation_get", "automation"),
    ("automation_upsert", "automation"),
    ("automation_set_enabled", "automation"),
    ("automation_run", "automation"),
    ("automation_runs", "automation"),
    ("automation_delete", "automation"),
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

/// Preserve the action implied by a historical tool call before schema/policy checks.
pub(crate) fn inject_legacy_tool_action(name: &str, arguments: &mut serde_json::Value) {
    let canonical = canonical_tool_name(name);
    if matches!(canonical, "automation" | "schedule") && canonical != name {
        if let (Some(action), Some(object)) = (
            name.strip_prefix(canonical)
                .and_then(|s| s.strip_prefix('_')),
            arguments.as_object_mut(),
        ) {
            object.insert("action".into(), action.into());
        }
    }
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

impl From<&ChatToolDefinition> for ModelTool {
    fn from(tool: &ChatToolDefinition) -> Self {
        Self {
            name: tool.openai_tool_name(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
        }
    }
}

impl ChatToolDefinition {
    pub(crate) fn for_run(
        &self,
        settings: &crate::settings::Settings,
    ) -> crate::chat::tools::ToolDefinition {
        let model = ModelTool::from(self);
        crate::chat::tools::ToolDefinition {
            key: self.id.clone(),
            aliases: LEGACY_TOOL_ALIASES.iter().filter(|(_, to)| *to == self.name || *to == model.name)
                .map(|(from, _)| (*from).into()).collect(),
            model,
            label: crate::chat::tools::ToolLabel {
                name: self.name.clone(), source: self.source.clone(), server_id: self.server_id.clone(), sensitive: self.sensitive,
            },
            parallel_safe: self.parallel_safe() && !crate::chat::tool_runtime::tool_requires_approval(settings, self),
            checkpoint_before_dispatch: self.source == "native" && self.name == crate::chat::sub_agent::TASK_TOOL_NAME,
            speculative: self.source == "native" && self.name == crate::chat::sub_agent::TASK_TOOL_NAME,
            argument_limit: (self.openai_tool_name() == "present_artifacts").then_some(PRESENT_ARTIFACTS_ARGUMENTS_MAX_CHARS),
            argument_limit_error: (self.openai_tool_name() == "present_artifacts").then(|| format!("{} arguments are too large. Pass only artifact_ids or paths; never file contents, base64, or data URLs.", self.name)),
        }
    }

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

    pub fn timeout_policy(&self) -> ToolTimeout {
        match self.source.as_str() {
            "native" => super::native_registry::find_entry(&self.name)
                .map_or(ToolTimeout::Default, |entry| entry.timeout),
            "mixer" => match self.name.as_str() {
                "mixer_video_analysis" => ToolTimeout::AtLeast(300_000),
                "mixer_generate_image" => {
                    ToolTimeout::AtLeast(crate::chat::image_generation::IMAGE_GENERATION_TIMEOUT_MS)
                }
                _ => ToolTimeout::Default,
            },
            _ => ToolTimeout::Default,
        }
    }

    pub fn argument_policy(&self) -> ArgumentPolicy {
        if self.source == "native" {
            return super::native_registry::find_entry(&self.name)
                .map_or(ArgumentPolicy::Schema, |entry| entry.arguments);
        }
        ArgumentPolicy::Schema
    }

    pub fn parallel_safe(&self) -> bool {
        match self.source.as_str() {
            "native" => super::native_registry::find_entry(&self.name)
                .is_some_and(|entry| entry.parallel_safe),
            "mcp" => self.is_read_only_tool(),
            _ => false,
        }
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

    pub fn read_only_actions(&self) -> &'static [&'static str] {
        if self.source != "native" {
            return &[];
        }
        super::native_registry::find_entry(&self.name).map_or(&[], |entry| entry.read_only_actions)
    }

    pub fn has_read_only_calls(&self) -> bool {
        self.is_read_only_tool() || !self.read_only_actions().is_empty()
    }

    pub fn is_read_only_call(&self, arguments: &serde_json::Value) -> bool {
        self.is_read_only_tool()
            || arguments
                .get("action")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|action| self.read_only_actions().contains(&action))
    }

    pub(crate) fn restrict_to_read_only_calls(&mut self, mode: &str) {
        if !self.read_only_actions().is_empty() {
            self.annotations
                .get_or_insert_with(|| serde_json::json!({}))["kivioReadOnlyMode"] = mode.into();
        }
    }

    pub fn is_read_only_tool(&self) -> bool {
        if self.source == "mcp" {
            return self.read_only_hint() == Some(true)
                && self.destructive_hint() != Some(true)
                && self.open_world_hint() != Some(true);
        }
        // Native read-only metadata lives in the static registry
        // (mcp/native_registry.rs). Note: this set includes memory_read and
        // memory_search, which are read-only but deliberately not parallel-safe.
        self.source == "native"
            && super::native_registry::find_entry(&self.name).is_some_and(|entry| entry.read_only)
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

pub use kivio_agent_sdk::mcp::McpTool;

impl From<kivio_agent_sdk::mcp::McpToolCallResult> for McpToolCallResult {
    fn from(result: kivio_agent_sdk::mcp::McpToolCallResult) -> Self {
        Self {
            content: result.content,
            is_error: result.is_error,
            raw: result.raw,
            artifacts: result.artifacts,
            structured_content: result.structured_content,
            follow_up_user_messages: Vec::new(),
        }
    }
}

pub fn tool_definition_from_mcp(server: &ChatMcpServer, tool: McpTool) -> ChatToolDefinition {
    let definition = kivio_agent_sdk::mcp::mcp_tool_definition(&server.id, &tool);
    ChatToolDefinition {
        id: definition.key,
        name: tool.name,
        description: definition.model.description,
        source: definition.label.source,
        server_id: definition.label.server_id,
        server_name: Some(server.name.clone()),
        input_schema: definition.model.input_schema,
        sensitive: definition.label.sensitive,
        annotations: tool.annotations,
        output_schema: tool.output_schema,
    }
}

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

pub fn native_web_search_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__web_search".to_string(),
        name: "web_search".to_string(),
        description: "Search the web for current facts and return source snippets.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                }
            },
            "required": ["query"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_skill_activate_tool() -> ChatToolDefinition {
    let definition = crate::skills::SkillTool::definition();
    ChatToolDefinition {
        id: "skill__activate".to_string(),
        name: definition.model.name,
        description: definition.model.description,
        source: "skill".to_string(),
        server_id: None,
        server_name: Some("Skill".to_string()),
        input_schema: definition.model.input_schema,
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_skill_tools() -> Vec<ChatToolDefinition> {
    vec![native_skill_activate_tool()]
}

pub fn native_read_file_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__read_file".to_string(),
        name: "read".to_string(),
        description: "Read a file, directory entries (ignores offset/limit), images (vision or description/OCR), or authorized MCP resource. Text prefixes `N<TAB>` are display-only: never copy into edit old_string. Capped at 2000 lines/50KB; follow the returned offset notice to read needed content beyond the window. PDF/Word/Excel need host parsers; binary documents are not parsed. MCP: mcp:// lists authorized servers, mcp://<server-id> lists resources; returned paths read/subscribe. Updates are notices; re-read for current contents.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path/directory/MCP URI. Relative to cwd; absolute/~/ may point outside, subject to permissions/approvals. Disk paths need no registration. Omit/empty when using other selectors." },
                "artifact_ids": { "type": "array", "items": { "type": "string", "minLength": 1 }, "minItems": 0, "maxItems": 12, "description": "Exact returned art_ IDs; nonempty overrides path/paths. Omit/[] for disk paths; never invent IDs." },
                "paths": {
                    "type": "array",
                    "description": "Image paths only, shown separately by default; omit/[] for other selectors. Never invent paths.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 12
                },
                "overview": {
                    "type": "boolean",
                    "description": "Numbered contact sheet for skimming 6–12 similar images only. Analysis/QA/text/logos/per-image detail always use separate images; re-read one path for fine text. Default false."
                },
                "offset": { "type": "integer", "description": "1-based start line; past EOF errors." },
                "limit": { "type": "integer", "description": "Max lines; output cap still applies." }
            }
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

/// Directory listing tool def. No longer part of the chat native tool set (chat's
/// `read` now lists directories directly), but still used by the Kivio Code surface,
/// which keeps a dedicated `ls` in its own tool list.
pub fn native_list_dir_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__list_dir".to_string(),
        name: "ls".to_string(),
        description: "List files and directories in a directory. Omit path (or pass \".\") to list the current working directory; relative paths resolve from it. Do not guess or invent an absolute path, and never translate/\"correct\" directory names — pass an absolute or ~/ path only when the user gave one or an earlier tool returned it.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory to list. Defaults to the current working directory; relative paths resolve from it. Do not fabricate an absolute path." },
                "include_hidden": { "type": "boolean", "description": "Include dotfiles and hidden entries" },
                "max_entries": { "type": "integer", "description": "Maximum entries to return, default 200, max 500" }
            }
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_search_files_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__search_files".to_string(),
        name: "grep".to_string(),
        description: "Search text in a file or under a directory; if known, pass that file path directly. Respects .gitignore; skips dependency/build folders. No matches does not prove absence/inaccessibility.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search text; query or pattern required." },
                "pattern": { "type": "string", "description": "Alias for query." },
                "path": { "type": "string", "description": "File or directory path, default cwd; relative or explicit absolute/~/ allowed." },
                "regex": { "type": "boolean", "description": "Default false: literal substring." },
                "case_sensitive": { "type": "boolean", "description": "Default false." },
                "include_hidden": { "type": "boolean", "description": "Include dotfiles and hidden entries" },
                "glob": { "type": "string", "description": "Filter relative paths/names, e.g. src/**/*.{rs,ts}." },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Default content: lines; count: per-file counts." },
                "context": { "type": "integer", "description": "Lines before/after matches; content only, default 0." },
                "max_results": { "type": "integer", "description": "Default 100, max 1000." }
            },
            "required": []
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_glob_files_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__glob_files".to_string(),
        name: "glob".to_string(),
        description: "Find files/directories; respects .gitignore. Empty results cover only this scope/filters. Read known paths directly.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Relative glob (*, ?, **); directory goes in path." },
                "path": { "type": "string", "description": "Directory, default cwd; relative or explicit absolute/~/ allowed." },
                "include_hidden": { "type": "boolean", "description": "Include dotfiles and hidden entries" },
                "max_results": { "type": "integer", "description": "Default 200, max 500." }
            },
            "required": ["pattern"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_write_file_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__write_file".to_string(),
        name: "write".to_string(),
        description: "Write a full text file: create it if missing, overwrite it if it exists. Use this for requested deliverables and necessary supporting files within the user's task scope; preserve unrelated files. For small changes to an existing file prefer edit. For a code block without saving, answer inline. Returns structured file mutation metadata including diff stats.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: crate::chat::tools::input_schema::<crate::native_tools::WriteFile>(),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_edit_file_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__edit_file".to_string(),
        name: "edit".to_string(),
        description: "Replace exact text in an existing file; prefer over write. Returns mutation metadata/diff stats.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Existing file; relative to cwd or absolute/~/ outside, subject to permissions/approvals." },
                "edits": {
                    "type": "array",
                    "description": "Applied in order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": { "type": "string", "description": "Unique contiguous exact match; omit read's line numbers. Add context if repeated." },
                            "new_string": { "type": "string" }
                        },
                        "required": ["old_string", "new_string"]
                    },
                    "minItems": 1
                }
            },
            "required": ["path", "edits"]
        }),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_run_command_tool() -> ChatToolDefinition {
    let shell_hint = crate::native_tools::run_command_shell_hint();
    ChatToolDefinition {
        id: "native__run_command".to_string(),
        name: "bash".to_string(),
        description: format!("Run a noninteractive host command in a fresh process.{shell_hint} Use call-local env. Do not background finite jobs; parallelize within one command. Nonzero exit returns an error with stdout/stderr. Obey user constraints and required authorization for destructive/network/environment changes."),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "cwd": { "type": "string", "description": "Existing startup directory, default cwd; relative or absolute/~/ outside allowed, subject to permissions/approvals. Not a filesystem boundary. Use this even for spaces, never leading cd." },
                "background": { "type": "boolean", "description": "Returns job_id immediately; automatic for never-ending dev servers." },
                "timeout_ms": { "type": "integer", "minimum": 0, "description": "Foreground deadline: default 300000ms, 0 unlimited, positive clamped 1000–600000. Kills on timeout with partial output; never backgrounds." },
                "allow_host_python_package_install": { "type": "boolean", "description": "Host Python installs require true AND explicit user request; use --user or a venv." }
            },
            "required": ["command"]
        }),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_bash_output_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__bash_output".to_string(),
        name: "bash_output".to_string(),
        description: "Inspect bash background jobs: stdout/stderr, status/exit_code, next_offset; pass the returned next_offset as since_offset. Refresh before reporting results. Jobs survive turns until killed/app exit; finite jobs belong in foreground bash.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "description": "Returned job ID; omit to list all tracked jobs with status/command/cwd/age." },
                "since_offset": { "type": "integer", "description": "Byte offset, default 0." },
                "wait_ms": { "type": "integer", "description": "Wait for exit, not output; default 30000ms, 0 snapshot, capped at tool timeout." }
            }
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_kill_background_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__kill_background".to_string(),
        name: "kill_background".to_string(),
        description: "Stop a background command started by bash (background:true) by killing its process group. Pass the job_id. Use this to stop a dev server or other long-running background process when you are done with it; otherwise it keeps running until the app exits.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "description": "The job_id of the background command to kill" }
            },
            "required": ["job_id"]
        }),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_save_assistant_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__save_assistant".to_string(),
        name: "save_assistant".to_string(),
        description: "Create a new Kivio assistant (专家). ONLY available while building an assistant by chat, and only call it after you have restated the full config and the user confirmed. system_prompt is the assistant's own instructions (write it in the user's language). mcp_server_ids and skill_ids MUST be chosen from the available lists given in your builder instructions — use the exact ids, never invent them; leave a list empty if none apply. Returns the new assistant id.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Assistant display name (1-64 chars)" },
                "description": { "type": "string", "description": "Short one-line description (optional)" },
                "icon": { "type": "string", "description": "Optional short icon label/emoji" },
                "color": { "type": "string", "description": "Optional hex color like #6A8FBD" },
                "system_prompt": { "type": "string", "description": "The assistant's own system instructions" },
                "mcp_server_ids": { "type": "array", "items": { "type": "string" }, "description": "Allowed MCP server ids (exact ids from the available list; empty = none)" },
                "skill_ids": { "type": "array", "items": { "type": "string" }, "description": "Allowed skill ids (exact ids from the available list; empty = none)" }
            },
            "required": ["name", "system_prompt"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

fn native_orchestration_tool(
    name: &str,
    description: &str,
    input_schema: serde_json::Value,
) -> ChatToolDefinition {
    ChatToolDefinition {
        id: format!("native__{name}"),
        name: name.to_string(),
        description: description.to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema,
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_kivio_list_agents_tool() -> ChatToolDefinition {
    native_orchestration_tool(
        "kivio_list_agents",
        "List available runtimes: the built-in Kivio agent and enabled, available external CLI agents. Returns {agents: [{id, name}]}; use an id as the agent when creating a conversation.",
        serde_json::json!({ "type": "object", "properties": {} }),
    )
}

pub fn native_kivio_list_projects_tool() -> ChatToolDefinition {
    native_orchestration_tool(
        "kivio_list_projects",
        "List Kivio projects. Returns {projects: [{id, name, root_path}]}; use an id as project_id when listing or creating conversations.",
        serde_json::json!({ "type": "object", "properties": {} }),
    )
}

pub fn native_kivio_list_conversations_tool() -> ChatToolDefinition {
    native_orchestration_tool(
        "kivio_list_conversations",
        "List recent Kivio conversations, optionally within a project. The voice brain conversation is excluded. Returns {conversations: [{id, title, project, runtime, running, updated_at}]}.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string", "description": "Only include conversations in this project." },
                "limit": { "type": "integer", "default": 20, "description": "Maximum number of conversations (default 20)." }
            }
        }),
    )
}

pub fn native_kivio_read_conversation_tool() -> ChatToolDefinition {
    native_orchestration_tool(
        "kivio_read_conversation",
        "Read recent messages in a Kivio conversation. Returns {conversation_id, running, messages: [{role, text}]}.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "conversation_id": { "type": "string", "description": "Kivio conversation id." },
                "limit": { "type": "integer", "default": 6, "description": "Maximum number of recent messages (default 6)." }
            },
            "required": ["conversation_id"]
        }),
    )
}

pub fn native_kivio_create_conversation_tool() -> ChatToolDefinition {
    native_orchestration_tool(
        "kivio_create_conversation",
        "Create a Kivio conversation and send its first prompt. Choose the built-in Kivio runtime or an available external CLI agent using an id from kivio_list_agents. Returns {conversation_id, status, response?}: wait=false queues the reply and returns status queued; wait=true returns status completed with any response, or running after the 10-minute wait times out. This creates a separate conversation, not the voice brain.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "First message to send." },
                "title": { "type": "string", "description": "Optional conversation title." },
                "project_id": { "type": "string", "description": "Optional project id." },
                "agent": { "type": "string", "description": "Runtime id from kivio_list_agents; omit for the built-in Kivio agent." },
                "provider_id": { "type": "string", "description": "Optional model provider id." },
                "model": { "type": "string", "description": "Optional model id." },
                "wait": { "type": "boolean", "default": false, "description": "Wait for a reply instead of returning after it is queued." }
            },
            "required": ["prompt"]
        }),
    )
}

pub fn native_kivio_send_message_tool() -> ChatToolDefinition {
    native_orchestration_tool(
        "kivio_send_message",
        "Send a message to an existing Kivio conversation using its bound built-in or external CLI runtime; the voice brain cannot be targeted. Returns {conversation_id, status, response?}: wait=false queues the reply and returns status queued; wait=true returns status completed with any response, or running after the 10-minute wait times out.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "conversation_id": { "type": "string", "description": "Kivio conversation id." },
                "text": { "type": "string", "description": "Message to send." },
                "wait": { "type": "boolean", "default": false, "description": "Wait for a reply instead of returning after it is queued." }
            },
            "required": ["conversation_id", "text"]
        }),
    )
}

pub fn native_kivio_interrupt_tool() -> ChatToolDefinition {
    native_orchestration_tool(
        "kivio_interrupt",
        "Interrupt the current reply in a built-in or external CLI conversation without deleting it. Returns {conversation_id, interrupted: true}.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "conversation_id": { "type": "string", "description": "Kivio conversation id." }
            },
            "required": ["conversation_id"]
        }),
    )
}

fn native_html_tool(name: &str) -> ChatToolDefinition {
    let mut definition = crate::html_render::tool_definitions()
        .into_iter()
        .find(|definition| definition["name"] == name)
        .expect("registered HTML tool has a definition");
    ChatToolDefinition {
        id: format!("native__{name}"),
        name: name.to_owned(),
        description: definition["description"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        source: "native".to_owned(),
        server_id: None,
        server_name: Some("Kivio".to_owned()),
        input_schema: definition["inputSchema"].take(),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_html_render_tool() -> ChatToolDefinition {
    native_html_tool(crate::html_render::HTML_RENDER_TOOL)
}

pub fn native_html_preview_tool() -> ChatToolDefinition {
    native_html_tool(crate::html_render::HTML_PREVIEW_TOOL)
}

/// `present_artifacts` only takes id/path strings. Models sometimes dump
/// base64 or file contents into the argument stream; abort past this.
pub const PRESENT_ARTIFACTS_ARGUMENTS_MAX_CHARS: usize = 8_192;

pub fn native_present_artifacts_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__present_artifacts".to_string(),
        name: "present_artifacts".to_string(),
        description: "Select final deliverables (max 16); unselected files stay hidden. Never send contents/bytes/base64/data URLs. Final references: [label](artifact:art_ID), images ![description](artifact:art_ID); known IDs need no call. Hide QA screenshots/frames/drafts/failed attempts by default. Example: {\"artifact_ids\":[\"art_…\"]}".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "artifact_ids": {
                    "type": "array",
                    "description": "Exact returned `art_…` IDs; never placeholders. Omit/[] for local paths. Generated files use IDs, not invented paths.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 16
                },
                "paths": {
                    "type": "array",
                    "description": "Existing files read/written; obtains art_ IDs. Omit/[] for ID-only selection; never select the same file both ways.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 16
                },
                "mode": {
                    "type": "string",
                    "enum": ["prepare", "preview"],
                    "description": "Default prepare: register without display. preview: immediate display only for explicit user preview/necessary choice."
                },
                "caption": {
                    "type": "string",
                    "description": "Optional plain-text caption.",
                    "maxLength": 300
                }
            },
            // 不用顶层 anyOf 表达“二选一必填”：grok/Vertex/Anthropic 都会拒或需剥离，
            // call_present_artifacts 在运行时校验，模型侧靠 description 提示即可。
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_memory_read_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__memory_read".to_string(),
        name: "memory_read".to_string(),
        description: "Read Kivio Chat memory. L1 is online memory already injected when memory is enabled; use this mainly to inspect exact L1 text or read L2 long-term memory by exact query/heading.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "layer": {
                    "type": "string",
                    "enum": ["l1", "l2"],
                    "description": "Memory layer to read"
                },
                "query": {
                    "type": "string",
                    "description": "Optional exact text/heading query. Especially useful for L2."
                },
                "maxBytes": {
                    "type": "integer",
                    "description": "Maximum bytes returned to the model"
                }
            },
            "required": ["layer"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_memory_modify_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__memory_modify".to_string(),
        name: "memory_modify".to_string(),
        description: "Modify Kivio Chat memory. Use for adding, replacing, removing, or archiving durable user-approved memory. L1 is short online memory limited to 5000 bytes; L2 is long-term memory that is never auto-loaded.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "layer": {
                    "type": "string",
                    "enum": ["l1", "l2"],
                    "description": "Target memory layer"
                },
                "operation": {
                    "type": "string",
                    "enum": ["append", "replace", "remove", "archive"],
                    "description": "Modification operation"
                },
                "content": {
                    "type": "string",
                    "description": "New Markdown content for append/replace, or optional archived content"
                },
                "oldText": {
                    "type": "string",
                    "description": "Exact unique snippet for replace/remove/archive"
                },
                "heading": {
                    "type": "string",
                    "description": "Optional L2 heading to append/archive under"
                },
                "archiveMode": {
                    "type": "string",
                    "enum": ["move", "copy"],
                    "description": "archive only; default is move"
                }
            },
            "required": ["layer", "operation"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_memory_search_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__memory_search".to_string(),
        name: "memory_search".to_string(),
        description: "Search Kivio Chat long-term memory (L2) by keywords and get the most relevant entries back as heading + snippet. Prefer this over memory_read when you are not sure of the exact L2 heading: memory_read needs an exact heading/text match, while memory_search ranks sections by query-token overlap.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to search for in long-term memory"
                },
                "maxResults": {
                    "type": "integer",
                    "description": "Maximum number of matching entries to return (default 5, max 20)"
                },
                "layer": {
                    "type": "string",
                    "enum": ["l1", "l2"],
                    "description": "Memory layer to search; defaults to l2 (L1 is small and already injected)"
                }
            },
            "required": ["query"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn mixer_generate_image_tool() -> ChatToolDefinition {
    mixer_generate_image_tool_for(None)
}

/// Mixer 生图工具。`model` 写进 description，因为发给上游的只有 name/description/parameters，
/// `server_id` 只给 UI，模型看不见。
pub fn mixer_generate_image_tool_for(model: Option<&str>) -> ChatToolDefinition {
    ChatToolDefinition {
        id: "mixer__generate_image".to_string(),
        name: "mixer_generate_image".to_string(),
        description: mixer_generate_image_description(model),
        source: "mixer".to_string(),
        server_id: None,
        server_name: Some("Mixer".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "What the image should look like, or how to change the reference"
                },
                "size": {
                    "type": "string",
                    "description": "Only if the user asked. 512, 1K, 2K, 4K, or WIDTHxHEIGHT such as 1536x864."
                },
                "aspect_ratio": {
                    "type": "string",
                    "enum": [
                        "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "4:5", "5:4", "21:9",
                        "2:1", "1:2", "1:4", "4:1"
                    ],
                    "description": "Only if the user asked for a shape. Omit to let the model choose."
                },
                "quality": {
                    "type": "string",
                    "enum": ["low", "medium", "high"],
                    "description": "Only if the user asked"
                },
                "n": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 4,
                    "description": "How many images. Default 1"
                },
                "paths": {
                    "type": "array",
                    "description": "Local image files to edit. Omit or use [] when no disk file is needed; never invent placeholder paths.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 4
                },
                "artifact_ids": {
                    "type": "array",
                    "description": "Exact art_ IDs from earlier generate/edit results. Omit or use [] when no prior artifact is needed; never invent placeholder IDs.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 4
                }
            },
            "required": ["prompt"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

fn mixer_generate_image_description(model: Option<&str>) -> String {
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("the Mixer image model from Settings");
    format!(
        "Generate or edit an image with {model}. The image model is fixed; do not pass a model name.\n\
         New image: {{\"prompt\":\"a fox in snow\"}}\n\
         Widescreen 2K: {{\"prompt\":\"city skyline\",\"aspect_ratio\":\"16:9\",\"size\":\"2K\"}}\n\
         4K: {{\"prompt\":\"poster\",\"aspect_ratio\":\"16:9\",\"size\":\"4K\"}}\n\
         Custom pixels: {{\"prompt\":\"poster\",\"size\":\"1536x864\"}}\n\
         Edit a file: {{\"prompt\":\"make it night\",\"paths\":[\"C:/ref.png\"]}}\n\
         Edit a prior result: {{\"prompt\":\"add a title\",\"artifact_ids\":[\"art_...\"]}}\n\
         User attached images this turn: omit paths and artifact_ids.\n\
         Omit size and aspect_ratio unless the user asked. quality=low|medium|high only if asked. n=1-4."
    )
}

pub fn native_web_fetch_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__web_fetch".to_string(),
        name: "web_fetch".to_string(),
        description: "Fetch readable text from an HTTPS URL. Uses the configured fetch provider's extract API when available (independent of the search provider: Tavily, Exa, Ollama, TinyFish, Serper, Kimi); otherwise fetches the page directly (HTML stripped to plain text, with a hosted reader fallback).".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "HTTPS URL" },
                "reader_fallback": {
                    "type": "boolean",
                    "description": "Whether to try a hosted reader fallback when direct fetch fails or returns too little readable text. Defaults to true."
                }
            },
            "required": ["url"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_automation_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__automation".into(), name: "automation".into(),
        description: "Manage Kivio workflow graphs; use schedule action=create for timed prompts. List first to avoid duplicates. get/set_enabled/run/runs/delete require id; set_enabled also needs enabled. upsert needs a complete automation graph; omit its id to create and positions to auto-layout. Validation errors return allowedNodeTypes and schemaHint. run waits (default 600s, max 1800), then returns runId to poll with runs; delete is permanent.".into(),
        source: "native".into(), server_id: None, server_name: Some("Kivio".into()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type":"string", "enum":["list","get","upsert","set_enabled","run","runs","delete"]},
                "id": {"type":"string"},
                "automation": {"type":"object", "description":"Complete graph: {id?, name, enabled, nodes:[{id,type,data}], edges:[{id,source,target,sourceHandle?,targetHandle?}]}."},
                "dry_run": {"type":"boolean", "description":"Validate a complete graph without saving; not for discovering node types."},
                "enabled": {"type":"boolean"},
                "input": {"description":"run payload: string or {text,json}; available as {{output}} / {{json.input.*}}."},
                "timeout_seconds": {"type":"integer"},
                "run_id": {"type":"string", "description":"runs: fetch one run's per-node details."}
            },
            "required":["action"]
        }),
        sensitive: true, annotations: None, output_schema: None,
    }
}

/// `knowledge_search` — retrieve passages from the user's knowledge bases
/// (RAG). Read-only. Returns passages each tagged with a `[n]` citation marker.
pub fn native_knowledge_search_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__knowledge_search".to_string(),
        name: "knowledge_search".to_string(),
        description: "Search the user's knowledge base(s) for passages relevant to a query and return them with citation markers. Use this whenever the question may be answered by the user's uploaded documents. Each returned passage is prefixed with a [n] marker and its source; when you use a passage, cite it inline as [n]. If no relevant passage is returned, say you don't have that information in the knowledge base instead of guessing. This searches only the libraries the user attached to the current conversation; if none are attached it returns nothing.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to look for. Use a focused natural-language query."
                },
                "kb_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: restrict the search to these knowledge base ids. Defaults to the libraries attached to the current conversation; if none are attached, the search returns nothing."
                },
                "top_k": {
                    "type": "integer",
                    "description": "Optional: number of passages to return (default 5)."
                }
            },
            "required": ["query"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

/// `advisor` — consult a stronger model (executor-advisor pattern). The main
/// loop model calls this on demand when it is stuck, repeatedly failing, or
/// facing a significant design decision; the advisor model configured under
/// Mixer answers in a single shot (no tools, no recursion). Read-only.
pub fn native_advisor_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__advisor".to_string(),
        name: "advisor".to_string(),
        description: "Consult a stronger advisor model for guidance. Use this when you are stuck, have failed the same approach repeatedly, or face a significant design/architecture decision — NOT for routine steps you can handle yourself. Pass the specific question and enough context (relevant code, what you already tried, constraints). Returns the advisor's diagnosis and direction; you remain responsible for carrying it out.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The specific question or problem you want advice on."
                },
                "context": {
                    "type": "string",
                    "description": "Optional but recommended: relevant code, constraints, error messages, and what you have already tried. The advisor sees only what you pass here."
                }
            },
            "required": ["question"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

/// Builtin native tool exposure: iterates the static registry in
/// `mcp/native_registry.rs` (declaration order = model-facing order).
pub fn list_native_builtin_tool_defs(
    native: &ChatNativeToolsConfig,
    web_search_configured: bool,
    memory_enabled: bool,
) -> Vec<ChatToolDefinition> {
    super::native_registry::NATIVE_TOOLS
        .iter()
        .filter(|entry| (entry.enabled)(native, web_search_configured, memory_enabled))
        .map(|entry| (entry.def)())
        .collect()
}

#[cfg(test)]
const MAX_WIRE_TOOL_NAME: usize = 64;
pub use kivio_agent_sdk::mcp::sanitize_openai_tool_name;

/// Shared wire-name builder for [`ChatToolDefinition`] and `ModelTool`.
pub fn wire_tool_name(source: &str, name: &str, id: &str, server_id: Option<&str>) -> String {
    match source {
        // Native and Skill tools are model-facing APIs owned by Kivio. Keep their names
        // aligned with the system prompt so models can call exactly what we instruct.
        "native" | "skill" | "mixer" => apply_reserved_wire_alias(&sanitize_openai_tool_name(name)),
        _ => kivio_agent_sdk::mcp::wire_mcp_tool_id(id, server_id),
    }
}

pub fn looks_sensitive_tool(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "write", "delete", "remove", "exec", "shell", "command", "run", "update", "patch", "move",
        "rename", "create", "save", "upload", "publish", "replace", "modify", "edit", "insert",
        "drop", "truncate", "grant", "revoke", "deploy", "apply",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_tool_legacy_names_route_and_preserve_their_action() {
        let settings = crate::settings::Settings::default();
        for tool in [
            native_automation_tool(),
            crate::scheduled_tasks::tools::tool_definition(),
        ] {
            let definition = tool.for_run(&settings);
            for (legacy, canonical) in LEGACY_TOOL_ALIASES
                .iter()
                .filter(|(_, canonical)| *canonical == tool.name)
            {
                assert_eq!(
                    crate::chat::agent::execute::match_tool_call(
                        std::slice::from_ref(&definition),
                        legacy
                    )
                    .unwrap()
                    .label
                    .name,
                    *canonical
                );
                let mut arguments = serde_json::json!({"action":"delete", "id":"existing"});
                inject_legacy_tool_action(legacy, &mut arguments);
                assert_eq!(
                    arguments["action"],
                    legacy.strip_prefix(&format!("{canonical}_")).unwrap()
                );
                assert_eq!(arguments["id"], "existing");
            }
        }
    }

    #[test]
    fn voice_orchestration_tool_schemas_match_their_contract() {
        use serde_json::json;

        let cases = [
            (
                native_kivio_list_agents_tool(),
                "kivio_list_agents",
                json!({}),
                json!([]),
            ),
            (
                native_kivio_list_projects_tool(),
                "kivio_list_projects",
                json!({}),
                json!([]),
            ),
            (
                native_kivio_list_conversations_tool(),
                "kivio_list_conversations",
                json!({"project_id": "string", "limit": "integer"}),
                json!([]),
            ),
            (
                native_kivio_read_conversation_tool(),
                "kivio_read_conversation",
                json!({"conversation_id": "string", "limit": "integer"}),
                json!(["conversation_id"]),
            ),
            (
                native_kivio_create_conversation_tool(),
                "kivio_create_conversation",
                json!({
                    "prompt": "string", "title": "string", "project_id": "string",
                    "agent": "string", "provider_id": "string", "model": "string",
                    "wait": "boolean"
                }),
                json!(["prompt"]),
            ),
            (
                native_kivio_send_message_tool(),
                "kivio_send_message",
                json!({"conversation_id": "string", "text": "string", "wait": "boolean"}),
                json!(["conversation_id", "text"]),
            ),
            (
                native_kivio_interrupt_tool(),
                "kivio_interrupt",
                json!({"conversation_id": "string"}),
                json!(["conversation_id"]),
            ),
        ];

        for (tool, name, properties, required) in cases {
            assert_eq!(tool.name, name);
            assert_eq!(tool.id, format!("native__{name}"));
            assert_eq!(tool.openai_tool_name(), name);
            assert_eq!(tool.input_schema["type"], "object");
            let actual = tool.input_schema["properties"].as_object().unwrap();
            let expected = properties.as_object().unwrap();
            assert_eq!(actual.len(), expected.len(), "{name}");
            for (property, expected_type) in expected {
                assert_eq!(
                    &actual.get(property).unwrap()["type"],
                    expected_type,
                    "{name}.{property}"
                );
            }
            assert_eq!(
                tool.input_schema
                    .get("required")
                    .cloned()
                    .unwrap_or(json!([])),
                required,
                "{name}"
            );
            match name {
                "kivio_list_conversations" => {
                    assert_eq!(actual.get("limit").unwrap()["default"], 20)
                }
                "kivio_read_conversation" => assert_eq!(actual.get("limit").unwrap()["default"], 6),
                "kivio_create_conversation" | "kivio_send_message" => {
                    assert_eq!(actual.get("wait").unwrap()["default"], false);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn sensitive_tool_detection_covers_common_write_verbs() {
        for name in ["save_file", "uploadAsset", "publish_page", "replace_rows"] {
            assert!(
                looks_sensitive_tool(name),
                "{name} should require confirmation"
            );
        }
        assert!(!looks_sensitive_tool("read_file"));
        assert!(!looks_sensitive_tool("web_search"));
    }

    #[test]
    fn mixer_image_tool_description_names_model_and_shows_calls() {
        let unnamed = mixer_generate_image_tool();
        assert!(unnamed.description.contains("Mixer image model"));
        assert!(unnamed
            .description
            .contains("{\"prompt\":\"a fox in snow\"}"));
        assert!(unnamed.description.contains("aspect_ratio"));
        assert!(unnamed.description.contains("2K"));
        assert!(unnamed.description.contains("paths"));
        assert!(unnamed.description.contains("artifact_ids"));

        let named = mixer_generate_image_tool_for(Some("gpt-image-2"));
        assert!(
            named.description.contains("gpt-image-2"),
            "{}",
            named.description
        );
        assert!(named.description.contains("do not pass a model name"));
        let wire = named.to_openai_tool();
        assert_eq!(wire["function"]["name"], "mixer_generate_image");
        assert!(wire["function"]["description"]
            .as_str()
            .is_some_and(|text| text.contains("gpt-image-2")));
    }

    #[test]
    fn skill_and_native_tools_use_prompt_facing_names() {
        assert_eq!(native_skill_activate_tool().openai_tool_name(), "skill");
        // 保留名规避：native web_search 在 wire/prompt 上声明为别名 search_web。
        assert_eq!(native_web_search_tool().openai_tool_name(), "search_web");
    }

    #[test]
    fn skill_activate_legacy_name_maps_to_skill() {
        assert_eq!(canonical_tool_name("skill_activate"), "skill");
    }

    #[test]
    fn native_skill_tools_expose_single_tool() {
        let tools = native_skill_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "skill");
    }

    #[test]
    fn builtin_skill_tools_are_not_marked_sensitive() {
        assert!(!native_skill_activate_tool().sensitive);
    }

    #[test]
    fn automation_tool_description_stays_short() {
        let def = native_automation_tool();
        assert!(
            def.description.contains("Validation errors return"),
            "{}",
            def.description
        );
        assert!(
            !def.description.contains("Allowed node types"),
            "schema belongs in the skill and validation errors, not the always-on tool"
        );
        assert!(
            !def.description.contains("Minimal example"),
            "{}",
            def.description
        );
        assert!(
            def.description.len() < 500,
            "upsert description grew to {} chars",
            def.description.len()
        );
    }

    #[test]
    fn present_artifacts_tool_tells_model_to_pass_ids_only() {
        let def = native_present_artifacts_tool();
        assert!(
            def.description.contains(r#"{"artifact_ids":["art_…"]}"#),
            "{}",
            def.description
        );
        assert!(def.description.contains("base64"), "{}", def.description);
        assert!(
            def.input_schema["properties"]["artifact_ids"]["description"]
                .as_str()
                .unwrap()
                .contains("`art_…`"),
        );
        assert!(def.input_schema["properties"]["caption"]["description"]
            .as_str()
            .unwrap()
            .contains("plain-text"),);
    }

    #[test]
    fn image_tool_unused_sources_allow_empty_arrays() {
        for tool in [
            native_read_file_tool(),
            native_present_artifacts_tool(),
            mixer_generate_image_tool(),
        ] {
            for field in ["artifact_ids", "paths"] {
                let minimum = tool.input_schema["properties"][field]["minItems"]
                    .as_u64()
                    .unwrap_or(0);
                assert_eq!(
                    minimum, 0,
                    "{}.{field} must not force placeholder entries",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn native_file_and_web_tools_have_expected_sensitivity() {
        assert!(!native_read_file_tool().sensitive);
        assert!(!native_web_fetch_tool().sensitive);
        assert!(!native_memory_read_tool().sensitive);
        assert!(!native_memory_modify_tool().sensitive);
        assert!(native_write_file_tool().sensitive);
        assert!(native_edit_file_tool().sensitive);
        assert!(native_run_command_tool().sensitive);
    }

    #[test]
    fn bash_timeout_schema_accepts_zero_and_values_above_the_cap() {
        let tool = native_run_command_tool();
        for timeout in [0, 600_001] {
            assert!(crate::chat::tool_runtime::validate_tool_arguments(
                &tool,
                &serde_json::json!({ "command": "echo ok", "timeout_ms": timeout })
            )
            .is_ok());
        }
        assert!(crate::chat::tool_runtime::validate_tool_arguments(
            &tool,
            &serde_json::json!({ "command": "echo ok", "timeout_ms": -1 })
        )
        .is_err());
    }

    #[test]
    fn file_tool_path_descriptions_are_scope_specific() {
        let read_schema = native_read_file_tool().input_schema;
        let grep = native_search_files_tool();
        let find = native_glob_files_tool();

        assert!(read_schema["properties"]["path"]["description"]
            .as_str()
            .unwrap()
            .contains("File path"));
        assert!(read_schema["properties"].get("paths").is_some());
        assert!(read_schema.get("required").is_none());
        // read 现在也列目录，描述里应提到目录
        assert!(native_read_file_tool().description.contains("directory"));
        assert!(grep.description.contains("file or under a directory"));
        assert!(grep.description.contains("pass that file path directly"));
        assert!(grep.input_schema["properties"]["path"]["description"]
            .as_str()
            .unwrap()
            .contains("File or directory path"));
        assert!(find.input_schema["properties"]["path"]["description"]
            .as_str()
            .unwrap()
            .contains("Directory"));
        for tool in [
            native_read_file_tool(),
            grep,
            find,
            native_write_file_tool(),
            native_edit_file_tool(),
            native_run_command_tool(),
        ] {
            let declaration = tool.to_openai_tool().to_string();
            assert!(
                !declaration.contains("when allowed by workspace mode"),
                "{}",
                tool.name
            );
            assert!(
                !declaration.contains("validated as workspace-local"),
                "{}",
                tool.name
            );
            assert!(
                declaration.contains("absolute"),
                "{} must describe explicit paths",
                tool.name
            );
        }
        assert!(!native_skill_activate_tool()
            .description
            .contains("`run_command`"));
    }

    #[test]
    fn write_file_tool_description_discourages_inline_code_requests() {
        let tool = native_write_file_tool();

        assert!(tool.description.contains("necessary supporting files"));
        assert!(tool.description.contains("answer inline"));
        assert!(tool.description.contains("code block"));
        assert!(tool.description.contains("prefer edit"));
        assert!(tool
            .description
            .contains("structured file mutation metadata"));
    }

    #[test]
    fn write_gate_exposes_exactly_whole_file_and_path_tools() {
        // Start from an explicitly write-disabled config so this gate test is
        // independent of the default-on baseline.
        let mut native = crate::settings::ChatNativeToolsConfig::default();
        native.write_file = false;
        native.edit_file = false;
        let defs = list_native_builtin_tool_defs(&native, false, false);
        assert!(defs.is_empty() || !defs.iter().any(|tool| tool.name == "write"));

        native.write_file = true;
        native.edit_file = true;
        let defs = list_native_builtin_tool_defs(&native, false, false);
        let names: Vec<&str> = defs.iter().map(|tool| tool.name.as_str()).collect();
        for expected in ["write", "edit"] {
            assert!(names.contains(&expected), "{expected} should be exposed");
        }
        for removed in [
            "create_dir",
            "delete_path",
            "move_path",
            "copy_path",
            "patch",
            "write_file_chunk",
            "begin_file_write",
            "append_file_write",
            "finish_file_write",
            "abort_file_write",
        ] {
            assert!(!names.contains(&removed), "{removed} must not be exposed");
        }
    }

    #[test]
    fn default_native_config_exposes_file_and_command_tools() {
        // Regression for the "sub-agent stuck on skill_activate" bug: with the
        // agentic default config (all native tools ON), the agent — and its
        // sub-agents, which inherit the same tool table — must actually receive
        // read/ls/grep/find/write/edit/bash. Before the green-light default, only
        // skill tools were exposed, so weak models looped guessing skill names.
        // web_search is intentionally NOT asserted here: it stays gated behind a
        // configured provider key (web_search_configured=false below).
        let native = crate::settings::ChatNativeToolsConfig::default();
        let defs = list_native_builtin_tool_defs(&native, false, false);
        let names: Vec<&str> = defs.iter().map(|tool| tool.name.as_str()).collect();
        for expected in ["read", "grep", "glob", "write", "edit", "bash"] {
            assert!(
                names.contains(&expected),
                "default config must expose `{expected}` (got {names:?})"
            );
        }
    }

    #[test]
    fn mcp_tools_keep_namespaced_openai_names() {
        let server = ChatMcpServer {
            id: "server.one".to_string(),
            name: "Server One".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };
        let tool = tool_definition_from_mcp(
            &server,
            McpTool {
                name: "search.web".to_string(),
                description: String::new(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: None,
                annotations: None,
            },
        );

        assert_eq!(tool.openai_tool_name(), "mcp__server_one__search_web");
        assert_ne!(tool.openai_tool_name(), tool.name);
    }

    #[test]
    fn mcp_tool_definition_preserves_metadata_and_read_only_hint() {
        let server = ChatMcpServer {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };
        let annotations = serde_json::json!({ "readOnlyHint": true });
        let output_schema = serde_json::json!({
            "type": "object",
            "properties": { "items": { "type": "array" } }
        });

        let tool = tool_definition_from_mcp(
            &server,
            McpTool {
                name: "search".to_string(),
                description: "Search".to_string(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: Some(output_schema.clone()),
                annotations: Some(annotations.clone()),
            },
        );

        assert_eq!(tool.annotations.as_ref(), Some(&annotations));
        assert_eq!(tool.output_schema.as_ref(), Some(&output_schema));
        assert_eq!(tool.read_only_hint(), Some(true));
        assert!(tool.is_read_only_tool());
        assert!(tool.parallel_safe());
        assert!(tool.sensitive);
    }

    #[test]
    fn mcp_destructive_and_open_world_annotations_are_sensitive() {
        let server = ChatMcpServer {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };

        for annotations in [
            serde_json::json!({ "destructiveHint": true }),
            serde_json::json!({ "openWorldHint": true }),
            serde_json::json!({ "readOnlyHint": false }),
        ] {
            let tool = tool_definition_from_mcp(
                &server,
                McpTool {
                    name: "remote_action".to_string(),
                    description: "Remote action".to_string(),
                    input_schema: serde_json::json!({ "type": "object" }),
                    output_schema: None,
                    annotations: Some(annotations),
                },
            );
            assert!(tool.sensitive);
        }
    }

    #[test]
    fn mcp_open_world_annotation_overrides_read_only_hint() {
        let server = ChatMcpServer {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };

        let tool = tool_definition_from_mcp(
            &server,
            McpTool {
                name: "remote_search".to_string(),
                description: "Remote search".to_string(),
                input_schema: serde_json::json!({ "type": "object" }),
                output_schema: None,
                annotations: Some(serde_json::json!({
                    "readOnlyHint": true,
                    "openWorldHint": true
                })),
            },
        );

        assert!(tool.sensitive);
        assert!(!tool.is_read_only_tool());
        assert!(!tool.parallel_safe());
    }

    #[test]
    fn reserved_wire_alias_maps_web_search_both_ways() {
        // Cursor 系上游把 web_search 当内部保留工具吞掉（实测空响应）——wire 层全局改名。
        assert_eq!(apply_reserved_wire_alias("web_search"), "search_web");
        assert_eq!(apply_reserved_wire_alias("web_fetch"), "web_fetch");
        assert_eq!(resolve_reserved_wire_alias("search_web"), "web_search");
        assert_eq!(resolve_reserved_wire_alias("read"), "read");
    }

    #[test]
    fn native_web_search_tool_declares_alias_on_the_wire() {
        let tool = ChatToolDefinition {
            id: "native__web_search".to_string(),
            name: "web_search".to_string(),
            description: "Search the web".to_string(),
            source: "native".to_string(),
            server_id: None,
            server_name: Some("Kivio".to_string()),
            input_schema: serde_json::json!({ "type": "object" }),
            sensitive: false,
            annotations: None,
            output_schema: None,
        };
        assert_eq!(tool.openai_tool_name(), "search_web");
        let wire = tool.to_openai_tool();
        assert_eq!(wire["function"]["name"], "search_web");
        // MCP 工具不受别名影响（按 id 命名）。
        let mcp_tool = ChatToolDefinition {
            source: "mcp".to_string(),
            id: "mcp__srv__web_search".to_string(),
            ..tool
        };
        assert_eq!(mcp_tool.openai_tool_name(), "mcp__srv__web_search");
    }

    #[test]
    fn canonical_tool_name_maps_legacy_names() {
        // 旧名 → 现名（移除/合并/改名后仍可路由）。
        assert_eq!(canonical_tool_name("read_file"), "read");
        assert_eq!(canonical_tool_name("write_file"), "write");
        assert_eq!(canonical_tool_name("edit_file"), "edit");
        assert_eq!(canonical_tool_name("list_dir"), "read");
        assert_eq!(canonical_tool_name("ls"), "read");
        assert_eq!(canonical_tool_name("search_files"), "grep");
        assert_eq!(canonical_tool_name("glob_files"), "glob");
        assert_eq!(canonical_tool_name("find"), "glob");
        assert_eq!(canonical_tool_name("run_command"), "bash");
        assert_eq!(canonical_tool_name("list_background"), "bash_output");
        assert_eq!(canonical_tool_name("todo_update"), "todo_write");
        // 现名与未知名原样返回。
        assert_eq!(canonical_tool_name("read"), "read");
        assert_eq!(canonical_tool_name("glob"), "glob");
        assert_eq!(canonical_tool_name("bash"), "bash");
        assert_eq!(canonical_tool_name("read_file_extra"), "read_file_extra");
    }

    #[test]
    fn sanitize_keeps_short_names_and_hash_suffixes_long_ones() {
        assert_eq!(sanitize_openai_tool_name("search.web"), "search_web");
        assert_eq!(sanitize_openai_tool_name("read_file"), "read_file");
        let long = "a".repeat(80);
        let wire = sanitize_openai_tool_name(&long);
        assert_eq!(wire.chars().count(), MAX_WIRE_TOOL_NAME);
        assert!(wire.contains('_'), "{wire}");
        assert_ne!(
            sanitize_openai_tool_name(&format!("{}x", "a".repeat(79))),
            wire,
            "distinct inputs must not collapse after the 64-char cap"
        );
    }

    #[test]
    fn uuid_mcp_server_tools_keep_distinct_wire_names() {
        // Issue #33: `mcp__<uuid>__zai-mcp-server-ui_*` all left-truncated to
        // the same `...-ui` and Gemini dropped the duplicates.
        let server = ChatMcpServer {
            id: "mcp-d1ad400e-e2c7-4f34-9cd0-4e4cf1afa728".to_string(),
            name: "zhipu".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: "demo".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        };
        let names = [
            "zai-mcp-server-ui_to_artifact",
            "zai-mcp-server-ui_diff_check",
            "zai-mcp-server-analyze_data_visualization",
            "zai-mcp-server-analyze_image",
            "zai-mcp-server-analyze_video",
        ];
        let mut wires = std::collections::HashSet::new();
        for name in names {
            let tool = tool_definition_from_mcp(
                &server,
                McpTool {
                    name: name.to_string(),
                    description: String::new(),
                    input_schema: serde_json::json!({ "type": "object" }),
                    output_schema: None,
                    annotations: None,
                },
            );
            let wire = tool.openai_tool_name();
            assert!(
                wire.chars().count() <= MAX_WIRE_TOOL_NAME,
                "{wire} exceeds 64"
            );
            assert!(
                wire.contains("zai-mcp-server"),
                "tool suffix should stay visible: {wire}"
            );
            assert!(wires.insert(wire.clone()), "collided: {wire}");
        }
        assert_eq!(wires.len(), names.len());
    }
}
