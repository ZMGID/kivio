use std::{collections::HashMap, fs, path::Path, time::Duration};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::oneshot;

use crate::{
    native_tools::{
        resolve_tool_read_path, FileMutationResult, NativeToolWorkspace, ReadFileResult,
    },
    settings::{
        ChatMcpServer, WebSearchProvider, CHAT_TOOL_MAX_TIMEOUT_MS, CHAT_TOOL_MIN_TIMEOUT_MS,
        SKILL_SCRIPT_MIN_TIMEOUT_MS,
    },
    state::AppState,
};

use super::{
    client::{StdioMcpClient, StreamableHttpMcpClient},
    types::{
        list_native_builtin_tool_defs, mixer_generate_image_tool, native_skill_tools,
        tool_definition_from_mcp, ChatToolDefinition, McpToolCallResult,
    },
};

const TOOL_LIST_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct NativeToolContext {
    pub conversation_id: String,
    pub message_id: String,
    pub tool_call_id: Option<String>,
    /// Parent run id of the agent loop issuing this call. Used by sub-agent
    /// management tools to address the parent tool card and cascade
    /// cancellation. Empty when not running under an agent loop.
    #[allow(dead_code)]
    pub run_id: String,
    /// Generation of the issuing agent loop (for cancellation cascade).
    #[allow(dead_code)]
    pub generation: u64,
    /// Sub-agent nesting depth of the issuing agent loop (0 = top-level).
    #[allow(dead_code)]
    pub depth: u8,
}
const MAX_PYTHON_INPUT_FILE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_PYTHON_INPUT_FILES: usize = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PythonInputFilePayload {
    name: String,
    data_base64: String,
    size_bytes: u64,
}

fn sanitize_python_input_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("input");
    let sanitized = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches(['.', ' ', '_']).trim();
    if trimmed.is_empty() {
        "input".to_string()
    } else {
        trimmed.to_string()
    }
}

fn collect_python_input_files(
    _app: &AppHandle,
    workspace: &NativeToolWorkspace,
    arguments: &Value,
) -> Result<Vec<PythonInputFilePayload>, String> {
    let Some(files) = arguments.get("files") else {
        return Ok(Vec::new());
    };
    let files = files
        .as_array()
        .ok_or_else(|| "run_python files must be an array of file paths".to_string())?;
    if files.len() > MAX_PYTHON_INPUT_FILES {
        return Err(format!(
            "run_python supports at most {MAX_PYTHON_INPUT_FILES} input files"
        ));
    }

    let mut payloads = Vec::new();
    for file in files {
        let raw_path = file
            .as_str()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "run_python files entries must be non-empty strings".to_string())?;
        let path = resolve_tool_read_path(workspace, raw_path)?;
        if !path.is_file() {
            return Err(format!("run_python input is not a file: {raw_path}"));
        }
        let metadata =
            fs::metadata(&path).map_err(|err| format!("Read input metadata failed: {err}"))?;
        if metadata.len() > MAX_PYTHON_INPUT_FILE_BYTES {
            return Err(format!(
                "run_python input file too large: {} bytes (max {MAX_PYTHON_INPUT_FILE_BYTES})",
                metadata.len()
            ));
        }
        let bytes = fs::read(&path).map_err(|err| format!("Read input file failed: {err}"))?;
        payloads.push(PythonInputFilePayload {
            name: sanitize_python_input_name(&path),
            data_base64: general_purpose::STANDARD.encode(bytes),
            size_bytes: metadata.len(),
        });
    }
    Ok(payloads)
}

