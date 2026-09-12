//! Built-in child collaboration: durable identities, asynchronous executions,
//! explicit messages/continuation and independent cancellation. All model and
//! tool work uses the shared Agent loop; control/storage live in child modules.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::agents::AgentDefinition;
use crate::chat::agent::prepare::{available_builtin_tool_names, build_chat_system_prompt};
use crate::chat::agent::types::AgentRunResult;
use crate::chat::agent::{
    run_agent_loop, AgentHost, AgentHostFuture, AgentRunConfig, ToolExecutionContext, ToolExecutor,
    ToolExecutorFuture,
};
use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
use crate::chat::types::{ChatMessageSegment, ToolCallRecord, ToolCallStatus};
use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;
use crate::settings::{ModelProvider, Settings};
use crate::skills::SkillRegistry;

pub mod control;
pub mod runtime;
use crate::state::AppState;

/// An agent at depth ≥ this cannot spawn another sub-agent. Top-level chat is
/// depth 0; a spawned sub-agent runs at depth 1. NOTE: this is a
/// defense-in-depth backstop, not the real limiter — `filter_tools_for_agent`
/// always strips the `agent` tool from sub-agents, so actual nesting is one
/// level and depth ≥ 2 is unreachable unless that strip regresses.
pub const MAX_SUB_AGENT_DEPTH: u8 = 1;
/// Default concurrent sub-agent cap. User-overridable (live) via
/// `settings.chat_tools.sub_agent_concurrency`, clamped to
/// `[SUB_AGENT_CONCURRENCY_MIN, SUB_AGENT_CONCURRENCY_MAX]`.
pub const DEFAULT_SUB_AGENT_CONCURRENCY: usize = 12;
pub const SUB_AGENT_CONCURRENCY_MIN: usize = 1;
pub const SUB_AGENT_CONCURRENCY_MAX: usize = 64;
const PROGRESS_EMIT_INTERVAL_MS: u128 = 350;

pub const AGENT_TOOL_NAME: &str = "agent";

pub fn is_sub_agent_tool_name(name: &str) -> bool {
    matches!(name, AGENT_TOOL_NAME | "agent_control")
}

/// Whether an agent at `depth` may spawn a sub-agent. The child runs at
/// `depth + 1`; an agent at depth ≥ `MAX_SUB_AGENT_DEPTH` is denied
/// (research doc 05 §1.3 / acceptance #2).
pub fn depth_allows_spawn(depth: u8) -> bool {
    depth < MAX_SUB_AGENT_DEPTH
}

/// Process-owned durable runtime, initialized at the application's data path.
#[derive(Default)]
pub struct SubAgentManager {
    durable: Mutex<Option<Arc<runtime::Runtime>>>,
}
impl SubAgentManager {
    pub fn set_concurrency(&self, limit: usize) {
        if let Some(runtime) = self
            .durable
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            runtime.set_limit(limit);
        }
    }
    pub fn stop_conversation(&self, conversation: &str) {
        if let Some(runtime) = self
            .durable
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if let Err(error) = runtime.stop_parent(conversation, None) {
                eprintln!("Cannot save child cancellation: {error}");
            }
        }
    }
}

#[derive(Default)]
struct ProgressState {
    text: String,
    last_emit: Option<Instant>,
    /// Per-tool-call tracking, ordered by first sight, keyed by `record.id`.
    /// Status is updated in place (Pending→Running→Success/Error) instead of
    /// appending an event line per transition, so the same call never produces
    /// multiple step rows.
    tools: Vec<ToolProgress>,
}

/// One tracked tool call inside a sub-agent run.
struct ToolProgress {
    id: String,
    name: String,
    status: ToolCallStatus,
}

impl ProgressState {
    /// Insert or update a tool call by `id`. Existing call ⇒ refresh its status;
    /// new call ⇒ append (preserving first-seen order).
    fn upsert_tool(&mut self, id: &str, name: &str, status: ToolCallStatus) {
        if let Some(existing) = self.tools.iter_mut().find(|t| t.id == id) {
            existing.status = status;
        } else {
            self.tools.push(ToolProgress {
                id: id.to_string(),
                name: name.to_string(),
                status,
            });
        }
    }

    /// Aggregate tracked tool calls into a compact per-tool-name summary, one
    /// line per distinct tool name with status counts, e.g.
    /// `web_search · 6 done · 2 running`. Zero-count states are omitted.
    fn aggregate_steps(&self) -> Vec<String> {
        // Preserve first-seen order of tool names.
        let mut order: Vec<&str> = Vec::new();
        let mut counts: HashMap<&str, [usize; 3]> = HashMap::new(); // [done, running, failed]
        for tool in &self.tools {
            let entry = counts.entry(tool.name.as_str()).or_insert_with(|| {
                order.push(tool.name.as_str());
                [0, 0, 0]
            });
            match tool.status {
                ToolCallStatus::Success | ToolCallStatus::Skipped => entry[0] += 1,
                ToolCallStatus::Pending | ToolCallStatus::Running => entry[1] += 1,
                ToolCallStatus::Error | ToolCallStatus::Cancelled => entry[2] += 1,
            }
        }
        order
            .into_iter()
            .map(|name| {
                let [done, running, failed] = counts[name];
                let mut line = name.to_string();
                if done > 0 {
                    line.push_str(&format!(" · {done} done"));
                }
                if running > 0 {
                    line.push_str(&format!(" · {running} running"));
                }
                if failed > 0 {
                    line.push_str(&format!(" · {failed} failed"));
                }
                line
            })
            .collect()
    }
}

