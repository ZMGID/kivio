//! Multi-agent / sub-agent runtime (P3).
//!
//! A sub-agent is "a fresh isolated message history run through the same
//! `run_agent_loop`" — there is no second execution engine. `run_sub_agent`
//! builds an isolated `AgentRunConfig` (system + user only, a synthetic
//! `conversation_id` for generation/streaming isolation, but the PARENT
//! conversation as `tool_conversation_id` so the child's native file tools
//! resolve the parent's project workspace), wraps it in a `SubAgentHost`, and
//! reuses the existing loop. The `agent` native tool spawns one synchronously
//! and reports live nested progress onto the parent tool card.
//!
//! Orchestrator-worker model: a sub-agent is a PURE WORKER. It receives one
//! self-contained prompt, runs in isolation, and returns a result. It is given
//! NO todo tools and NO todo prompt, so it cannot read or mutate any todo list.
//! Task delegation is top-down: the parent conversation owns its todos and uses
//! its own todo tools to set `owner` (= sub-agent name) and status itself.
//!
//! Safety rails (research doc 05 + architecture P3):
//! - depth guard (`MAX_SUB_AGENT_DEPTH`): an agent at depth ≥ 3 cannot spawn.
//! - the `agent` tool is stripped from every sub-agent's tool table
//!   (`filter::filter_tools_for_agent`), a second guard against recursion.
//! - `Semaphore(3)` caps concurrent sub-agents (desktop API-quota sensitive).
//! - `SubAgentHost` auto-denies approval-gated (sensitive) tools at depth > 0
//!   and cascades parent cancellation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;

use crate::chat::agent::prepare::{available_builtin_tool_names, build_chat_system_prompt};
use crate::chat::agent::types::AgentRunResult;
use crate::chat::agent::{
    run_agent_loop, AgentHost, AgentHostFuture, AgentRunConfig, AgentRunEntry, ToolExecutionContext,
    ToolExecutor, ToolExecutorFuture,
};
use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
use crate::chat::types::{ChatMessageSegment, ToolCallRecord};
use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;
use crate::settings::{ModelProvider, Settings};
use crate::skills::SkillRegistry;
use crate::state::AppState;

/// An agent at depth ≥ this cannot spawn another sub-agent. Top-level chat is
/// depth 0; a spawned sub-agent runs at depth 1. With 3, a (hypothetical)
/// chain general→sub→sub→sub is the hard ceiling.
pub const MAX_SUB_AGENT_DEPTH: u8 = 3;
const SUB_AGENT_CONCURRENCY: usize = 3;
const SUB_AGENT_SYNC_TIMEOUT_SECS: u64 = 300;
/// Max attempts for a single sub-agent run. Reasoning models (e.g. DeepSeek-V4)
/// intermittently return an empty assistant message in the planning step, which
/// `run_agent_loop` surfaces as `Err`. Top-level chat recovers via user resend;
/// a sub-agent has no resend loop, so we retry the run once before giving up.
const SUB_AGENT_MAX_ATTEMPTS: usize = 2;
const PROGRESS_EMIT_INTERVAL_MS: u128 = 350;
const RESULT_PREVIEW_MAX: usize = 4000;

pub const AGENT_TOOL_NAME: &str = "agent";
pub const CHECK_AGENT_RESULT_TOOL_NAME: &str = "check_agent_result";
pub const LIST_AGENT_TASKS_TOOL_NAME: &str = "list_agent_tasks";

#[allow(dead_code)]
pub fn is_sub_agent_tool_name(name: &str) -> bool {
    matches!(
        name,
        AGENT_TOOL_NAME | CHECK_AGENT_RESULT_TOOL_NAME | LIST_AGENT_TASKS_TOOL_NAME
    )
}

/// Whether an agent at `depth` may spawn a sub-agent. The child runs at
/// `depth + 1`; an agent at depth ≥ `MAX_SUB_AGENT_DEPTH` is denied
/// (research doc 05 §1.3 / acceptance #2).
pub fn depth_allows_spawn(depth: u8) -> bool {
    depth < MAX_SUB_AGENT_DEPTH
}

/// Whether a failed sub-agent run should be retried. Retry only when the outcome
/// is an error that is NOT a cancellation, there are attempts remaining, and the
/// parent generation is still active (so we never retry after a cascade cancel).
fn should_retry_sub_agent(
    outcome: &Result<AgentRunResult, String>,
    attempt: usize,
    parent_active: bool,
) -> bool {
    matches!(outcome, Err(err) if err != "cancelled")
        && attempt + 1 < SUB_AGENT_MAX_ATTEMPTS
        && parent_active
}