pub(crate) fn effective_skill_script_timeout_ms(
    default_timeout_ms: u64,
    requested_timeout_ms: Option<u64>,
) -> u64 {
    let base_timeout_ms = default_timeout_ms.max(SKILL_SCRIPT_MIN_TIMEOUT_MS);
    requested_timeout_ms
        .unwrap_or(base_timeout_ms)
        .clamp(CHAT_TOOL_MIN_TIMEOUT_MS, CHAT_TOOL_MAX_TIMEOUT_MS)
        .max(base_timeout_ms)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpListToolsResult {
    pub success: bool,
    pub tools: Vec<ChatToolDefinition>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTestResult {
    pub success: bool,
    pub tools: Vec<ChatToolDefinition>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImportResult {
    pub success: bool,
    pub servers: Vec<ChatMcpServer>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CursorMcpJson {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: HashMap<String, CursorMcpServer>,
}

#[derive(Debug, Deserialize)]
struct CursorMcpServer {
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default, rename = "type")]
    server_type: Option<String>,
}

#[tauri::command]
pub async fn chat_mcp_list_tools(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<McpListToolsResult, String> {
    Ok(match list_enabled_tool_defs(&app, &state).await {
        Ok(tools) => McpListToolsResult {
            success: true,
            tools,
            error: None,
        },
        Err(err) => McpListToolsResult {
            success: false,
            tools: Vec::new(),
            error: Some(err),
        },
    })
}

pub async fn list_enabled_tool_defs(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<ChatToolDefinition>, String> {
    let settings = state.settings_read().clone();
    let cache_key = enabled_tools_cache_key(&settings);
    if let Some(tools) = state.get_cached_chat_tools(&cache_key, TOOL_LIST_CACHE_TTL) {
        return Ok(tools);
    }

    let mut tools = list_native_builtin_tool_defs(
        &settings.chat_tools.native_tools,
        web_search_configured(&settings),
        crate::settings::chat_memory_tools_enabled(&settings),
    );
    if let Some((provider_id, model)) = settings.image_generation_model() {
        let mut tool = mixer_generate_image_tool();
        let provider_name = settings
            .get_provider(&provider_id)
            .map(|provider| {
                if provider.name.trim().is_empty() {
                    provider.id.clone()
                } else {
                    provider.name.clone()
                }
            })
            .unwrap_or(provider_id);
        tool.server_id = Some(format!("{provider_name} / {model}"));
        tools.push(tool);
    }

    if settings.chat_tools.enabled {
        // 并行从每个已启用 server 拉工具列表（持久会话，命中即复用）。失败仅置 Error
        // 态并发事件（mcp_get_or_connect 内部已发），不阻塞其他 server。借用 &AppState 的
        // future 用 join_all 并发驱动（无需 'static / spawn）。
        let enabled_servers: Vec<&ChatMcpServer> = settings
            .chat_tools
            .servers
            .iter()
            .filter(|server| server.enabled)
            .collect();
        let listings = enabled_servers.iter().map(|server| async move {
            let result = state.mcp_list_tools(app, server).await;
            (*server, result)
        });
        for (server, result) in futures::future::join_all(listings).await {
            match result {
                Ok(server_tools) => {
                    tools.extend(tools_from_mcp(server, server_tools));
                }
                Err(err) => {
                    eprintln!("MCP server {} failed while listing tools: {err}", server.name);
                }
            }
        }
    }

    tools.extend(list_skill_tool_defs(&settings));

    state.set_cached_chat_tools(cache_key, tools.clone());
    Ok(tools)
}

/// 把 server 返回的 McpTool 列表按 enabled_tools 过滤后映射成 ChatToolDefinition。
fn tools_from_mcp(
    server: &ChatMcpServer,
    tools: Vec<crate::mcp::types::McpTool>,
) -> Vec<ChatToolDefinition> {
    let allowed = server
        .enabled_tools
        .iter()
        .map(|tool| tool.as_str())
        .collect::<Vec<_>>();
    tools
        .into_iter()
        .filter(|tool| allowed.is_empty() || allowed.contains(&tool.name.as_str()))
        .map(|tool| tool_definition_from_mcp(server, tool))
        .collect()
}

#[tauri::command]
pub async fn chat_mcp_test_server(
    state: State<'_, AppState>,
    server: ChatMcpServer,
    timeout_ms: Option<u64>,
) -> Result<McpTestResult, String> {
    // tauri command 必须返回 Result 才能用 State<'_, _>；前端仍读 McpTestResult（永不 Err）。
    Ok(
        match list_server_tools(&state.http, &server, timeout_ms.unwrap_or(60_000)).await {
            Ok(tools) => McpTestResult {
                success: true,
                tools,
                error: None,
            },
            Err(err) => McpTestResult {
                success: false,
                tools: Vec::new(),
                error: Some(err),
            },
        },
    )
}

#[tauri::command]
pub fn chat_mcp_import_json(path: String) -> McpImportResult {
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) => {
            return McpImportResult {
                success: false,
                servers: Vec::new(),
                error: Some(format!("Read mcp.json failed: {err}")),
            }
        }
    };
    let parsed: CursorMcpJson = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            return McpImportResult {
                success: false,
                servers: Vec::new(),
                error: Some(format!("Parse mcp.json failed: {err}")),
            }
        }
    };

    let servers = parsed
        .mcp_servers
        .into_iter()
        .map(|(name, server)| ChatMcpServer {
            id: format!("mcp-{}", uuid::Uuid::new_v4()),
            name,
            enabled: false,
            transport: normalize_imported_transport(&server),
            url: server.url,
            command: server.command,
            args: server.args,
            env: server.env,
            headers: server.headers,
            cwd: server.cwd,
            enabled_tools: Vec::new(),
        })
        .collect();

    McpImportResult {
        success: true,
        servers,
        error: None,
    }
}