struct SubAgentHost {
    managed: Option<(Arc<runtime::Runtime>, String)>,
    workflow_hooks: crate::chat::workflow_hooks::Runtime,
    app: AppHandle,
    parent_conversation_id: String,
    parent_run_id: String,
    parent_tool_call_id: String,
    parent_generation: u64,
    task_id: String,
    name: String,
    model: String,
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

impl SubAgentHost {
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
            (clip(&guard.text, 1200), guard.aggregate_steps())
        };
        if let Some((runtime, run)) = &self.managed {
            if let Err(error) = runtime.progress(
                &self.parent_conversation_id,
                &self.task_id,
                run,
                text,
                steps,
            ) {
                eprintln!("Cannot save child progress: {error}");
            }
            return;
        }
        crate::chat::protocol::emit_run_event(
            &self.app,
            &self.parent_run_id,
            crate::chat::protocol::ChatRunEvent::SubagentUpdated {
                parent_tool_call_id: self.parent_tool_call_id.clone(),
                task_id: self.task_id.clone(),
                name: self.name.clone(),
                model: Some(self.model.clone()),
                depth: self.depth,
                status: status.to_string(),
                preview: Some(text),
                steps,
            },
        );
    }
}

impl AgentHost for SubAgentHost {
    fn requires_tool_completion(&self) -> bool {
        self.managed.is_some()
    }
    fn close_runtime_input(&self) -> Result<(), String> {
        if let Some((runtime, run)) = &self.managed {
            runtime.close_input(&self.parent_conversation_id, &self.task_id, run)?;
        }
        Ok(())
    }
    fn checkpoint_runtime<'a>(
        &'a self,
        _conversation_id: &'a str,
        _run_id: &'a str,
        history: &'a [Value],
        finishing: bool,
    ) -> AgentHostFuture<'a, Result<Vec<Value>, String>> {
        Box::pin(async move {
            match &self.managed {
                Some((runtime, run)) => runtime.checkpoint(
                    &self.parent_conversation_id,
                    &self.task_id,
                    run,
                    history,
                    finishing,
                ),
                None => Ok(Vec::new()),
            }
        })
    }
    fn workflow_hooks(&self) -> Option<&crate::chat::workflow_hooks::Runtime> {
        Some(&self.workflow_hooks)
    }
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

    fn emit_tool_record(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        record: &ToolCallRecord,
    ) {
        // Surface which tools the sub-agent is using as compact nested step
        // lines. Track each call by id and update its status in place, so a
        // single call (Pending→Running→Success) never spams multiple rows;
        // `emit_progress` aggregates them into per-tool-name count lines.
        {
            let mut guard = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            guard.upsert_tool(&record.id, &record.name, record.status.clone());
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

    fn request_session_consent<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
    ) -> AgentHostFuture<'a, bool> {
        // A sub-agent cannot prompt the user, but it inherits the parent
        // conversation's session consent: if the user already authorized
        // file/shell tools for this conversation, the sub-agent reuses that
        // grant. Otherwise it denies (the parent must consent first).
        Box::pin(async move {
            self.app
                .state::<AppState>()
                .has_chat_consent(&self.parent_conversation_id)
        })
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
        if let Some((runtime, run)) = &self.managed {
            return runtime.running(&self.parent_conversation_id, &self.task_id, run)
                && self
                    .app
                    .state::<AppState>()
                    .is_chat_generation_active(conversation_id, generation);
        }
        // Cascade: the sub-agent run is active only while BOTH its own
        // generation and the parent generation are live. Parent cancel ⇒
        // sub-agent stops on its next loop check.
        generation_cascade_active(
            &self.app.state::<AppState>(),
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

struct SubAgentToolExecutor {
    app: AppHandle,
    managed: Option<(String, String, String)>,
}

impl ToolExecutor for SubAgentToolExecutor {
    fn call<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
        skill_cache: Option<&'a mut crate::skills::SkillRunCache>,
    ) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            if let Some((conversation, id, run)) = &self.managed {
                control::runtime(&self.app)?.tool_record(conversation, id, run, serde_json::json!({"id":ctx.tool_call_id,"name":tool.name,"arguments":arguments,"status":"unknown"}))?;
            }
            let native_ctx = crate::mcp::registry::NativeToolContext {
                conversation_id: ctx.tool_conversation_id.to_string(),
                message_id: ctx.message_id.to_string(),
                tool_call_id: Some(ctx.tool_call_id.to_string()),
                run_id: ctx.run_id.to_string(),
                generation: ctx.generation,
                depth: ctx.depth,
            };
            let result = crate::mcp::registry::call_tool(
                &self.app,
                &self.app.state::<AppState>(),
                tool,
                arguments,
                skill_cache,
                Some(native_ctx),
            )
            .await;
            if let Some((conversation, id, run)) = &self.managed {
                control::runtime(&self.app)?.tool_record(conversation, id, run, serde_json::json!({"id":ctx.tool_call_id,"name":tool.name,"status":if result.is_ok() {"returned"} else {"failed"},"result":format!("{result:?}")}))?;
            }
            result
        })
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Resolve the provider+model a sub-agent should run on. Three tiers:
/// agent-definition `model` (on the PARENT provider) → global override
/// (`chat_tools.sub_agent_provider_id`/`sub_agent_model`, may switch provider) →
/// parent conversation provider+model. An override provider that no longer
/// resolves falls back to the parent instead of failing the spawn.
fn resolve_sub_agent_provider_model(
    settings: &Settings,
    parent_provider_id: &str,
    parent_model: &str,
    def_model: Option<&str>,
) -> Result<(ModelProvider, String), String> {
    let parent_provider = || {
        settings
            .get_provider(parent_provider_id)
            .cloned()
            .ok_or_else(|| "Parent chat provider not found".to_string())
    };

    // Tier 1: definition override — parent provider, definition's model.
    if let Some(m) = def_model.map(str::trim).filter(|m| !m.is_empty()) {
        return Ok((parent_provider()?, m.to_string()));
    }

    // Tier 2: global sub-agent override (both fields must be usable).
    let ov_provider_id = settings.chat_tools.sub_agent_provider_id.trim();
    let ov_model = settings.chat_tools.sub_agent_model.trim();
    if !ov_provider_id.is_empty() && !ov_model.is_empty() {
        if let Some(p) = settings
            .get_provider(ov_provider_id)
            .filter(|p| p.enabled && p.has_credentials())
        {
            return Ok((p.clone(), ov_model.to_string()));
        }
        // Unusable override → fall through to the parent (tier 3).
    }

    // Tier 3: follow the parent conversation.
    Ok((parent_provider()?, parent_model.to_string()))
}