// ---------------------------------------------------------------------------
// Task registry (lives on AppState)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTaskRecord {
    pub id: String,
    pub name: String,
    pub agent_type: String,
    pub status: SubAgentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub depth: u8,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    /// Provider token usage for this sub-agent's own run (input/output/total),
    /// surfaced on the parent tool card. None until the run completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SubAgentUsage>,
}

/// Compact token usage for a finished sub-agent run, derived from the run's
/// `AgentRunResult.usage` (the sub-agent's own provider usage, not overlapping
/// the parent conversation's). All fields optional: providers may omit any.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

impl SubAgentUsage {
    fn from_model_usage(usage: &crate::chat::model::ModelUsage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
        }
    }
}

/// Process-level sub-agent task table + concurrency gate. Held on `AppState`.
pub struct SubAgentManager {
    tasks: Mutex<HashMap<String, SubAgentTaskRecord>>,
    by_name: Mutex<HashMap<String, String>>,
    semaphore: Arc<Semaphore>,
}

impl Default for SubAgentManager {
    fn default() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            by_name: Mutex::new(HashMap::new()),
            semaphore: Arc::new(Semaphore::new(SUB_AGENT_CONCURRENCY)),
        }
    }
}

impl SubAgentManager {
    fn lock_tasks(&self) -> std::sync::MutexGuard<'_, HashMap<String, SubAgentTaskRecord>> {
        self.tasks.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn register(&self, record: SubAgentTaskRecord) {
        {
            let mut by_name = self.by_name.lock().unwrap_or_else(|e| e.into_inner());
            by_name.insert(record.name.clone(), record.id.clone());
        }
        self.lock_tasks().insert(record.id.clone(), record);
    }

    #[allow(dead_code)]
    pub fn set_status(&self, id: &str, status: SubAgentStatus) {
        if let Some(record) = self.lock_tasks().get_mut(id) {
            record.status = status;
        }
    }

    pub fn finish(
        &self,
        id: &str,
        status: SubAgentStatus,
        result: Option<String>,
        error: Option<String>,
        usage: Option<SubAgentUsage>,
    ) {
        if let Some(record) = self.lock_tasks().get_mut(id) {
            record.status = status;
            record.result = result;
            record.error = error;
            record.usage = usage;
            record.completed_at = Some(chrono::Local::now().timestamp());
        }
    }

    pub fn get(&self, id_or_name: &str) -> Option<SubAgentTaskRecord> {
        if let Some(record) = self.lock_tasks().get(id_or_name) {
            return Some(record.clone());
        }
        let resolved = self
            .by_name
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id_or_name)
            .cloned();
        resolved.and_then(|id| self.lock_tasks().get(&id).cloned())
    }

    pub fn list(&self) -> Vec<SubAgentTaskRecord> {
        let mut all: Vec<SubAgentTaskRecord> = self.lock_tasks().values().cloned().collect();
        all.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        all
    }

    fn semaphore(&self) -> Arc<Semaphore> {
        self.semaphore.clone()
    }
}

// ---------------------------------------------------------------------------
// Sub-agent host: forwards live progress, denies sensitive tools, cascades cancel
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ProgressState {
    text: String,
    last_emit: Option<Instant>,
    steps: Vec<String>,
}

struct SubAgentHost<'a> {
    app: AppHandle,
    state: &'a AppState,
    parent_conversation_id: String,
    parent_run_id: String,
    parent_tool_call_id: String,
    parent_generation: u64,
    task_id: String,
    name: String,
    depth: u8,
    progress: Mutex<ProgressState>,
}

/// Whether a sub-agent run is still active: BOTH its own generation and the
/// parent generation must be live. Parent cancel ⇒ cascade (acceptance #3).
fn generation_cascade_active(
    state: &AppState,
    conversation_id: &str,
    generation: u64,
    parent_conversation_id: &str,
    parent_generation: u64,
) -> bool {
    state.is_chat_generation_active(conversation_id, generation)
        && state.is_chat_generation_active(parent_conversation_id, parent_generation)
}