/// 读取某个 MCP server 的持久连接状态快照（状态点 / handshake 次数 / stderr 尾巴）。
#[tauri::command]
pub async fn chat_mcp_server_status(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<crate::mcp::manager::McpServerStatusSnapshot, String> {
    Ok(state.mcp_server_state(&server_id).await)
}

/// 主动断开某个 MCP server 的持久会话（重连按钮）：下次调用透明重连。
#[tauri::command]
pub async fn chat_mcp_reload_server(
    app: AppHandle,
    state: State<'_, AppState>,
    server_id: String,
) -> Result<(), String> {
    state.mcp_reload_server(&app, &server_id).await;
    Ok(())
}

pub async fn call_tool(
    app: &AppHandle,
    state: &AppState,
    tool: &ChatToolDefinition,
    arguments: Value,
    skill_cache: Option<&mut crate::skills::SkillRunCache>,
    native_ctx: Option<NativeToolContext>,
) -> Result<McpToolCallResult, String> {
    if tool.source == "native" {
        return call_native_tool(app, state, tool, arguments, native_ctx).await;
    }

    if tool.source == "skill" {
        return call_skill_tool(app, state, tool, arguments, skill_cache).await;
    }

    if tool.source == "mixer" {
        return call_mixer_tool(state, tool, arguments).await;
    }

    let server_id = tool
        .server_id
        .as_deref()
        .ok_or_else(|| "MCP tool has no server id".to_string())?;
    let settings = state.settings_read().clone();
    let server = settings
        .chat_tools
        .servers
        .iter()
        .find(|server| server.id == server_id && server.enabled)
        .cloned()
        .ok_or_else(|| "MCP server is disabled or missing".to_string())?;
    // 走持久连接池：复用长连接、liveness 探活 + 透明重连、按 server_id 隔离。
    state.mcp_call_tool(app, &server, &tool.name, arguments).await
}

async fn list_server_tools(
    http: &reqwest::Client,
    server: &ChatMcpServer,
    timeout_ms: u64,
) -> Result<Vec<ChatToolDefinition>, String> {
    let tools = match server.transport.as_str() {
        "streamable_http" => {
            StreamableHttpMcpClient::new(server.clone(), timeout_ms, http.clone())
                .list_tools()
                .await?
        }
        _ => {
            StdioMcpClient::new(server.clone(), timeout_ms)
                .list_tools()
                .await?
        }
    };
    Ok(tools_from_mcp(server, tools))
}

fn normalize_imported_transport(server: &CursorMcpServer) -> String {
    let raw = server
        .transport
        .as_deref()
        .or(server.server_type.as_deref())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if raw == "http" || raw == "sse" || raw == "streamable_http" || !server.url.trim().is_empty() {
        "streamable_http".to_string()
    } else {
        "stdio".to_string()
    }
}

fn enabled_tools_cache_key(settings: &crate::settings::Settings) -> String {
    serde_json::to_string(&serde_json::json!({
        "chatTools": settings.chat_tools,
        "chatMemory": settings.chat_memory,
        "imageGeneration": settings.default_models.image_generation,
        "lensWebSearchProvider": settings.lens.web_search.provider,
        "lensWebSearchMaxResults": settings.lens.web_search.max_results,
        "lensWebSearchDepth": settings.lens.web_search.search_depth,
    }))
    .unwrap_or_else(|_| "chat-tools".to_string())
}

fn web_search_configured(settings: &crate::settings::Settings) -> bool {
    match settings.lens.web_search.provider {
        WebSearchProvider::Tavily => !settings.lens.web_search.tavily_api_key.trim().is_empty(),
        WebSearchProvider::Exa => !settings.lens.web_search.exa_api_key.trim().is_empty(),
    }
}

async fn call_mixer_tool(
    state: &AppState,
    tool: &ChatToolDefinition,
    arguments: Value,
) -> Result<McpToolCallResult, String> {
    match tool.name.as_str() {
        "mixer_generate_image" => {
            crate::chat::image_generation::tool_generate_image(state, &arguments).await
        }
        other => Err(format!("Unknown mixer tool: {other}")),
    }
}

async fn call_skill_tool(
    app: &AppHandle,
    state: &AppState,
    tool: &ChatToolDefinition,
    arguments: Value,
    skill_cache: Option<&mut crate::skills::SkillRunCache>,
) -> Result<McpToolCallResult, String> {
    let settings = state.settings_read().clone();
    let skill_name = crate::skills::extract_skill_name(&arguments)?;

    // Resolve the SkillRecord, preferring the run-scoped cached registry (T1).
    // Clone it out so we drop the immutable borrow on the cache before we need a
    // mutable borrow for activate/read dispatch and T3 allowed-tools recording.
    let mut skill_cache = skill_cache;
    let record = if let Some(cache) = skill_cache.as_deref_mut() {
        let registry = cache.registry_for(app, &settings.chat_tools.skill_scan_paths)?;
        crate::skills::lookup_skill(registry, &skill_name)
            .cloned()
            .ok_or_else(|| format!("Skill not found: {skill_name}"))?
    } else {
        let registry =
            crate::skills::build_registry(app, &settings.chat_tools.skill_scan_paths)?;
        crate::skills::lookup_skill(&registry, &skill_name)
            .cloned()
            .ok_or_else(|| format!("Skill not found: {skill_name}"))?
    };
    if !crate::settings::is_skill_enabled(&settings.chat_tools, &record.meta.id) {
        return Err(format!("Skill is disabled in Settings: {skill_name}"));
    }
    // 助手技能白名单硬 gate(防绕过):模型报个不在目录里的技能名也会被这里拦下。
    if let Some(cache) = skill_cache.as_deref() {
        if !cache.skill_id_allowed(&record.meta.id) {
            return Err(format!(
                "Skill is not allowed for the active assistant: {skill_name}"
            ));
        }
    }

    let content = match tool.name.as_str() {
        "skill_activate" => {
            if let Some(cache) = skill_cache.as_deref_mut() {
                // T3: a model-activated skill narrows the tool set on later rounds.
                cache.record_activated_allowed_tools(&record.allowed_tools);
                cache.activate_with_cache(&record)
            } else {
                crate::skills::activate_skill(&record)
            }
        }
        "skill_read_file" => {
            let relative_path = crate::skills::extract_relative_path(&arguments)?;
            if let Some(cache) = skill_cache.as_deref_mut() {
                cache.read_file_with_cache(&record, &relative_path)?
            } else {
                crate::skills::read_skill_file(&record, &relative_path)?
            }
        }
        "skill_run_script" => {
            let relative_path = crate::skills::extract_relative_path(&arguments)?;
            let args = crate::skills::extract_script_args(&arguments);
            let timeout_ms = effective_skill_script_timeout_ms(
                settings.chat_tools.tool_timeout_ms,
                arguments.get("timeout_ms").and_then(|value| value.as_u64()),
            );
            crate::skills::run_skill_script(
                &record,
                &relative_path,
                &args,
                timeout_ms,
                &settings.chat_tools.skill_script_allowlist,
            )
            .await?
        }
        other => return Err(format!("Unknown skill tool: {other}")),
    };

    Ok(McpToolCallResult {
        content,
        is_error: false,
        raw: Value::Null,
        artifacts: Vec::new(),
        structured_content: None,
    })
}

pub fn skill_runtime_tools_enabled(settings: &crate::settings::Settings) -> bool {
    settings.chat_tools.native_tools.skill_runtime
}

pub fn list_skill_tool_defs(settings: &crate::settings::Settings) -> Vec<ChatToolDefinition> {
    if skill_runtime_tools_enabled(settings) {
        native_skill_tools()
    } else {
        Vec::new()
    }
}

async fn call_native_tool(
    app: &AppHandle,
    state: &AppState,
    tool: &ChatToolDefinition,
    arguments: Value,
    native_ctx: Option<NativeToolContext>,
) -> Result<McpToolCallResult, String> {
    use super::native_registry::{find_entry, text_tool_result, NativeCallCtx, NativeToolCall};

    let Some(entry) = find_entry(&tool.name) else {
        return Err(format!("Unknown native tool: {}", tool.name));
    };

    // Conversation-scoped tools (todo) run before workspace resolution: they
    // only need the conversation id and must not fail when the conversation
    // project cannot be resolved.
    if let NativeToolCall::Conversation(handler) = &entry.call {
        let ctx = native_ctx
            .as_ref()
            .ok_or_else(|| format!("{} requires a conversation context", entry.name))?;
        return handler(app, &ctx.conversation_id, &tool.name, arguments);
    }
    if let NativeToolCall::SubAgent(handler) = &entry.call {
        // Sub-agent management tools manage agents, not files: dispatch before
        // workspace resolution with the parent run context.
        let ctx = native_ctx
            .as_ref()
            .ok_or_else(|| format!("{} requires an agent context", entry.name))?;
        return handler(crate::chat::sub_agent::SubAgentCallCtx {
            app,
            state,
            native_ctx: ctx,
            arguments: &arguments,
        })
        .await;
    }
    if matches!(entry.call, NativeToolCall::HostMediated) {
        // ask_user is host-mediated in chat/agent/execute.rs and must never
        // reach the registry dispatcher; keep the legacy fallback wording.
        return Err(format!("Unknown native tool: {}", tool.name));
    }

    let settings = state.settings_read().clone();
    let workspace = resolve_native_workspace(
        app,
        &settings.chat_tools.native_tools.workspace_roots,
        native_ctx.as_ref(),
    )?;

    match &entry.call {
        NativeToolCall::SyncText(call) => Ok(text_tool_result(call(&workspace, &arguments)?)),
        NativeToolCall::SyncResult(call) => call(&workspace, &arguments),
        NativeToolCall::BlockingText(call) => {
            let content = run_blocking_file_mutation(&workspace, &arguments, *call).await?;
            Ok(text_tool_result(content))
        }
        NativeToolCall::BlockingMutation(call) => {
            let result = run_blocking_file_mutation(&workspace, &arguments, *call).await?;
            file_mutation_tool_result(result)
        }
        NativeToolCall::Async(call) => {
            call(NativeCallCtx {
                app,
                state,
                settings: &settings,
                workspace: &workspace,
                arguments: &arguments,
                native_ctx: native_ctx.as_ref(),
            })
            .await
        }
        NativeToolCall::Conversation(_)
        | NativeToolCall::HostMediated
        | NativeToolCall::SubAgent(_) => unreachable!(),
    }
}

/// Runs a file mutation tool on the blocking thread pool so in-process path
/// lock waits (`Condvar::wait`) and large synchronous IO do not stall tokio
/// runtime workers.
async fn run_blocking_file_mutation<T, F>(
    workspace: &NativeToolWorkspace,
    arguments: &Value,
    mutate: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&NativeToolWorkspace, &Value) -> Result<T, String> + Send + 'static,
{
    let workspace = workspace.clone();
    let arguments = arguments.clone();
    tokio::task::spawn_blocking(move || mutate(&workspace, &arguments))
        .await
        .map_err(|err| format!("File mutation task failed: {err}"))?
}

/// 文件变更结果给模型的 diff 最多带多少行（对齐 clawspring 的 80 行裁剪）。
const FILE_MUTATION_DIFF_MAX_LINES: usize = 80;

pub fn file_mutation_tool_result(result: FileMutationResult) -> Result<McpToolCallResult, String> {
    let summary = result.summary();
    let mut content = summary;
    if !result.warnings.is_empty() {
        content = format!("{}\n{}", content, result.warnings.join("\n"));
    }
    // 把裁剪后的 unified diff 直接回显给模型：模型在结果里"看到"自己实际改了什么，
    // 能立即发现写歪。完整 diff 始终在 structured_content 里给前端渲染。
    if !result.diff.trim().is_empty() {
        let lines: Vec<&str> = result.diff.lines().collect();
        if lines.len() > FILE_MUTATION_DIFF_MAX_LINES {
            let clipped = lines[..FILE_MUTATION_DIFF_MAX_LINES].join("\n");
            content = format!(
                "{}\n\n{}\n[... diff clipped: showing first {FILE_MUTATION_DIFF_MAX_LINES} of {} lines ...]",
                content,
                clipped,
                lines.len()
            );
        } else {
            content = format!("{}\n\n{}", content, result.diff);
        }
    }
    let is_error = !result.ok;
    let structured = serde_json::to_value(&result)
        .map_err(|err| format!("Serialize file mutation result failed: {err}"))?;
    Ok(McpToolCallResult {
        content,
        is_error,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
    })
}

pub fn read_file_tool_result(result: ReadFileResult) -> Result<McpToolCallResult, String> {
    // structured_content 保留完整 ReadFileResult 给前端 ToolCallBlock 渲染（不变）。
    let structured = serde_json::to_value(&result)
        .map_err(|err| format!("Serialize read_file result failed: {err}"))?;
    // 模型看到的 content 改成 cat -n 风格（`行号\t内容` + 精简头），行号便于模型引用并构造
    // 后续 edit_file。行号仅供参考、不属于文件内容；read_file/edit_file 描述里已说明
    // old_string 不要带行号前缀。
    let content = format_read_file_for_model(&result);
    Ok(McpToolCallResult {
        content,
        is_error: false,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
    })
}

/// 把 ReadFileResult 渲染成模型友好的 `cat -n` 文本：一行精简元数据头 + `右对齐行号\t原文`。
fn format_read_file_for_model(result: &ReadFileResult) -> String {
    let mut out = format!(
        "{} — lines {}-{} of {}",
        result.path, result.start_line, result.end_line, result.total_lines
    );
    if result.truncated {
        match result.next_offset {
            Some(next) => out.push_str(&format!(" (truncated; continue with offset={next})")),
            None => out.push_str(" (truncated)"),
        }
    }
    for warning in &result.warnings {
        out.push_str("\n! ");
        out.push_str(warning);
    }
    if !result.content.is_empty() {
        let start = result.start_line.max(1);
        out.push('\n');
        let numbered: Vec<String> = result
            .content
            .lines()
            .enumerate()
            .map(|(i, line)| format!("{:>6}\t{}", start + i, line))
            .collect();
        out.push_str(&numbered.join("\n"));
    }
    out
}

fn resolve_native_workspace(
    app: &AppHandle,
    workspace_roots: &[String],
    native_ctx: Option<&NativeToolContext>,
) -> Result<NativeToolWorkspace, String> {
    let Some(native_ctx) = native_ctx else {
        return Ok(NativeToolWorkspace::global(workspace_roots));
    };
    let conversation = crate::chat::storage::load_conversation(app, &native_ctx.conversation_id)
        .map_err(|err| {
            format!(
                "Resolve native tool workspace failed for conversation {}: {err}",
                native_ctx.conversation_id
            )
        })?;
    let Some(project) = crate::chat::storage::resolve_conversation_project(app, &conversation)?
    else {
        return Ok(NativeToolWorkspace::global(workspace_roots));
    };
    Ok(NativeToolWorkspace::project(
        project.id,
        project.name,
        project.root_path,
    ))
}

pub(super) async fn run_python_via_pyodide(
    app: &AppHandle,
    state: &AppState,
    settings: &crate::settings::Settings,
    workspace: &NativeToolWorkspace,
    arguments: &Value,
    native_ctx: Option<NativeToolContext>,
) -> Result<McpToolCallResult, String> {
    let code = arguments
        .get("code")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "run_python requires code".to_string())?;

    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(|value| value.as_u64())
        .unwrap_or(settings.chat_tools.tool_timeout_ms)
        .clamp(1_000, 300_000);
    let input_files = collect_python_input_files(app, workspace, arguments)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let export_ctx = native_ctx
        .map(|ctx| crate::native_tools::SandboxExportContext {
            conversation_id: ctx.conversation_id,
            message_id: ctx.message_id,
            tool_call_id: ctx.tool_call_id,
        })
        .unwrap_or_else(|| crate::native_tools::SandboxExportContext {
            conversation_id: "standalone".to_string(),
            message_id: run_id.clone(),
            tool_call_id: None,
        });
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = state
            .pending_python_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.insert(
            run_id.clone(),
            crate::state::PendingPythonRun {
                sender: tx,
                export_ctx: export_ctx.clone(),
            },
        );
    }
    let emit_result = app.emit(
        "chat-run-python",
        serde_json::json!({
            "runId": run_id,
            "code": code,
            "timeoutMs": timeout_ms,
            "files": input_files,
        }),
    );
    if let Err(err) = emit_result {
        let mut pending = state
            .pending_python_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.remove(&run_id);
        return Err(format!("Failed to start Python runner: {err}"));
    }

    let wait =
        tokio::time::timeout(Duration::from_millis(timeout_ms.saturating_add(5_000)), rx).await;

    match wait {
        Ok(Ok(result)) => {
            if result.is_error {
                Err(result.content)
            } else {
                let mut content = result.content;
                let mut artifacts = result.artifacts;
                match crate::native_tools::export_sandbox_artifacts(&export_ctx, &artifacts)
                {
                    Ok(exported_artifacts) => {
                        for exported in &exported_artifacts {
                            if let Some(artifact) = artifacts.get_mut(exported.artifact_index) {
                                artifact.path = Some(exported.path.display().to_string());
                            }
                        }
                        let export_note =
                            crate::native_tools::format_exported_paths(&exported_artifacts);
                        if !export_note.is_empty() {
                            if !content.trim().is_empty() {
                                content.push_str("\n\n");
                            }
                            content.push_str(&export_note);
                        }
                    }
                    Err(err) => {
                        if !content.trim().is_empty() {
                            content.push_str("\n\n");
                        }
                        content.push_str(&crate::native_tools::format_export_error(&err));
                    }
                }
                Ok(McpToolCallResult {
                    content,
                    is_error: false,
                    raw: Value::Null,
                    artifacts,
                    structured_content: None,
                })
            }
        }
        Ok(Err(_)) => Err("Python runner channel closed".to_string()),
        Err(_) => {
            let mut pending = state
                .pending_python_runs
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&run_id);
            Err(format!("Python execution timed out after {timeout_ms}ms"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_tools::{ReadFileResult, ReadFileState};

    #[test]
    fn read_file_tool_result_preserves_structured_content() {
        let result = ReadFileResult {
            path: "src/App.tsx".to_string(),
            resolved_path: "/tmp/project/src/App.tsx".to_string(),
            content: "alpha\nbeta".to_string(),
            total_lines: 2,
            start_line: 1,
            end_line: 2,
            truncated: false,
            file_size: 10,
            next_offset: None,
            read_state: ReadFileState {
                scope: "full".to_string(),
                mtime: Some(123),
                already_read: false,
            },
            warnings: Vec::new(),
        };

        let output = read_file_tool_result(result).expect("tool result");
        let structured = output
            .structured_content
            .as_ref()
            .expect("structured content");

        assert!(!output.is_error);
        assert_eq!(output.raw, *structured);
        assert_eq!(structured["path"], "src/App.tsx");
        assert_eq!(structured["resolved_path"], "/tmp/project/src/App.tsx");
        assert_eq!(structured["content"], "alpha\nbeta");
        assert_eq!(structured["total_lines"], 2);
        assert_eq!(structured["start_line"], 1);
        assert_eq!(structured["end_line"], 2);
        assert_eq!(structured["truncated"], false);
        assert_eq!(structured["file_size"], 10);
        assert_eq!(structured["read_state"]["scope"], "full");
        // 模型看到的 content 是 cat -n 文本（不再是 JSON），结构化内容仍完整保留给前端。
        assert_eq!(
            output.content,
            "src/App.tsx — lines 1-2 of 2\n     1\talpha\n     2\tbeta"
        );
    }

    #[test]
    fn read_file_tool_result_numbers_from_offset_and_flags_truncation() {
        let result = ReadFileResult {
            path: "src/big.txt".to_string(),
            resolved_path: "/tmp/project/src/big.txt".to_string(),
            content: "line ten\nline eleven".to_string(),
            total_lines: 100,
            start_line: 10,
            end_line: 11,
            truncated: true,
            file_size: 4096,
            next_offset: Some(12),
            read_state: ReadFileState {
                scope: "partial".to_string(),
                mtime: Some(1),
                already_read: false,
            },
            warnings: Vec::new(),
        };
        let output = read_file_tool_result(result).expect("tool result");
        assert_eq!(
            output.content,
            "src/big.txt — lines 10-11 of 100 (truncated; continue with offset=12)\n    10\tline ten\n    11\tline eleven"
        );
    }
}