pub struct SubAgentRequest {
    pub managed: Option<runtime::Record>,
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
    /// 父对话工作目录，用于扫描项目 `.kivio/skills` 与 `.agents/skills`。
    pub skill_project_cwd: Option<std::path::PathBuf>,
}

/// Run one worker with the shared loop's bounded model-step recovery.
/// Never restart a whole task after an error: its tools may already have run.
pub(crate) async fn run_worker_loop(
    mut config: AgentRunConfig<'_>,
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
) -> Result<AgentRunResult, String> {
    let mut previous: Option<AgentRunResult> = None;
    loop {
        let incoming = host
            .checkpoint_runtime(
                &config.conversation_id,
                &config.run_id,
                &config.runtime_messages,
                false,
            )
            .await?;
        config.runtime_messages.extend(incoming);
        let mut result = run_agent_loop(config.clone(), host, executor).await?;
        let mut history = result.compacted_history.clone().unwrap_or_else(|| {
            let mut history = config.runtime_messages.clone();
            history.extend(result.api_messages.iter().cloned());
            history
        });
        if result.stream_outcome != "completed" {
            host.close_runtime_input()?;
        }
        let incoming = host
            .checkpoint_runtime(&config.conversation_id, &config.run_id, &history, true)
            .await?;
        if let Some(mut prior) = previous.take() {
            if let Some(old) = prior.usage {
                let total = result.usage.get_or_insert_with(Default::default);
                let add = |slot: &mut Option<u64>, value: Option<u64>| {
                    if let Some(value) = value {
                        *slot = Some(slot.unwrap_or(0).saturating_add(value));
                    }
                };
                add(&mut total.input_tokens, old.input_tokens);
                add(&mut total.output_tokens, old.output_tokens);
                add(&mut total.total_tokens, old.total_tokens);
                add(&mut total.cached_input_tokens, old.cached_input_tokens);
                add(
                    &mut total.cache_creation_input_tokens,
                    old.cache_creation_input_tokens,
                );
                add(&mut total.reasoning_tokens, old.reasoning_tokens);
            }
            prior.api_messages.extend(result.api_messages);
            result.api_messages = prior.api_messages;
            prior.tool_records.extend(result.tool_records);
            result.tool_records = prior.tool_records;
            prior.segments.extend(result.segments);
            result.segments = prior.segments;
        }
        if incoming.is_empty() || result.stream_outcome != "completed" {
            return Ok(result);
        }
        history.extend(incoming);
        config.runtime_messages = history;
        previous = Some(result);
    }
}

async fn run_sub_agent(app: AppHandle, req: SubAgentRequest) -> Result<AgentRunResult, String> {
    let state = app.state::<AppState>();
    let state: &AppState = &state;
    let sub_conversation_id = format!("subagent-{}", req.task_id);

    let sub_generation = state.next_chat_generation(&sub_conversation_id);
    let sub_run_id = req
        .managed
        .as_ref()
        .map(|r| r.current().id.clone())
        .unwrap_or_else(|| format!("subrun-{}", req.task_id));
    let sub_message_id = format!("submsg-{}", req.task_id);

    let runtime_messages = req
        .managed
        .as_ref()
        .map(|r| r.history.clone())
        .unwrap_or_else(|| {
            vec![
                serde_json::json!({ "role": "system", "content": req.system_prompt }),
                serde_json::json!({ "role": "user", "content": req.prompt }),
            ]
        });

    let host = SubAgentHost {
        managed: req.managed.as_ref().map(|r| {
            (
                control::runtime(&app).expect("runtime already initialized"),
                r.current().id.clone(),
            )
        }),
        workflow_hooks: crate::plugins::packages::hook_runtime(
            req.skill_project_cwd
                .clone()
                .unwrap_or_else(std::env::temp_dir),
            req.agent_type.clone(),
            None,
        ),
        app: app.clone(),
        parent_conversation_id: req.parent_conversation_id.clone(),
        parent_run_id: req.parent_run_id.clone(),
        parent_tool_call_id: req.parent_tool_call_id.clone(),
        parent_generation: req.parent_generation,
        task_id: req.task_id.clone(),
        name: req.name.clone(),
        model: req.model.clone(),
        depth: req.depth,
        progress: Mutex::new(ProgressState::default()),
    };
    let executor = SubAgentToolExecutor {
        app: app.clone(),
        managed: req.managed.as_ref().map(|r| {
            (
                r.conversation_id.clone(),
                r.id.clone(),
                r.current().id.clone(),
            )
        }),
    };

    let thinking_enabled = req.settings.chat.thinking_enabled;
    let max_output_tokens = req.max_output_tokens;
    let retry_attempts = if req.settings.retry_enabled {
        req.settings.retry_attempts as usize
    } else {
        1
    };
    let effective_chat_tools = req.settings.chat_tools.clone();

    let config = AgentRunConfig {
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
        thinking_enabled,
        thinking_level: None,
        // 子代理不做联网搜索（父代理的搜索结果已在上下文里）。
        web_search_mode: crate::chat::types::WebSearchMode::Off,
        max_output_tokens,
        retry_attempts,
        assistant_snapshot: None,
        provider_tools_fallback_system_prompt: req.system_prompt.clone(),
        initial_anchor_total_tokens: None,
        initial_anchor_trailing_estimate: 0,
        skill_project_cwd: req.skill_project_cwd.clone(),
    };

    // No wall-clock cap: a sub-agent now runs to natural completion or until
    // cancelled via generation cascade (parent stop ⇒ host.is_generation_active
    // flips false ⇒ the loop ends gracefully). A 300s hard timeout was removed
    // because real multi-round file work legitimately exceeds it.
    let outcome = run_worker_loop(config, &host, &executor).await;
    state.cancel_chat_generation(&sub_conversation_id);
    outcome
}

// ---------------------------------------------------------------------------
// Native tool: agent
// ---------------------------------------------------------------------------