impl SubAgentHost<'_> {
    fn emit_progress(&self, status: &str, force: bool) {
        let (text, steps) = {
            let mut guard = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            if !force {
                if let Some(last) = guard.last_emit {
                    if now.duration_since(last).as_millis() < PROGRESS_EMIT_INTERVAL_MS {
                        return;
                    }
                }
            }
            guard.last_emit = Some(now);
            (clip(&guard.text, 1200), guard.steps.clone())
        };
        let _ = self.app.emit(
            "chat-subagent",
            serde_json::json!({
                "parentConversationId": self.parent_conversation_id,
                "parentRunId": self.parent_run_id,
                "parentToolCallId": self.parent_tool_call_id,
                "taskId": self.task_id,
                "name": self.name,
                "depth": self.depth,
                "status": status,
                "preview": text,
                "steps": steps,
            }),
        );
    }
}

impl AgentHost for SubAgentHost<'_> {
    fn emit_stream_delta(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        delta: &str,
        _reasoning_delta: Option<&str>,
        _segment: Option<&ChatMessageSegment>,
    ) {
        if !delta.is_empty() {
            let mut guard = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            guard.text.push_str(delta);
        }
        self.emit_progress("running", false);
    }

    fn emit_stream_done(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        _reason: &str,
        _full: &str,
    ) {
        // Final state is delivered as the parent tool record's structured
        // content by the spawn handler; nothing to do here.
    }

    fn emit_tool_record(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        record: &ToolCallRecord,
    ) {
        // Surface which tool the sub-agent is using as a nested step line.
        let label = format!("{} ({:?})", record.name, record.status);
        {
            let mut guard = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            if guard.steps.last().map(|s| s.as_str()) != Some(label.as_str()) {
                guard.steps.push(label);
                if guard.steps.len() > 40 {
                    let overflow = guard.steps.len() - 40;
                    guard.steps.drain(0..overflow);
                }
            }
        }
        self.emit_progress("running", true);
    }

    fn request_tool_approval<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        _record: &'a ToolCallRecord,
    ) -> AgentHostFuture<'a, bool> {
        // depth > 0: a sub-agent can never escalate to the user for approval,
        // so any approval-gated (sensitive) tool is auto-denied. Read-only /
        // bypass-approval tools never reach this method.
        Box::pin(async move { false })
    }

    fn request_user_response<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        _record: &'a ToolCallRecord,
        _prompt: AskUserPromptPayload,
    ) -> AgentHostFuture<'a, AskUserResponseResult> {
        // Sub-agents run autonomously; ask_user is filtered out of their tool
        // table, but if reached, resolve as a cancelled prompt.
        Box::pin(async move {
            AskUserResponseResult {
                phase: "cancelled".to_string(),
                answers: HashMap::new(),
            }
        })
    }

    fn is_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
        // Cascade: the sub-agent run is active only while BOTH its own
        // generation and the parent generation are live. Parent cancel ⇒
        // sub-agent stops on its next loop check.
        generation_cascade_active(
            self.state,
            conversation_id,
            generation,
            &self.parent_conversation_id,
            self.parent_generation,
        )
    }

    fn wait_for_generation_inactive<'a>(
        &'a self,
        conversation_id: &'a str,
        generation: u64,
    ) -> AgentHostFuture<'a, ()> {
        Box::pin(async move {
            loop {
                if !self.is_generation_active(conversation_id, generation) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tool executor for sub-agents (delegates to the MCP/native registry)
// ---------------------------------------------------------------------------

struct SubAgentToolExecutor<'a> {
    app: AppHandle,
    state: &'a AppState,
}

impl ToolExecutor for SubAgentToolExecutor<'_> {
    fn call<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
        skill_cache: Option<&'a mut crate::skills::SkillRunCache>,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            let native_ctx = crate::mcp::registry::NativeToolContext {
                conversation_id: ctx.tool_conversation_id.to_string(),
                message_id: ctx.message_id.to_string(),
                tool_call_id: Some(ctx.tool_call_id.to_string()),
                run_id: ctx.run_id.to_string(),
                generation: ctx.generation,
                depth: ctx.depth,
            };
            crate::mcp::registry::call_tool(
                &self.app,
                self.state,
                tool,
                arguments,
                skill_cache,
                Some(native_ctx),
            )
            .await
        })
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub struct SubAgentRequest {
    pub task_id: String,
    pub name: String,
    pub agent_type: String,
    pub prompt: String,
    pub system_prompt: String,
    pub provider: ModelProvider,
    pub model: String,
    pub tools: Vec<ChatToolDefinition>,
    pub settings: Settings,
    pub max_output_tokens: u32,
    pub language: String,
    pub depth: u8,
    pub parent_conversation_id: String,
    pub parent_run_id: String,
    pub parent_tool_call_id: String,
    pub parent_generation: u64,
}

