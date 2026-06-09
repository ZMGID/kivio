use std::{collections::HashMap, fs, path::Path, time::Duration};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::oneshot;

use crate::{
    native_tools::{resolve_tool_read_path, NativeToolWorkspace},
    settings::{
        ChatMcpServer, WebSearchProvider, CHAT_TOOL_MAX_TIMEOUT_MS, CHAT_TOOL_MIN_TIMEOUT_MS,
        SKILL_SCRIPT_MIN_TIMEOUT_MS,
    },
    state::AppState,
    web_search,
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
pub async fn chat_mcp_list_tools(state: State<'_, AppState>) -> Result<McpListToolsResult, String> {
    Ok(match list_enabled_tool_defs(&state).await {
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

pub async fn list_enabled_tool_defs(state: &AppState) -> Result<Vec<ChatToolDefinition>, String> {
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
        for server in settings
            .chat_tools
            .servers
            .iter()
            .filter(|server| server.enabled)
        {
            match list_server_tools(server, settings.chat_tools.tool_timeout_ms).await {
                Ok(mut server_tools) => tools.append(&mut server_tools),
                Err(err) => {
                    eprintln!(
                        "MCP server {} failed while listing tools: {err}",
                        server.name
                    );
                }
            }
        }
    }

    tools.extend(list_skill_tool_defs(&settings));

    state.set_cached_chat_tools(cache_key, tools.clone());
    Ok(tools)
}

#[tauri::command]
pub async fn chat_mcp_test_server(server: ChatMcpServer, timeout_ms: Option<u64>) -> McpTestResult {
    match list_server_tools(&server, timeout_ms.unwrap_or(60_000)).await {
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
    }
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
    match server.transport.as_str() {
        "streamable_http" => {
            StreamableHttpMcpClient::new(
                server,
                settings.chat_tools.tool_timeout_ms,
                state.http.clone(),
            )
            .call_tool(&tool.name, arguments)
            .await
        }
        _ => {
            StdioMcpClient::new(server, settings.chat_tools.tool_timeout_ms)
                .call_tool(&tool.name, arguments)
                .await
        }
    }
}

async fn list_server_tools(
    server: &ChatMcpServer,
    timeout_ms: u64,
) -> Result<Vec<ChatToolDefinition>, String> {
    if !server.enabled && !server.command.trim().is_empty() {
        // Test connection passes in disabled draft configs; listing enabled tools filters elsewhere.
    }
    let tools = match server.transport.as_str() {
        "streamable_http" => {
            StreamableHttpMcpClient::new(server.clone(), timeout_ms, reqwest::Client::new())
                .list_tools()
                .await?
        }
        _ => {
            StdioMcpClient::new(server.clone(), timeout_ms)
                .list_tools()
                .await?
        }
    };
    let allowed = server
        .enabled_tools
        .iter()
        .map(|tool| tool.as_str())
        .collect::<Vec<_>>();
    Ok(tools
        .into_iter()
        .filter(|tool| allowed.is_empty() || allowed.contains(&tool.name.as_str()))
        .map(|tool| tool_definition_from_mcp(server, tool))
        .collect())
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
    let registry = crate::skills::build_registry(app, &settings.chat_tools.skill_scan_paths)?;
    let skill_name = crate::skills::extract_skill_name(&arguments)?;
    let record = crate::skills::lookup_skill(&registry, &skill_name)
        .ok_or_else(|| format!("Skill not found: {skill_name}"))?;
    if !crate::settings::is_skill_enabled(&settings.chat_tools, &record.meta.id) {
        return Err(format!("Skill is disabled in Settings: {skill_name}"));
    }

    let content = match tool.name.as_str() {
        "skill_activate" => {
            if let Some(cache) = skill_cache {
                cache.activate_with_cache(record)
            } else {
                crate::skills::activate_skill(record)
            }
        }
        "skill_read_file" => {
            let relative_path = crate::skills::extract_relative_path(&arguments)?;
            if let Some(cache) = skill_cache {
                cache.read_file_with_cache(record, &relative_path)?
            } else {
                crate::skills::read_skill_file(record, &relative_path)?
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
                record,
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
    let settings = state.settings_read().clone();
    let workspace = resolve_native_workspace(
        app,
        &settings.chat_tools.native_tools.workspace_roots,
        native_ctx.as_ref(),
    )?;

    if tool.name == "run_python" {
        return run_python_via_pyodide(app, state, &settings, &workspace, &arguments, native_ctx)
            .await;
    }

    let content = match tool.name.as_str() {
        "web_search" => {
            let query = arguments
                .get("query")
                .and_then(|query| query.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if query.is_empty() {
                return Err("web_search query is empty".to_string());
            }
            let retry_attempts = if settings.retry_enabled {
                settings.retry_attempts as usize
            } else {
                1
            };
            let results =
                web_search::search_web(state, &settings.lens.web_search, &query, retry_attempts)
                    .await?;
            let raw = serde_json::to_value(&results).unwrap_or(Value::Null);
            return Ok(McpToolCallResult {
                content: web_search::format_web_context(&results),
                is_error: false,
                raw,
                artifacts: Vec::new(),
                structured_content: None,
            });
        }
        "web_fetch" => crate::native_tools::web_fetch(&state.http, &arguments).await?,
        "memory_read" => {
            if !settings.chat_memory.enabled {
                return Err("Chat memory is disabled in Settings".to_string());
            }
            return crate::chat::memory::tool_read(app, &arguments);
        }
        "memory_modify" => {
            if !settings.chat_memory.enabled {
                return Err("Chat memory is disabled in Settings".to_string());
            }
            return crate::chat::memory::tool_modify(app, &arguments);
        }
        "read_file" => crate::native_tools::read_file(&workspace, &arguments)?,
        "list_dir" => crate::native_tools::list_dir(&workspace, &arguments)?,
        "search_files" => crate::native_tools::search_files(&workspace, &arguments)?,
        "glob_files" => crate::native_tools::glob_files(&workspace, &arguments)?,
        "stat_path" => crate::native_tools::stat_path(&workspace, &arguments)?,
        "write_file" => crate::native_tools::write_file(&workspace, &arguments)?,
        "edit_file" => crate::native_tools::edit_file(&workspace, &arguments)?,
        "create_dir" => crate::native_tools::create_dir(&workspace, &arguments)?,
        "delete_path" => crate::native_tools::delete_path(&workspace, &arguments)?,
        "move_path" => crate::native_tools::move_path(&workspace, &arguments)?,
        "copy_path" => crate::native_tools::copy_path(&workspace, &arguments)?,
        "run_command" => {
            crate::native_tools::run_command(
                &workspace,
                settings.chat_tools.tool_timeout_ms,
                &arguments,
            )
            .await?
        }
        other => return Err(format!("Unknown native tool: {other}")),
    };

    Ok(McpToolCallResult {
        content,
        is_error: false,
        raw: Value::Null,
        artifacts: Vec::new(),
        structured_content: None,
    })
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

async fn run_python_via_pyodide(
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
                match crate::native_tools::export_sandbox_artifacts(&export_ctx, &result.artifacts)
                {
                    Ok(exported_paths) => {
                        let export_note =
                            crate::native_tools::format_exported_paths(&exported_paths);
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
                    artifacts: result.artifacts,
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