/// Context handed to sub-agent management tool handlers, dispatched before
/// workspace resolution (these tools manage agents, not files).
pub struct SubAgentCallCtx<'a> {
    pub app: &'a AppHandle,
    pub state: &'a AppState,
    pub native_ctx: &'a crate::mcp::registry::NativeToolContext,
    pub arguments: &'a Value,
}

/// Compose the `subagent_type` parameter description from the ACTUALLY loaded
/// definitions (built-in + user + project), so the model discovers user-authored
/// roles without being told they exist. Deliberately no JSON-Schema `enum`: a
/// role file created mid-run would be rejected provider-side, and inline ad-hoc
/// roles omit the field entirely — a soft failure listing the available roles is
/// more useful.
fn subagent_type_description(defs: &[AgentDefinition]) -> String {
    let available = defs
        .iter()
        .map(|d| {
            let description = d.description.trim();
            if description.is_empty() {
                d.name.clone()
            } else {
                format!("{} — {}", d.name, description)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut out = String::from("Named agent role.");
    if !available.is_empty() {
        out.push_str(" Available: ");
        out.push_str(&available);
        out.push('.');
    }
    out.push_str(
        " Omit to use general-purpose, or omit it and pass system_prompt/tools to define an ad-hoc role inline.",
    );
    out
}

pub fn agent_tool(defs: &[AgentDefinition]) -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__agent".to_string(),
        name: AGENT_TOOL_NAME.to_string(),
        description: "Start a child task asynchronously and return its identity and execution receipt. Provide a complete task; fresh children do not inherit parent history. Continue independent work, collect results with agent_control, and wait for required child results before finishing. Ordinary messages supplement an active child; explicit continue starts an idle child. Children share the parent working directory: avoid concurrent edits to the same file.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Complete, self-contained task for the sub-agent (it has no access to this conversation). Ask for a useful answer in the user's language. Prefer natural prose or Markdown; do not add rigid schemas, exact paragraph counts, or per-sentence citation requirements unless the user explicitly needs them. Useful partial findings with clear limitations are acceptable."
                },
                "subagent_type": {
                    "type": "string",
                    "description": subagent_type_description(defs)
                },
                "name": {
                    "type": "string",
                    "maxLength": 80,
                    "description": "Optional short label for this sub-agent run."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Ad-hoc persona for this run. Replaces the named role's prompt."
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Ad-hoc tool allow-list, replacing the named role's. Supports `mcp__<server>__*`, `mcp__*`, `*`. Empty means inherit every tool."
                },
                "disallowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Ad-hoc tool deny-list, replacing the named role's. Applied BEFORE `tools`."
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

pub fn tool_definitions(defs: &[AgentDefinition]) -> Vec<ChatToolDefinition> {
    vec![agent_tool(defs), control::definition()]
}

/// Append sub-agent management tools (model-facing), skipping the `agent`
/// spawn tool when `allow_spawn` is false (i.e. inside a sub-agent — second
/// guard against recursion alongside the depth check).
pub fn append_tool_definitions(
    tools: &mut Vec<ChatToolDefinition>,
    allow_spawn: bool,
    defs: &[AgentDefinition],
) {
    for tool in tool_definitions(defs) {
        if is_sub_agent_tool_name(&tool.name) && !allow_spawn {
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

/// Apply the `agent` call's inline role fields onto `def`. Each field is a WHOLE
/// REPLACEMENT, not a merge: merging could not express "researcher, but without
/// web_fetch", while replacement expresses any combination under one rule.
/// Empty strings / empty arrays count as "not provided".
fn apply_inline_overrides(def: &mut AgentDefinition, arguments: &Value) {
    if let Some(prompt) = arguments
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        def.system_prompt = prompt.to_string();
    }
    if let Some(tools) = inline_string_list(arguments, "tools") {
        def.tools = tools;
    }
    if let Some(denied) = inline_string_list(arguments, "disallowed_tools") {
        def.disallowed_tools = denied;
    }
}

/// Read an inline `Vec<String>` argument, returning `None` when absent or empty
/// (so an empty array never turns into a narrowing that blocks everything).
fn inline_string_list(arguments: &Value, key: &str) -> Option<Vec<String>> {
    let items: Vec<String> = arguments
        .get(key)?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    (!items.is_empty()).then_some(items)
}

/// Message for a spawn refused because its narrowing left no tools. The two
/// causes need different wording: a denylist that emptied the pool is NOT a
/// spelling problem, and reporting correctly-spelled allow entries as
/// "unresolved" sends the model off fixing names that were never wrong.
///
/// `deny_emptied_pool` must be computed on the pre-narrowing catalog, because
/// `unresolved_allow_entries` skips denied tools and therefore reports every
/// allow entry as unresolved under a deny-all — exactly the misleading case.
fn zero_tool_refusal(
    def: &AgentDefinition,
    unresolved: &[String],
    deny_emptied_pool: bool,
) -> String {
    let prefix = format!("Sub-agent '{}' would launch with zero tools", def.name);
    if deny_emptied_pool {
        return format!(
            "{prefix}: disallowed_tools ({}) removed every tool. Narrow the deny-list, or use `tools` to allow-list what the sub-agent needs.",
            def.disallowed_tools.join(", ")
        );
    }
    if !unresolved.is_empty() {
        return format!(
            "{prefix}. These entries matched no available tool: {}.",
            unresolved.join(", ")
        );
    }
    format!("{prefix}. Check its tools / disallowedTools configuration.")
}

fn err_result(message: impl Into<String>) -> McpToolCallResult {
    McpToolCallResult {
        content: message.into(),
        is_error: true,
        raw: Value::Null,
        artifacts: Vec::new(),
        structured_content: None,
        follow_up_user_messages: Vec::new(),
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}…")
}

/// Resolve the child configuration, durably admit work, and return a receipt.
pub fn handle_agent_spawn<'a>(
    ctx: SubAgentCallCtx<'a>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<McpToolCallResult, String>> + Send + 'a>,
> {
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
        let project_root =
            crate::chat::storage::resolve_conversation_project(ctx.app, &parent_conversation)
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
        // The named role is the BASE; inline `system_prompt`/`tools`/
        // `disallowed_tools` replace their counterparts for this run only (R2).
        let mut def = def.clone();
        apply_inline_overrides(&mut def, ctx.arguments);

        // Provider/model resolution, three tiers:
        //   1. agent definition `model` field — highest, resolved against the
        //      PARENT provider (pre-existing semantics, ignores the global override)
        //   2. global sub-agent override (settings.chat_tools.sub_agent_*) —
        //      switches provider AND model, allowing a cheap cross-provider model
        //   3. parent conversation provider+model — the default (follow)
        // Runtime defense: an unusable override provider falls back to the parent
        // rather than failing the spawn (sanitize_settings normally prevents this).
        let (provider, model) = resolve_sub_agent_provider_model(
            &settings,
            &parent_conversation.provider_id,
            &parent_conversation.model,
            def.model.as_deref(),
        )?;
        if !provider.has_credentials() {
            return Ok(err_result(
                "Sub-agent provider has no API key configured.".to_string(),
            ));
        }
        if model.trim().is_empty() {
            return Ok(err_result(
                "No model available for the sub-agent.".to_string(),
            ));
        }

        // Build the sub-agent's toolset: full enabled set, narrowed by the
        // agent definition, with the `agent` tool ALWAYS stripped (acceptance
        // #4). A sub-agent is a pure worker (orchestrator-worker model): it gets
        // NO todo tools, so it can never read or mutate any todo list. Task
        // delegation is top-down — the parent orchestrator owns the todos and
        // marks them itself (owner = sub-agent name) before/after the spawn.
        let mut tools = crate::mcp::registry::list_enabled_tool_catalog(ctx.app, ctx.state)
            .await
            .tools;
        // Computed on the UNFILTERED catalog (before narrowing) so the refusal
        // below can name the entries that matched nothing.
        let unresolved = crate::chat::agent::filter::unresolved_allow_entries(&tools, &def);
        // Whether the DENYLIST alone leaves nothing behind, measured before the
        // allow-list narrows anything. Drives the refusal wording below.
        let deny_emptied_pool = !def.disallowed_tools.is_empty()
            && !tools.is_empty()
            && tools.iter().all(|tool| {
                def.disallowed_tools
                    .iter()
                    .any(|entry| crate::chat::agent::filter::entry_matches(tool, entry))
                    || is_sub_agent_tool_name(&tool.name)
            });
        crate::chat::agent::filter::filter_tools_for_agent(&mut tools, &def);
        // Spec: a narrowing that resolves to the empty set must REFUSE to launch
        // rather than silently start a zero-tool worker. Skill tools are kept
        // unconditionally by the filter, so they don't count as resolved.
        // Both lists are checked: a deny-all (`disallowed_tools: ["*"]`) empties
        // the pool even when `tools` is empty, which the allow-only guard missed.
        let narrowing_requested = !def.tools.is_empty() || !def.disallowed_tools.is_empty();
        if narrowing_requested
            && tools.iter().all(|tool| {
                tool.source == "skill"
                    || crate::chat::agent::prepare::is_native_skill_tool_name(&tool.name)
            })
        {
            return Ok(err_result(zero_tool_refusal(
                &def,
                &unresolved,
                deny_emptied_pool,
            )));
        }
        let available_builtin_tools = available_builtin_tool_names(&tools);

        // Compose the sub-agent system prompt: persona prefix + base chat
        // system prompt. No todo context is injected — the worker is not aware
        // of and cannot touch the parent's todo list.
        // Real skill registry (was `SkillRegistry::default()` — an empty catalog,
        // so the skill tools the filter deliberately keeps had nothing to find).
        // Always the FULL registry: `def.skills` is a preload list, not a
        // visibility narrowing, so the sub-agent can still activate other skills.
        let skill_cwd = crate::chat::storage::resolve_conversation_working_directory(
            ctx.app,
            &parent_conversation,
            &settings.chat_tools.native_tools.working_directory,
        )
        .ok();
        let skill_registry = crate::skills::build_registry_in(
            ctx.app,
            &settings.chat_tools.skill_scan_paths,
            skill_cwd.as_deref(),
        )
        .unwrap_or_default();
        let persona =
            compose_persona_with_preloaded_skills(&def.system_prompt, &skill_registry, &def.skills);
        let system_prompt = build_chat_system_prompt(
            &language,
            false,
            settings.chat.thinking_enabled,
            &skill_registry,
            &settings.chat_tools,
            true,
            &available_builtin_tools,
            None,
            None,
            None,
            None,
            &persona,
            false,
            None,
            None,
            None,
            None,
            None,
            // Sub-agent tools use the parent conversation's same default workbench.
            crate::chat::storage::resolve_conversation_working_directory(
                ctx.app,
                &parent_conversation,
                &settings.chat_tools.native_tools.working_directory,
            )
            .ok()
            .map(|path| path.display().to_string())
            .as_deref(),
            None,
            (!settings.obsidian_vault_path.trim().is_empty())
                .then_some(settings.obsidian_vault_path.as_str()),
            &parent_conversation.additional_directories,
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

        // Model-aware output cap: prefer the model library / provider override
        // (matching top-level chat); the raw setting is only a fallback.
        let max_output_tokens = crate::chat::model_metadata::chat_max_output_tokens_on_wire(
            Some(&provider),
            &model,
            settings.chat.max_output_tokens,
        );

        let parent_run_id = ctx.native_ctx.run_id.clone();
        let parent_tool_call_id = ctx.native_ctx.tool_call_id.clone().unwrap_or_default();
        let request = SubAgentRequest {
            managed: None,
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
            parent_run_id: parent_run_id.clone(),
            parent_tool_call_id: parent_tool_call_id.clone(),
            parent_generation: ctx.native_ctx.generation,
            skill_project_cwd: skill_cwd.clone(),
        };

        let key = ctx.native_ctx.tool_call_id.as_deref().unwrap_or(&task_id);
        let record = control::launch(ctx.app, request, key)?;
        let value = serde_json::json!({"type":"subagent_started","id":record.id,"execution_id":record.current().id,"name":record.name,"conversation_id":parent_conversation_id,"status":"accepted"});
        Ok(McpToolCallResult {
            content: value.to_string(),
            is_error: false,
            raw: Value::Null,
            artifacts: Vec::new(),
            structured_content: Some(value),
            follow_up_user_messages: Vec::new(),
        })
    })
}

// Registry dispatch entry point (returns NativeToolFuture so the static
// `NativeToolCall::SubAgent` variant can hold a single fn-pointer shape).

/// `agent` spawn — already async-shaped.
pub fn dispatch_agent_spawn(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    handle_agent_spawn(ctx)
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

/// Persona + `skills:` PRELOAD (industry semantics): the listed skills' full
/// bodies are injected into the launch context so the sub-agent starts with that
/// knowledge instead of having to activate them first. It does NOT restrict which
/// skills it may activate later — the registry stays full.
///
/// Rides on `custom_system_prompt` (the persona channel) rather than
/// `active_skill_detail`: that channel only injects under the
/// `skill_md_only`/`legacy_full_body` fallback modes, so on the default
/// `progressive` mode a preload through it would silently do nothing.
fn compose_persona_with_preloaded_skills(
    persona: &str,
    registry: &SkillRegistry,
    skills: &[String],
) -> String {
    let base = compose_persona(persona);
    let bodies: Vec<String> = skills
        .iter()
        .filter_map(|id| registry.find(id))
        .filter(|record| !record.body.trim().is_empty())
        .map(|record| format!("Preloaded Skill ({}):\n{}", record.meta.name, record.body))
        .collect();
    if bodies.is_empty() {
        return base;
    }
    format!("{base}\n\n{}", bodies.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_agent_tool_name_detection() {
        assert!(is_sub_agent_tool_name("agent"));
        assert!(!is_sub_agent_tool_name("check_agent_result"));
        assert!(!is_sub_agent_tool_name("read_file"));
    }

    #[test]
    fn progress_upsert_dedups_by_id_and_updates_status() {
        let mut p = ProgressState::default();
        // Same call id transitions Pending→Running→Success: one slot, not three.
        p.upsert_tool("call-1", "web_search", ToolCallStatus::Pending);
        p.upsert_tool("call-1", "web_search", ToolCallStatus::Running);
        p.upsert_tool("call-1", "web_search", ToolCallStatus::Success);
        assert_eq!(p.tools.len(), 1);
        assert!(matches!(p.tools[0].status, ToolCallStatus::Success));
    }

    #[test]
    fn progress_aggregate_counts_per_tool_name() {
        let mut p = ProgressState::default();
        // 6 distinct web_search calls done, 2 still running.
        for i in 0..6 {
            p.upsert_tool(
                &format!("ws-done-{i}"),
                "web_search",
                ToolCallStatus::Success,
            );
        }
        for i in 0..2 {
            p.upsert_tool(
                &format!("ws-run-{i}"),
                "web_search",
                ToolCallStatus::Running,
            );
        }
        // 3 read_file done, 1 failed.
        for i in 0..3 {
            p.upsert_tool(
                &format!("rf-done-{i}"),
                "read_file",
                ToolCallStatus::Success,
            );
        }
        p.upsert_tool("rf-fail", "read_file", ToolCallStatus::Error);

        let steps = p.aggregate_steps();
        // One line per distinct tool name, first-seen order preserved.
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0], "web_search · 6 done · 2 running");
        assert_eq!(steps[1], "read_file · 3 done · 1 failed");
    }

    #[test]
    fn progress_aggregate_omits_zero_count_states() {
        let mut p = ProgressState::default();
        p.upsert_tool("g-1", "grep", ToolCallStatus::Running);
        let steps = p.aggregate_steps();
        assert_eq!(steps, vec!["grep · 1 running".to_string()]);
    }

    #[test]
    fn append_tools_strips_spawn_when_not_allowed() {
        let mut tools = Vec::new();
        append_tool_definitions(&mut tools, false, &[]);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !names.contains(&"agent"),
            "spawn tool must be hidden in sub-agents"
        );
    }

    #[test]
    fn append_tools_includes_spawn_when_allowed() {
        let mut tools = Vec::new();
        append_tool_definitions(&mut tools, true, &[]);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"agent"));
    }

    // -----------------------------------------------------------------------
    // Dynamic role schema (R1) + inline ad-hoc roles (R2)
    // -----------------------------------------------------------------------

    fn test_def(id: &str, description: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            name: id.to_string(),
            description: description.to_string(),
            system_prompt: "base persona".to_string(),
            model: None,
            tools: vec!["read".to_string()],
            disallowed_tools: Vec::new(),
            skills: Vec::new(),
            source: "user".to_string(),
        }
    }

    /// The `subagent_type` description must name EVERY loaded role (including
    /// user/project `.md` roles) — that is how the model discovers them.
    #[test]
    fn schema_description_lists_every_loaded_role() {
        let defs = [
            crate::agents::builtin_agent_definitions(),
            vec![test_def("my-analyst", "财报结构化分析")],
        ]
        .concat();
        let tool = agent_tool(&defs);
        let description = tool.input_schema["properties"]["subagent_type"]["description"]
            .as_str()
            .expect("subagent_type description");
        for def in &defs {
            assert!(
                description.contains(&def.name),
                "role {} must be listed in the schema",
                def.name
            );
        }
        assert!(description.contains("财报结构化分析"));
        // No `enum`: an inline ad-hoc role omits the field, and a role file
        // created mid-run must not be rejected provider-side.
        assert!(tool.input_schema["properties"]["subagent_type"]
            .get("enum")
            .is_none());
    }

    /// The inline ad-hoc fields are declared even though `additionalProperties`
    /// stays false — otherwise a strict provider rejects the call.
    #[test]
    fn schema_declares_inline_role_fields() {
        let tool = agent_tool(&[]);
        let properties = &tool.input_schema["properties"];
        for key in ["system_prompt", "tools", "disallowed_tools"] {
            assert!(properties.get(key).is_some(), "{key} must be declared");
        }
        assert_eq!(tool.input_schema["additionalProperties"], false);
    }

    #[test]
    fn inline_overrides_replace_the_named_role_fields() {
        let mut def = test_def("researcher", "Read-only research");
        def.disallowed_tools = vec!["bash".to_string()];
        apply_inline_overrides(
            &mut def,
            &serde_json::json!({
                "system_prompt": "  You only read Notion.  ",
                "tools": ["mcp__notion__*", " "],
                "disallowed_tools": ["write"]
            }),
        );
        assert_eq!(def.system_prompt, "You only read Notion.");
        // Whole replacement, not a merge (blank entries dropped).
        assert_eq!(def.tools, vec!["mcp__notion__*".to_string()]);
        assert_eq!(def.disallowed_tools, vec!["write".to_string()]);
    }

    #[test]
    fn inline_overrides_absent_or_empty_leave_the_role_untouched() {
        let base = test_def("researcher", "Read-only research");
        // Nothing inline.
        let mut def = base.clone();
        apply_inline_overrides(&mut def, &serde_json::json!({ "prompt": "go" }));
        assert_eq!(def.system_prompt, base.system_prompt);
        assert_eq!(def.tools, base.tools);
        assert_eq!(def.disallowed_tools, base.disallowed_tools);
        // Blank string / empty arrays count as "not provided" — an empty allow
        // list must not become a narrowing that blocks everything.
        let mut def = base.clone();
        apply_inline_overrides(
            &mut def,
            &serde_json::json!({ "system_prompt": "   ", "tools": [], "disallowed_tools": [] }),
        );
        assert_eq!(def.system_prompt, base.system_prompt);
        assert_eq!(def.tools, base.tools);
        assert_eq!(def.disallowed_tools, base.disallowed_tools);
    }

    /// Pure-inline role: no `subagent_type`, so `general-purpose` is the base and
    /// the inline fields fill it in completely (no special-case branch needed).
    #[test]
    fn inline_only_role_builds_on_general_purpose() {
        let defs = crate::agents::builtin_agent_definitions();
        let mut def = crate::agents::find_definition(&defs, "general-purpose")
            .expect("general-purpose")
            .clone();
        apply_inline_overrides(
            &mut def,
            &serde_json::json!({
                "prompt": "audit the config",
                "system_prompt": "You audit configuration files.",
                "tools": ["read", "grep"]
            }),
        );
        assert_eq!(def.system_prompt, "You audit configuration files.");
        assert_eq!(
            def.tools,
            vec!["read".to_string(), "grep".to_string()],
            "inline allow-list must narrow a general-purpose base"
        );
    }

    /// The spawn refusal message must name the entries that resolved to nothing,
    /// which is exactly what `unresolved_allow_entries` reports.
    #[test]
    fn zero_tool_refusal_names_the_unresolved_entries() {
        let pool = vec![agent_tool(&[]), crate::mcp::types::native_read_file_tool()];
        let mut def = test_def("typo-agent", "");
        def.tools = vec!["reed_file".to_string(), "mcp__notionn__*".to_string()];
        let unresolved = crate::chat::agent::filter::unresolved_allow_entries(&pool, &def);
        assert_eq!(
            unresolved,
            vec!["reed_file".to_string(), "mcp__notionn__*".to_string()]
        );
        let message = zero_tool_refusal(&def, &unresolved, false);
        assert!(message.contains("reed_file"));
        assert!(message.contains("mcp__notionn__*"));
        assert!(message.contains("matched no available tool"));
        // And the tool table really is empty after filtering.
        let mut filtered = pool.clone();
        crate::chat::agent::filter::filter_tools_for_agent(&mut filtered, &def);
        assert!(filtered.is_empty());
    }

    /// Mirror of the spawn path's `deny_emptied_pool` computation, so the tests
    /// exercise the same predicate the runtime uses.
    fn deny_emptied_pool(pool: &[ChatToolDefinition], def: &AgentDefinition) -> bool {
        !def.disallowed_tools.is_empty()
            && !pool.is_empty()
            && pool.iter().all(|tool| {
                def.disallowed_tools
                    .iter()
                    .any(|entry| crate::chat::agent::filter::entry_matches(tool, entry))
                    || is_sub_agent_tool_name(&tool.name)
            })
    }

    #[test]
    fn deny_all_refusal_blames_the_denylist_not_the_allow_entries() {
        // Observed in the wild: the model sent `disallowed_tools: ["*"]` with the
        // built-in `coder` role. Every allow entry was spelled correctly, yet
        // `unresolved_allow_entries` reports them all (it skips denied tools), so
        // the message must key on the denylist instead — otherwise the model goes
        // off "fixing" names that were never wrong.
        let pool = vec![agent_tool(&[]), crate::mcp::types::native_read_file_tool()];
        let mut def = test_def("coder", "");
        def.tools = vec!["read".to_string(), "grep".to_string()];
        def.disallowed_tools = vec!["*".to_string()];

        assert!(deny_emptied_pool(&pool, &def));
        let unresolved = crate::chat::agent::filter::unresolved_allow_entries(&pool, &def);
        let message = zero_tool_refusal(&def, &unresolved, true);
        assert!(message.contains("disallowed_tools"), "{message}");
        assert!(
            !message.contains("matched no available tool"),
            "correctly-spelled entries must not be blamed: {message}"
        );

        // A deny-all really does empty the table even though `tools` is non-empty.
        let mut filtered = pool.clone();
        crate::chat::agent::filter::filter_tools_for_agent(&mut filtered, &def);
        assert!(filtered.is_empty());
    }

    #[test]
    fn deny_all_is_caught_even_when_no_allow_list_is_set() {
        // The refusal guard must key on EITHER list: with `tools` empty, an
        // allow-only guard would let a zero-tool sub-agent launch silently.
        let pool = vec![crate::mcp::types::native_read_file_tool()];
        let mut def = test_def("general-purpose", "");
        def.tools = Vec::new();
        def.disallowed_tools = vec!["*".to_string()];

        let mut filtered = pool.clone();
        crate::chat::agent::filter::filter_tools_for_agent(&mut filtered, &def);
        assert!(filtered.is_empty(), "deny-all empties the pool");

        let narrowing_requested = !def.tools.is_empty() || !def.disallowed_tools.is_empty();
        assert!(
            narrowing_requested,
            "the denylist alone must trip the guard"
        );
        assert!(deny_emptied_pool(&pool, &def));
    }

    // -----------------------------------------------------------------------
    // Skills preload (R4)
    // -----------------------------------------------------------------------

    fn skill_record(id: &str, body: &str) -> crate::skills::SkillRecord {
        crate::skills::SkillRecord {
            meta: crate::skills::SkillMeta {
                id: id.to_string(),
                name: id.to_string(),
                description: String::new(),
                source: "builtin".to_string(),
                path: None,
                recommended_tools: Vec::new(),
                disable_model_invocation: false,
                files: Vec::new(),
                triggers: Vec::new(),
                argument_hint: None,
                arguments: Vec::new(),
            },
            location: std::path::PathBuf::new(),
            base_dir: std::path::PathBuf::new(),
            body: body.to_string(),
        }
    }

    #[test]
    fn preloaded_skills_inject_full_bodies_into_the_persona() {
        let registry = SkillRegistry {
            records: vec![
                skill_record("pdf", "PDF STEPS BODY"),
                skill_record("docx", "DOCX STEPS BODY"),
            ],
            warnings: Vec::new(),
        };
        let persona = compose_persona_with_preloaded_skills(
            "You analyse documents.",
            &registry,
            &["pdf".to_string(), "docx".to_string()],
        );
        assert!(persona.contains("You analyse documents."));
        assert!(persona.contains("PDF STEPS BODY"));
        assert!(persona.contains("DOCX STEPS BODY"));
    }

    #[test]
    fn no_preload_leaves_the_persona_identical_and_unknown_ids_are_skipped() {
        let registry = SkillRegistry {
            records: vec![skill_record("pdf", "PDF STEPS BODY")],
            warnings: Vec::new(),
        };
        let plain = compose_persona("You analyse documents.");
        assert_eq!(
            compose_persona_with_preloaded_skills("You analyse documents.", &registry, &[]),
            plain
        );
        assert_eq!(
            compose_persona_with_preloaded_skills(
                "You analyse documents.",
                &registry,
                &["nope".to_string()]
            ),
            plain
        );
    }

    #[test]
    fn children_cannot_delegate_again() {
        assert!(depth_allows_spawn(0));
        assert!(!depth_allows_spawn(1));
        assert!(!depth_allows_spawn(2));
    }

    fn named_provider(id: &str) -> ModelProvider {
        ModelProvider {
            id: id.to_string(),
            name: id.to_string(),
            api_keys: vec!["key".to_string()],
            api_key_legacy: None,
            base_url: "http://localhost".to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        }
    }

    fn settings_with_providers(providers: Vec<ModelProvider>) -> Settings {
        Settings {
            providers,
            ..Settings::default()
        }
    }

    #[test]
    fn sub_agent_model_follows_parent_by_default() {
        let settings = settings_with_providers(vec![named_provider("parent-p")]);
        let (provider, model) =
            resolve_sub_agent_provider_model(&settings, "parent-p", "parent-model", None)
                .expect("resolves");
        assert_eq!(provider.id, "parent-p");
        assert_eq!(model, "parent-model");
    }

    #[test]
    fn sub_agent_model_uses_global_override_cross_provider() {
        let mut settings =
            settings_with_providers(vec![named_provider("parent-p"), named_provider("cheap-p")]);
        settings.chat_tools.sub_agent_provider_id = "cheap-p".to_string();
        settings.chat_tools.sub_agent_model = "cheap-model".to_string();
        let (provider, model) =
            resolve_sub_agent_provider_model(&settings, "parent-p", "parent-model", None)
                .expect("resolves");
        assert_eq!(provider.id, "cheap-p");
        assert_eq!(model, "cheap-model");
    }

    #[test]
    fn sub_agent_definition_model_beats_global_override() {
        let mut settings =
            settings_with_providers(vec![named_provider("parent-p"), named_provider("cheap-p")]);
        settings.chat_tools.sub_agent_provider_id = "cheap-p".to_string();
        settings.chat_tools.sub_agent_model = "cheap-model".to_string();
        // Definition model resolves on the PARENT provider (pre-existing semantics).
        let (provider, model) = resolve_sub_agent_provider_model(
            &settings,
            "parent-p",
            "parent-model",
            Some("def-model"),
        )
        .expect("resolves");
        assert_eq!(provider.id, "parent-p");
        assert_eq!(model, "def-model");
    }

    #[test]
    fn sub_agent_unusable_override_falls_back_to_parent() {
        // Override points at a provider that is disabled / keyless / missing →
        // fall back to the parent rather than failing the spawn.
        let mut keyless = named_provider("cheap-p");
        keyless.api_keys.clear();
        let mut settings = settings_with_providers(vec![named_provider("parent-p"), keyless]);
        settings.chat_tools.sub_agent_provider_id = "cheap-p".to_string();
        settings.chat_tools.sub_agent_model = "cheap-model".to_string();
        let (provider, model) =
            resolve_sub_agent_provider_model(&settings, "parent-p", "parent-model", None)
                .expect("resolves");
        assert_eq!(provider.id, "parent-p");
        assert_eq!(model, "parent-model");

        settings.chat_tools.sub_agent_provider_id = "missing-p".to_string();
        let (provider, model) =
            resolve_sub_agent_provider_model(&settings, "parent-p", "parent-model", None)
                .expect("resolves");
        assert_eq!(provider.id, "parent-p");
        assert_eq!(model, "parent-model");
    }
}