/// Run a sub-agent to completion (synchronous spawn). Builds an isolated
/// config and reuses `run_agent_loop`. Cancellation cascades from the parent
/// via `SubAgentHost`. Up to `SUB_AGENT_MAX_ATTEMPTS` tries: reasoning models
/// occasionally return an empty planning response (surfaced as `Err`); since a
/// sub-agent has no user resend loop, we retry once on non-cancel errors.
async fn run_sub_agent(
    app: &AppHandle,
    state: &AppState,
    req: SubAgentRequest,
) -> Result<AgentRunResult, String> {
    let sub_conversation_id = format!("subagent-{}", req.task_id);

    let mut last_outcome: Result<AgentRunResult, String> =
        Err("Sub-agent did not run".to_string());

    for attempt in 0..SUB_AGENT_MAX_ATTEMPTS {
        // A cascade cancel between attempts must short-circuit: never retry once
        // the parent generation is gone.
        if attempt > 0
            && !state
                .is_chat_generation_active(&req.parent_conversation_id, req.parent_generation)
        {
            return Err("cancelled".to_string());
        }

        // Fresh generation + runtime per attempt. The config is moved into
        // `run_agent_loop`, so host/executor/config are rebuilt each iteration.
        let sub_generation = state.next_chat_generation(&sub_conversation_id);
        let sub_run_id = format!("subrun-{}", req.task_id);
        let sub_message_id = format!("submsg-{}", req.task_id);

        let runtime_messages = vec![
            serde_json::json!({ "role": "system", "content": req.system_prompt }),
            serde_json::json!({ "role": "user", "content": req.prompt }),
        ];

        let host = SubAgentHost {
            app: app.clone(),
            state,
            parent_conversation_id: req.parent_conversation_id.clone(),
            parent_run_id: req.parent_run_id.clone(),
            parent_tool_call_id: req.parent_tool_call_id.clone(),
            parent_generation: req.parent_generation,
            task_id: req.task_id.clone(),
            name: req.name.clone(),
            depth: req.depth,
            progress: Mutex::new(ProgressState::default()),
        };
        let executor = SubAgentToolExecutor {
            app: app.clone(),
            state,
        };

        let thinking_enabled = req.settings.chat.thinking_enabled;
        let stream_enabled = req.settings.chat.stream_enabled;
        let max_output_tokens = req.max_output_tokens;
        let retry_attempts = if req.settings.retry_enabled {
            req.settings.retry_attempts as usize
        } else {
            1
        };
        let effective_chat_tools = req.settings.chat_tools.clone();

        let config = AgentRunConfig {
            entry: AgentRunEntry::Send,
            state,
            conversation_id: sub_conversation_id.clone(),
            tool_conversation_id: req.parent_conversation_id.clone(),
            depth: req.depth,
            run_id: sub_run_id,
            message_id: sub_message_id,
            generation: sub_generation,
            provider: req.provider.clone(),
            model: req.model.clone(),
            runtime_messages,
            tools: req.tools.clone(),
            blocked_tool_calls: Vec::new(),
            settings: req.settings.clone(),
            effective_chat_tools,
            language: req.language.clone(),
            has_image: false,
            thinking_enabled,
            stream_enabled,
            max_output_tokens,
            retry_attempts,
            skill_registry: SkillRegistry::default(),
            active_skill_id: None,
            active_skill_detail: None,
            assistant_snapshot: None,
            custom_system_prompt: String::new(),
            provider_tools_fallback_system_prompt: req.system_prompt.clone(),
        };

        let timeout = Duration::from_secs(SUB_AGENT_SYNC_TIMEOUT_SECS);
        let outcome =
            match tokio::time::timeout(timeout, run_agent_loop(config, &host, &executor)).await {
                Ok(result) => result,
                Err(_) => Err(format!(
                    "Sub-agent timed out after {SUB_AGENT_SYNC_TIMEOUT_SECS}s"
                )),
            };
        // Retire this attempt's generation on every exit path (success, failure,
        // timeout). Otherwise a timeout leaves the synthetic generation reading
        // "active" forever, and entries accumulate in chat_stream_generations.
        state.cancel_chat_generation(&sub_conversation_id);

        // Success or cancellation → return immediately.
        if matches!(outcome, Ok(_)) || matches!(&outcome, Err(err) if err == "cancelled") {
            return outcome;
        }

        let parent_active =
            state.is_chat_generation_active(&req.parent_conversation_id, req.parent_generation);
        if !should_retry_sub_agent(&outcome, attempt, parent_active) {
            return outcome;
        }
        last_outcome = outcome;
    }

    last_outcome
}

// ---------------------------------------------------------------------------
// Native tool: agent / check_agent_result / list_agent_tasks
// ---------------------------------------------------------------------------

/// Context handed to sub-agent management tool handlers, dispatched before
/// workspace resolution (these tools manage agents, not files).
pub struct SubAgentCallCtx<'a> {
    pub app: &'a AppHandle,
    pub state: &'a AppState,
    pub native_ctx: &'a crate::mcp::registry::NativeToolContext,
    pub arguments: &'a Value,
}

pub fn agent_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__agent".to_string(),
        name: AGENT_TOOL_NAME.to_string(),
        description: "Spawn a sub-agent to handle a focused sub-task and return its result. The sub-agent runs with its own fresh context and a restricted toolset. Use for parallel decomposition or delegating self-contained research/implementation/review work. Provide a complete, self-contained prompt — the sub-agent cannot see this conversation.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Complete, self-contained task for the sub-agent (it has no access to this conversation)."
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Agent type: general-purpose (default), researcher, coder, reviewer, or a user/project-defined type."
                },
                "name": {
                    "type": "string",
                    "maxLength": 80,
                    "description": "Optional short label for this sub-agent run."
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: Some(serde_json::json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "openWorldHint": false
        })),
        output_schema: None,
    }
}

pub fn check_agent_result_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__check_agent_result".to_string(),
        name: CHECK_AGENT_RESULT_TOOL_NAME.to_string(),
        description: "Look up the status and result of a previously spawned sub-agent by its task id or name.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Sub-agent task id or name" }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: Some(serde_json::json!({ "readOnlyHint": true })),
        output_schema: None,
    }
}

pub fn list_agent_tasks_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__list_agent_tasks".to_string(),
        name: LIST_AGENT_TASKS_TOOL_NAME.to_string(),
        description: "List all sub-agent tasks spawned in this session with their status.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: Some(serde_json::json!({ "readOnlyHint": true })),
        output_schema: None,
    }
}

pub fn tool_definitions() -> Vec<ChatToolDefinition> {
    vec![agent_tool(), check_agent_result_tool(), list_agent_tasks_tool()]
}

/// Append sub-agent management tools (model-facing), skipping the `agent`
/// spawn tool when `allow_spawn` is false (i.e. inside a sub-agent — second
/// guard against recursion alongside the depth check).
pub fn append_tool_definitions(tools: &mut Vec<ChatToolDefinition>, allow_spawn: bool) {
    for tool in tool_definitions() {
        if tool.name == AGENT_TOOL_NAME && !allow_spawn {
            continue;
        }
        if !tools
            .iter()
            .any(|existing| existing.openai_tool_name() == tool.openai_tool_name())
        {
            tools.push(tool);
        }
    }
}

fn err_result(message: impl Into<String>) -> McpToolCallResult {
    McpToolCallResult {
        content: message.into(),
        is_error: true,
        raw: Value::Null,
        artifacts: Vec::new(),
        structured_content: None,
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}…")
}

pub fn handle_check_agent_result(ctx: SubAgentCallCtx<'_>) -> Result<McpToolCallResult, String> {
    let id = ctx
        .arguments
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "check_agent_result requires an id".to_string())?;
    let manager = &ctx.state.sub_agents;
    match manager.get(id) {
        Some(record) => {
            let structured = serde_json::to_value(&record).unwrap_or(Value::Null);
            let body = record
                .result
                .clone()
                .or_else(|| record.error.clone())
                .unwrap_or_else(|| "(no result yet)".to_string());
            Ok(McpToolCallResult {
                content: format!(
                    "Sub-agent {} [{}]: {:?}\n\n{}",
                    record.name, record.id, record.status, body
                ),
                is_error: false,
                raw: structured.clone(),
                artifacts: Vec::new(),
                structured_content: Some(structured),
            })
        }
        None => Ok(err_result(format!("No sub-agent task found for '{id}'"))),
    }
}

pub fn handle_list_agent_tasks(ctx: SubAgentCallCtx<'_>) -> Result<McpToolCallResult, String> {
    let tasks = ctx.state.sub_agents.list();
    let structured = serde_json::json!({ "tasks": tasks });
    let lines = if tasks.is_empty() {
        "(no sub-agent tasks)".to_string()
    } else {
        tasks
            .iter()
            .map(|t| format!("- {} [{}] {:?}", t.name, t.id, t.status))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(McpToolCallResult {
        content: format!("Sub-agent tasks:\n{lines}"),
        is_error: false,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
    })
}

/// Spawn handler (the `agent` tool). Async (Box::pin) so it can drive
/// `run_sub_agent` to completion synchronously.
pub fn handle_agent_spawn<'a>(
    ctx: SubAgentCallCtx<'a>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<McpToolCallResult, String>> + Send + 'a>>
{
    Box::pin(async move {
        // Depth guard (research doc 05 §1.3 / acceptance #2): an agent at depth
        // >= MAX cannot spawn. Soft failure (Ok with is_error) so the parent
        // loop continues.
        if !depth_allows_spawn(ctx.native_ctx.depth) {
            return Ok(err_result(format!(
                "Cannot spawn a sub-agent: max nesting depth {MAX_SUB_AGENT_DEPTH} reached."
            )));
        }

        let prompt = ctx
            .arguments
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "agent requires a non-empty prompt".to_string())?
            .to_string();
        let agent_type = ctx
            .arguments
            .get("subagent_type")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("general-purpose")
            .to_string();

        let parent_conversation_id = ctx.native_ctx.conversation_id.clone();
        let parent_conversation =
            crate::chat::storage::load_conversation(ctx.app, &parent_conversation_id)?;
        let settings = ctx.state.settings_read().clone();
        let language = crate::settings::resolve_chat_language(&settings);

        // Resolve agent definition (built-in + user + project layers).
        let project_root = crate::chat::storage::resolve_conversation_project(
            ctx.app,
            &parent_conversation,
        )
        .ok()
        .flatten()
        .and_then(|p| p.root_path)
        .map(std::path::PathBuf::from);
        let defs = crate::agents::load_agent_definitions(ctx.app, project_root.as_deref());
        let Some(def) = crate::agents::find_definition(&defs, &agent_type) else {
            return Ok(err_result(format!(
                "Unknown sub-agent type '{agent_type}'. Available: {}",
                defs.iter()
                    .map(|d| d.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        let def = def.clone();

        // Provider/model inherited from the parent conversation unless the
        // agent definition overrides the model.
        let provider = settings
            .get_provider(&parent_conversation.provider_id)
            .cloned()
            .ok_or_else(|| "Parent chat provider not found".to_string())?;
        if provider.api_keys.is_empty() {
            return Ok(err_result(
                "Parent chat provider has no API key configured.".to_string(),
            ));
        }
        let model = def
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| parent_conversation.model.clone());
        if model.trim().is_empty() {
            return Ok(err_result("No model available for the sub-agent.".to_string()));
        }

        // Build the sub-agent's toolset: full enabled set, narrowed by the
        // agent definition, with the `agent` tool ALWAYS stripped (acceptance
        // #4). A sub-agent is a pure worker (orchestrator-worker model): it gets
        // NO todo tools, so it can never read or mutate any todo list. Task
        // delegation is top-down — the parent orchestrator owns the todos and
        // marks them itself (owner = sub-agent name) before/after the spawn.
        let mut tools = crate::mcp::registry::list_enabled_tool_defs(ctx.app, ctx.state)
            .await
            .unwrap_or_default();
        crate::chat::agent::filter::filter_tools_for_agent(&mut tools, &def);
        let available_builtin_tools = available_builtin_tool_names(&tools);

        // Compose the sub-agent system prompt: persona prefix + base chat
        // system prompt. No todo context is injected — the worker is not aware
        // of and cannot touch the parent's todo list.
        let system_prompt = build_chat_system_prompt(
            &language,
            false,
            settings.chat.thinking_enabled,
            &SkillRegistry::default(),
            &settings.chat_tools,
            true,
            &available_builtin_tools,
            None,
            None,
            None,
            &compose_persona(&def.system_prompt),
            None,
            None,
            None,
            None,
            None,
        );

        let task_id = format!("agent-{}", uuid::Uuid::new_v4().simple());
        let name = ctx
            .arguments
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| def.name.clone());

        let manager = &ctx.state.sub_agents;
        manager.register(SubAgentTaskRecord {
            id: task_id.clone(),
            name: name.clone(),
            agent_type: def.name.clone(),
            status: SubAgentStatus::Running,
            result: None,
            error: None,
            depth: ctx.native_ctx.depth + 1,
            created_at: chrono::Local::now().timestamp(),
            completed_at: None,
            usage: None,
        });

        // Concurrency gate.
        // Concurrency gate: held for the lifetime of the run. acquire_owned only
        // errors if the semaphore is closed (never — it lives on AppState).
        let _permit = manager.semaphore().acquire_owned().await.ok();

        // Model-aware output cap: prefer the model library / provider override
        // (matching top-level chat); the raw setting is only a fallback.
        let max_output_tokens = crate::chat::model_metadata::chat_max_output_tokens_for_model(
            Some(&provider),
            &model,
            settings.chat.max_output_tokens,
        );

        let request = SubAgentRequest {
            task_id: task_id.clone(),
            name: name.clone(),
            agent_type: def.name.clone(),
            prompt,
            system_prompt,
            provider,
            model,
            tools,
            settings,
            max_output_tokens,
            language,
            depth: ctx.native_ctx.depth + 1,
            parent_conversation_id: parent_conversation_id.clone(),
            parent_run_id: ctx.native_ctx.run_id.clone(),
            parent_tool_call_id: ctx
                .native_ctx
                .tool_call_id
                .clone()
                .unwrap_or_default(),
            parent_generation: ctx.native_ctx.generation,
        };

        let outcome = run_sub_agent(ctx.app, ctx.state, request).await;

        match outcome {
            Ok(result) => {
                let content = if result.content.trim().is_empty() {
                    "(sub-agent produced no text output)".to_string()
                } else {
                    result.content.clone()
                };
                let usage = result.usage.as_ref().map(SubAgentUsage::from_model_usage);
                manager.finish(
                    &task_id,
                    SubAgentStatus::Completed,
                    Some(clip(&content, RESULT_PREVIEW_MAX)),
                    None,
                    usage.clone(),
                );
                let mut structured = serde_json::json!({
                    "type": "subagent",
                    "taskId": task_id,
                    "name": name,
                    "agentType": def.name,
                    "status": "completed",
                    "result": clip(&content, RESULT_PREVIEW_MAX),
                });
                if let Some(usage) = usage {
                    if let Some(obj) = structured.as_object_mut() {
                        obj.insert(
                            "usage".to_string(),
                            serde_json::to_value(&usage).unwrap_or(Value::Null),
                        );
                    }
                }
                Ok(McpToolCallResult {
                    content: format!("[Sub-agent: {} ({})]\n\n{}", name, def.name, content),
                    is_error: false,
                    raw: structured.clone(),
                    artifacts: Vec::new(),
                    structured_content: Some(structured),
                })
            }
            Err(err) => {
                let cancelled = err == "cancelled";
                let status = if cancelled {
                    SubAgentStatus::Cancelled
                } else {
                    SubAgentStatus::Failed
                };
                // Surface a clean, user/model-facing message instead of the raw
                // internal error string. The empty-assistant-response case (a
                // reasoning model returning nothing in planning) is the common
                // failure; other errors keep their original text.
                let display_err = if err.contains("empty assistant response") {
                    "子 agent 运行失败：模型返回了空响应（可重试）。".to_string()
                } else {
                    err.clone()
                };
                manager.finish(&task_id, status, None, Some(display_err.clone()), None);
                let structured = serde_json::json!({
                    "type": "subagent",
                    "taskId": task_id,
                    "name": name,
                    "agentType": def.name,
                    "status": if cancelled { "cancelled" } else { "failed" },
                    "error": display_err,
                });
                Ok(McpToolCallResult {
                    content: format!("[Sub-agent: {} ({})] failed: {}", name, def.name, display_err),
                    is_error: !cancelled,
                    raw: structured.clone(),
                    artifacts: Vec::new(),
                    structured_content: Some(structured),
                })
            }
        }
    })
}

// Registry dispatch entry points (all return NativeToolFuture so the static
// `NativeToolCall::SubAgent` variant can hold a single fn-pointer shape).

/// `agent` spawn — already async-shaped.
pub fn dispatch_agent_spawn(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    handle_agent_spawn(ctx)
}

pub fn dispatch_check_agent_result(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move { handle_check_agent_result(ctx) })
}

pub fn dispatch_list_agent_tasks(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move { handle_list_agent_tasks(ctx) })
}

fn compose_persona(persona: &str) -> String {
    let persona = persona.trim();
    if persona.is_empty() {
        "You are a sub-agent spawned to complete a focused task autonomously. Use the available tools, then return a clear, complete final answer. You cannot ask the user questions.".to_string()
    } else {
        format!(
            "{persona}\n\nYou are running as a sub-agent: work autonomously with the available tools and return a clear, complete final answer. You cannot ask the user questions."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_agent_tool_name_detection() {
        assert!(is_sub_agent_tool_name("agent"));
        assert!(is_sub_agent_tool_name("check_agent_result"));
        assert!(is_sub_agent_tool_name("list_agent_tasks"));
        assert!(!is_sub_agent_tool_name("read_file"));
    }

    #[test]
    fn append_tools_strips_spawn_when_not_allowed() {
        let mut tools = Vec::new();
        append_tool_definitions(&mut tools, false);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"agent"), "spawn tool must be hidden in sub-agents");
        assert!(names.contains(&"check_agent_result"));
        assert!(names.contains(&"list_agent_tasks"));
    }

    #[test]
    fn append_tools_includes_spawn_when_allowed() {
        let mut tools = Vec::new();
        append_tool_definitions(&mut tools, true);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"agent"));
    }

    #[test]
    fn manager_register_and_lookup_by_id_and_name() {
        let manager = SubAgentManager::default();
        manager.register(SubAgentTaskRecord {
            id: "agent-1".to_string(),
            name: "researcher".to_string(),
            agent_type: "researcher".to_string(),
            status: SubAgentStatus::Running,
            result: None,
            error: None,
            depth: 1,
            created_at: 100,
            completed_at: None,
            usage: None,
        });
        assert_eq!(manager.get("agent-1").unwrap().name, "researcher");
        assert_eq!(manager.get("researcher").unwrap().id, "agent-1");
        manager.finish(
            "agent-1",
            SubAgentStatus::Completed,
            Some("done".into()),
            None,
            None,
        );
        let rec = manager.get("agent-1").unwrap();
        assert_eq!(rec.status, SubAgentStatus::Completed);
        assert_eq!(rec.result.as_deref(), Some("done"));
        assert_eq!(manager.list().len(), 1);
    }

    #[test]
    fn depth_guard_rejects_at_max_depth() {
        // Acceptance #2: depth >= 3 cannot spawn.
        assert!(depth_allows_spawn(0));
        assert!(depth_allows_spawn(1));
        assert!(depth_allows_spawn(2));
        assert!(!depth_allows_spawn(3));
        assert!(!depth_allows_spawn(4));
        assert_eq!(MAX_SUB_AGENT_DEPTH, 3);
    }

    #[test]
    fn host_cancels_when_parent_generation_cancelled() {
        // Acceptance #3: parent cancel cascades to the sub-agent.
        let state = crate::state::test_app_state();
        let parent_gen = state.next_chat_generation("conv-parent");
        let sub_gen = state.next_chat_generation("subagent-x");
        // Both live → active.
        assert!(generation_cascade_active(
            &state,
            "subagent-x",
            sub_gen,
            "conv-parent",
            parent_gen
        ));
        // Cancel the PARENT → sub-agent must report inactive (cascade).
        state.cancel_chat_generation("conv-parent");
        assert!(!generation_cascade_active(
            &state,
            "subagent-x",
            sub_gen,
            "conv-parent",
            parent_gen
        ));
    }

    fn ok_run_result() -> Result<AgentRunResult, String> {
        Ok(AgentRunResult {
            content: "done".to_string(),
            reasoning: None,
            tool_records: Vec::new(),
            segments: Vec::new(),
            api_messages: Vec::new(),
            steps: Vec::new(),
            stream_outcome: String::new(),
            usage: None,
        })
    }

    #[test]
    fn retry_only_on_recoverable_error_with_attempts_and_active_parent() {
        // SUB_AGENT_MAX_ATTEMPTS == 2: attempt 0 may retry, attempt 1 may not.
        assert_eq!(SUB_AGENT_MAX_ATTEMPTS, 2);

        let recoverable: Result<AgentRunResult, String> =
            Err("Chat tools planning returned an empty assistant response".to_string());

        // First attempt, parent alive, recoverable error → retry.
        assert!(should_retry_sub_agent(&recoverable, 0, true));
        // Last attempt → no retry even though recoverable.
        assert!(!should_retry_sub_agent(&recoverable, 1, true));
        // Parent cancelled → never retry (cascade).
        assert!(!should_retry_sub_agent(&recoverable, 0, false));

        // Cancellation is not a recoverable error → never retry.
        let cancelled: Result<AgentRunResult, String> = Err("cancelled".to_string());
        assert!(!should_retry_sub_agent(&cancelled, 0, true));

        // Success → never retry.
        assert!(!should_retry_sub_agent(&ok_run_result(), 0, true));
    }
}
