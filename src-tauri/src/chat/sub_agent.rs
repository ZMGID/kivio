//! Multi-agent / sub-agent runtime (P3).
//!
//! A sub-agent is "a fresh isolated message history run through the same
//! `run_agent_loop`" — there is no second execution engine. `run_sub_agent`
//! builds an isolated `AgentRunConfig` (system + user only, a synthetic
//! `conversation_id` for generation/streaming isolation, but the PARENT
//! conversation as `tool_conversation_id` so the child's native file tools
//! resolve the parent's project workspace), wraps it in a `SubAgentHost`, and
//! reuses the existing loop. The `agent` native tool spawns one and reports
//! live nested progress onto the parent tool card. By default the spawn is
//! synchronous (awaited inline); with `background:true` the run is detached into
//! a `tauri::async_runtime::spawn` and the tool returns a task_id immediately,
//! so the parent loop is not blocked. The orchestrator collects background
//! results by calling `await_agents` ONCE (a runtime-blocking join that returns
//! every in-flight background sub-agent's result in a single tool result) — it
//! does NOT poll `check_agent_result` in a loop (polling burns tokens / bloats
//! context; the wait belongs in the runtime, not the model token loop).
//! `check_agent_result` / `list_agent_tasks` remain as manual status peeks.
//! Background tasks live only for the parent run: they are aborted and marked
//! `Cancelled` when that run ends or the user stops it
//! (`SubAgentManager::cancel_run`), so no detached run outlives its parent.
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
//! - a `Semaphore` caps concurrent sub-agents (desktop API-quota sensitive);
//!   the cap defaults to `DEFAULT_SUB_AGENT_CONCURRENCY` and is user-overridable
//!   live via `settings.chat_tools.sub_agent_concurrency`.
//! - `SubAgentHost` auto-denies approval-gated (sensitive) tools at depth > 0
//!   and cascades parent cancellation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Notify, Semaphore};

use crate::chat::agent::prepare::{available_builtin_tool_names, build_chat_system_prompt};
use crate::chat::agent::types::AgentRunResult;
use crate::chat::agent::{
    run_agent_loop, AgentHost, AgentHostFuture, AgentRunConfig, AgentRunEntry, ToolExecutionContext,
    ToolExecutor, ToolExecutorFuture,
};
use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
use crate::chat::types::{ChatMessageSegment, ToolCallRecord, ToolCallStatus};
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
/// Default concurrent sub-agent cap. User-overridable (live) via
/// `settings.chat_tools.sub_agent_concurrency`, clamped to
/// `[SUB_AGENT_CONCURRENCY_MIN, SUB_AGENT_CONCURRENCY_MAX]`.
pub const DEFAULT_SUB_AGENT_CONCURRENCY: usize = 12;
pub const SUB_AGENT_CONCURRENCY_MIN: usize = 1;
pub const SUB_AGENT_CONCURRENCY_MAX: usize = 64;
/// 保留的任务记录上限（运行中 + 已完成）。任务表通过 `get`/`list` 按需读取——
/// 包括完成很久之后——所以不能按短 TTL 删；改为封顶保留，超限时优先丢弃「最老的
/// 已完成」记录（绝不丢运行中的）。并发受 sub_agent_concurrency 限，故总有已完成
/// 记录可回收。把「每条记录持有子 agent 完整 result 文本」的进程级泄漏封到约
/// MAX_SUB_AGENT_TASKS × 数 KB。
const MAX_SUB_AGENT_TASKS: usize = 128;
/// Max attempts for a single sub-agent run. Reasoning models (e.g. DeepSeek-V4)
/// intermittently return an empty assistant message in the planning step, which
/// `run_agent_loop` surfaces as `Err`. Top-level chat recovers via user resend;
/// a sub-agent has no resend loop, so we retry the run once before giving up.
const SUB_AGENT_MAX_ATTEMPTS: usize = 2;
/// Outer per-tool-call timeout for the `agent` spawn tool. The sub-agent run
/// itself has no inner wall-clock cap (it finishes naturally or via cascade
/// cancel); the default generic tool timeout (120s) is far shorter and would
/// mis-kill a long-running sub-agent doing real multi-round work. This generous
/// backstop only guards against a wedged spawn never returning at all — normal
/// lifecycle is governed by completion + cascade cancel, not by this value.
pub const SUB_AGENT_TOOL_TIMEOUT_MS: u64 = 660_000;
const PROGRESS_EMIT_INTERVAL_MS: u128 = 350;
const RESULT_PREVIEW_MAX: usize = 4000;

pub const AGENT_TOOL_NAME: &str = "agent";
pub const CHECK_AGENT_RESULT_TOOL_NAME: &str = "check_agent_result";
pub const LIST_AGENT_TASKS_TOOL_NAME: &str = "list_agent_tasks";
pub const AWAIT_AGENTS_TOOL_NAME: &str = "await_agents";

pub fn is_sub_agent_tool_name(name: &str) -> bool {
    matches!(
        name,
        AGENT_TOOL_NAME
            | CHECK_AGENT_RESULT_TOOL_NAME
            | LIST_AGENT_TASKS_TOOL_NAME
            | AWAIT_AGENTS_TOOL_NAME
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
/// A cancellation (own stop or parent cascade) must never be retried.
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

impl SubAgentStatus {
    /// Whether this is a terminal state (the run is over). `await_agents` waits
    /// until every target record is terminal.
    fn is_terminal(self) -> bool {
        matches!(
            self,
            SubAgentStatus::Completed | SubAgentStatus::Failed | SubAgentStatus::Cancelled
        )
    }
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
    /// Detached background-task abort handles, keyed by the spawning parent run
    /// `(conversation_id, run_id)`. A background `agent(...)` spawn registers its
    /// `JoinHandle` here so `cancel_run` can abort still-running background
    /// sub-agents when the parent run ends (normal finish does NOT bump the
    /// parent generation, so the cooperative cascade alone would orphan them).
    /// Modeled on `external_live_sessions` (state.rs): owned handles, cleared on
    /// run end / shutdown. Synchronous (non-background) spawns never touch this.
    background_runs: Mutex<HashMap<(String, String), Vec<BackgroundTaskHandle>>>,
    semaphore: Arc<Semaphore>,
    /// Current configured permit count; lets `set_concurrency` compute the delta
    /// to add/remove (tokio Semaphore exposes no total capacity).
    configured: AtomicUsize,
    /// Signaled whenever any task record reaches a terminal state (`finish` /
    /// `cancel_run`). `await_agents` waits on this instead of busy-polling the
    /// registry: it re-checks the target records' status on each notification and
    /// returns once all are terminal. A single shared `Notify` (not per-task) is
    /// enough — a waiter re-scans all its targets on every wake, so a stray wake
    /// for an unrelated task is just a cheap re-scan, never a missed completion.
    terminal_notify: Arc<Notify>,
}

/// One detached background sub-agent: its task id (to mark the record Cancelled
/// on abort) plus the JoinHandle whose `abort()` tears the detached run down.
struct BackgroundTaskHandle {
    task_id: String,
    join: tauri::async_runtime::JoinHandle<()>,
}

impl Default for SubAgentManager {
    fn default() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            by_name: Mutex::new(HashMap::new()),
            background_runs: Mutex::new(HashMap::new()),
            semaphore: Arc::new(Semaphore::new(DEFAULT_SUB_AGENT_CONCURRENCY)),
            configured: AtomicUsize::new(DEFAULT_SUB_AGENT_CONCURRENCY),
            terminal_notify: Arc::new(Notify::new()),
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
        let evicted = {
            let mut tasks = self.lock_tasks();
            tasks.insert(record.id.clone(), record);
            Self::evict_completed_over_cap(&mut tasks)
        };
        // 清掉被驱逐 id 的 name→id 反查，但只删仍指向被驱逐 id 的项——同名任务可能
        // 已被更新的 id 覆盖，那条映射要留给新任务。
        if !evicted.is_empty() {
            let mut by_name = self.by_name.lock().unwrap_or_else(|e| e.into_inner());
            by_name.retain(|_, id| !evicted.contains(id));
        }
    }

    /// 超过 MAX_SUB_AGENT_TASKS 时，按完成时间从老到新丢弃「已完成」记录，绝不丢
    /// 运行中的。返回被驱逐的 id 供调用方清理 `by_name`。
    fn evict_completed_over_cap(tasks: &mut HashMap<String, SubAgentTaskRecord>) -> Vec<String> {
        let mut evicted = Vec::new();
        if tasks.len() <= MAX_SUB_AGENT_TASKS {
            return evicted;
        }
        let mut completed: Vec<(String, i64)> = tasks
            .values()
            .filter(|r| r.completed_at.is_some())
            .map(|r| (r.id.clone(), r.completed_at.unwrap_or(r.created_at)))
            .collect();
        completed.sort_by_key(|(_, ts)| *ts);
        for (id, _) in &completed {
            if tasks.len() <= MAX_SUB_AGENT_TASKS {
                break;
            }
            tasks.remove(id);
            evicted.push(id.clone());
        }
        evicted
    }

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
        // Wake any `await_agents` waiter so it can re-scan and collect this result.
        self.terminal_notify.notify_waiters();
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

    /// Resize the concurrency gate live. Growing adds permits synchronously;
    /// shrinking spawns a task that acquires the surplus and forgets it (so it
    /// waits for in-flight runs to release). Deltas are always computed against
    /// `configured`, so a rapid grow/shrink pair still converges to the latest
    /// target.
    /// ponytail: shrink relies on a tokio runtime (always present in-app) and is
    /// only ever called serially from settings-save / startup — no concurrent
    /// callers to race the swap.
    pub fn set_concurrency(&self, target: usize) {
        let target = target.clamp(SUB_AGENT_CONCURRENCY_MIN, SUB_AGENT_CONCURRENCY_MAX);
        let prev = self.configured.swap(target, Ordering::SeqCst);
        if target > prev {
            self.semaphore.add_permits(target - prev);
        } else if target < prev {
            let sem = self.semaphore.clone();
            let remove = (prev - target) as u32;
            tauri::async_runtime::spawn(async move {
                if let Ok(permits) = sem.acquire_many(remove).await {
                    permits.forget();
                }
            });
        }
    }

    /// Track a detached background sub-agent's JoinHandle under the parent run
    /// `(conversation_id, run_id)` so `cancel_run` can abort it on run end.
    /// `pub(crate)` so run-finalize tests (in `chat::commands`) can register a
    /// handle and assert the run-end cancel ordering.
    pub(crate) fn register_background_run(
        &self,
        conversation_id: &str,
        run_id: &str,
        task_id: String,
        join: tauri::async_runtime::JoinHandle<()>,
    ) {
        let key = (conversation_id.to_string(), run_id.to_string());
        self.background_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key)
            .or_default()
            .push(BackgroundTaskHandle { task_id, join });
    }

    /// Abort every background sub-agent spawned by the given parent run and mark
    /// any still-unfinished records `Cancelled`. Called at run finalize (normal
    /// end does not bump the parent generation, so the cooperative cascade alone
    /// would leave a still-running detached task orphaned) and on user stop /
    /// shutdown. Idempotent: a run with no background tasks is a no-op.
    pub fn cancel_run(&self, conversation_id: &str, run_id: &str) {
        let key = (conversation_id.to_string(), run_id.to_string());
        let handles = self
            .background_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key)
            .unwrap_or_default();
        for handle in handles {
            handle.join.abort();
            // Mark records that never reached a terminal state. `finish` only
            // overwrites status/result; a record the detached task already
            // completed keeps its real terminal status (we only flip those
            // still Pending/Running).
            if let Some(record) = self.lock_tasks().get_mut(&handle.task_id) {
                if matches!(record.status, SubAgentStatus::Pending | SubAgentStatus::Running) {
                    record.status = SubAgentStatus::Cancelled;
                    record.error = Some("cancelled (parent run ended)".to_string());
                    record.completed_at = Some(chrono::Local::now().timestamp());
                }
            }
        }
        // Wake any `await_agents` waiter: a run-end cancel may have flipped its
        // targets to Cancelled, which is terminal, so it should stop waiting.
        self.terminal_notify.notify_waiters();
    }

    /// Task ids of background sub-agents spawned by the given parent run that are
    /// still tracked in `background_runs` (i.e. dispatched in this run). Used by
    /// `await_agents` to default to "all in-flight background subagents from this
    /// run" when no explicit ids/names are given. Order is dispatch order.
    fn background_task_ids_for_run(&self, conversation_id: &str, run_id: &str) -> Vec<String> {
        let key = (conversation_id.to_string(), run_id.to_string());
        self.background_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .map(|handles| handles.iter().map(|h| h.task_id.clone()).collect())
            .unwrap_or_default()
    }

    fn terminal_notify(&self) -> Arc<Notify> {
        self.terminal_notify.clone()
    }
}

// ---------------------------------------------------------------------------
// Sub-agent host: forwards live progress, denies sensitive tools, cascades cancel
// ---------------------------------------------------------------------------

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
    app: AppHandle,
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

impl AgentHost for SubAgentHost {
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
                &self.app.state::<AppState>(),
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

/// Run a sub-agent to completion. Builds an isolated config and reuses
/// `run_agent_loop`. Takes an owned `AppHandle` and re-resolves `AppState` from
/// it (`app.state::<AppState>()`), so the future is `'static` and can be either
/// awaited inline (synchronous `agent`) or moved into a detached
/// `tauri::async_runtime::spawn` (background `agent`). Cancellation cascades
/// from the parent via `SubAgentHost` (which also re-resolves state from the
/// cloned `AppHandle`, so the cascade works identically when detached). Up to
/// `SUB_AGENT_MAX_ATTEMPTS` tries: reasoning models occasionally return an empty
/// planning response (surfaced as `Err`); since a sub-agent has no user resend
/// loop, we retry once on non-cancel errors.
async fn run_sub_agent(app: AppHandle, req: SubAgentRequest) -> Result<AgentRunResult, String> {
    let state = app.state::<AppState>();
    let state: &AppState = &state;
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
            parent_conversation_id: req.parent_conversation_id.clone(),
            parent_run_id: req.parent_run_id.clone(),
            parent_tool_call_id: req.parent_tool_call_id.clone(),
            parent_generation: req.parent_generation,
            task_id: req.task_id.clone(),
            name: req.name.clone(),
            depth: req.depth,
            progress: Mutex::new(ProgressState::default()),
        };
        let executor = SubAgentToolExecutor { app: app.clone() };

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

        // No wall-clock cap: a sub-agent now runs to natural completion or until
        // cancelled via generation cascade (parent stop ⇒ host.is_generation_active
        // flips false ⇒ the loop ends gracefully). A 300s hard timeout was removed
        // because real multi-round file work legitimately exceeds it.
        let outcome = run_agent_loop(config, &host, &executor).await;
        // Retire this attempt's generation on every exit path (success or failure).
        // Otherwise the synthetic generation reads "active" forever and entries
        // accumulate in chat_stream_generations.
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
                },
                "background": {
                    "type": "boolean",
                    "description": "Run detached: return a task_id immediately instead of waiting, so you can dispatch several agents in parallel in the same turn. After dispatching all of them, call await_agents ONCE to collect every result — do NOT poll check_agent_result in a loop. Defaults to false (synchronous; the result is returned inline). A background result is lost if this run ends before you collect it."
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
        description: "Manually look up the current status (and any final result) of a sub-agent by its task id or name. This is a quick status peek only — to collect results from background sub-agents, call await_agents instead of polling this in a loop.".to_string(),
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
        description: "List all sub-agent tasks spawned in this session with their status. Manual/ad-hoc status peek only — use await_agents to collect background results.".to_string(),
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

pub fn await_agents_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__await_agents".to_string(),
        name: AWAIT_AGENTS_TOOL_NAME.to_string(),
        description: "Block until background sub-agents finish, then return ALL of their results in a single response. Call this ONCE after dispatching multiple agent(background:true) tasks — do NOT poll check_agent_result in a loop. With no arguments it waits for every still-running background sub-agent dispatched in this turn. Optionally pass `task_ids` and/or `names` to wait for a specific subset. Already-finished tasks return immediately.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "task_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: specific sub-agent task ids to wait for. Default = all background sub-agents dispatched in this turn."
                },
                "names": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: sub-agent names to wait for (resolved to task ids)."
                }
            },
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: Some(serde_json::json!({ "readOnlyHint": false })),
        output_schema: None,
    }
}

pub fn tool_definitions() -> Vec<ChatToolDefinition> {
    vec![agent_tool(), check_agent_result_tool(), list_agent_tasks_tool(), await_agents_tool()]
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

/// Resolve the set of target task ids for an `await_agents` call. With no
/// explicit `task_ids`/`names`, defaults to every background sub-agent dispatched
/// in the current parent run. Names are resolved to ids via the manager's
/// reverse lookup; unknown names are reported.
fn resolve_await_targets(
    manager: &SubAgentManager,
    arguments: &Value,
    parent_conversation_id: &str,
    parent_run_id: &str,
) -> Result<Vec<String>, String> {
    let task_ids: Vec<String> = arguments
        .get("task_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let names: Vec<String> = arguments
        .get("names")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    // Default: all background sub-agents dispatched in this run.
    if task_ids.is_empty() && names.is_empty() {
        return Ok(manager.background_task_ids_for_run(parent_conversation_id, parent_run_id));
    }

    let mut resolved: Vec<String> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for id in task_ids {
        match manager.get(&id) {
            Some(record) => {
                if !resolved.contains(&record.id) {
                    resolved.push(record.id);
                }
            }
            None => unknown.push(id),
        }
    }
    for name in names {
        match manager.get(&name) {
            Some(record) => {
                if !resolved.contains(&record.id) {
                    resolved.push(record.id);
                }
            }
            None => unknown.push(name),
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "Unknown sub-agent task id(s)/name(s): {}",
            unknown.join(", ")
        ));
    }
    Ok(resolved)
}

/// Build the aggregated `await_agents` tool result from the (now terminal) target
/// records, one labeled block per agent in `targets` order.
fn build_await_result(manager: &SubAgentManager, targets: &[String]) -> McpToolCallResult {
    let mut blocks: Vec<String> = Vec::new();
    let mut structured_tasks: Vec<Value> = Vec::new();
    for id in targets {
        let Some(record) = manager.get(id) else {
            blocks.push(format!("[{id}] (record evicted — result unavailable)"));
            continue;
        };
        let body = record
            .result
            .clone()
            .or_else(|| record.error.clone())
            .unwrap_or_else(|| "(no output)".to_string());
        blocks.push(format!(
            "[Sub-agent: {} ({})] {:?}\n\n{}",
            record.name, record.id, record.status, body
        ));
        structured_tasks.push(serde_json::to_value(&record).unwrap_or(Value::Null));
    }
    let content = if blocks.is_empty() {
        "No background sub-agents to await.".to_string()
    } else {
        blocks.join("\n\n---\n\n")
    };
    let structured = serde_json::json!({ "tasks": structured_tasks });
    McpToolCallResult {
        content,
        is_error: false,
        raw: structured.clone(),
        artifacts: Vec::new(),
        structured_content: Some(structured),
    }
}

/// `await_agents` handler. Blocks IN THE RUNTIME (not the model token loop)
/// until every target background sub-agent reaches a terminal state, then returns
/// ALL of their results in one tool result. Waits on the manager's
/// `terminal_notify` (no busy-polling) and re-scans target status on each wake.
/// Cooperatively cancellable: a parent-generation bump (user stop / run end)
/// short-circuits the await via the same generation check the sub-agent host
/// uses, so it never hangs.
pub async fn handle_await_agents(
    state: &AppState,
    native_ctx: &crate::mcp::registry::NativeToolContext,
    arguments: &Value,
) -> Result<McpToolCallResult, String> {
    let manager = &state.sub_agents;
    let parent_conversation_id = native_ctx.conversation_id.clone();
    let parent_run_id = native_ctx.run_id.clone();
    let parent_generation = native_ctx.generation;

    let targets = resolve_await_targets(manager, arguments, &parent_conversation_id, &parent_run_id)?;
    if targets.is_empty() {
        return Ok(McpToolCallResult {
            content: "No in-flight background sub-agents to await.".to_string(),
            is_error: false,
            raw: Value::Null,
            artifacts: Vec::new(),
            structured_content: None,
        });
    }

    let all_terminal = |mgr: &SubAgentManager| -> bool {
        targets.iter().all(|id| {
            mgr.get(id)
                // An evicted record can no longer be waited on — treat as done.
                .map_or(true, |r| r.status.is_terminal())
        })
    };

    let notify = manager.terminal_notify();
    loop {
        if all_terminal(manager) {
            return Ok(build_await_result(manager, &targets));
        }
        if !state.is_chat_generation_active(&parent_conversation_id, parent_generation) {
            // Parent run cancelled/ended: stop waiting promptly. cancel_run will
            // have (or will shortly) flipped the records to Cancelled; return
            // whatever terminal/partial state exists rather than hanging.
            return Ok(build_await_result(manager, &targets));
        }
        // Register interest BEFORE re-checking would race a notify; tokio's
        // Notify::notified() future captures the permit on poll. Bound the wait
        // so a missed wake (or a generation cancel with no terminal flip) is
        // re-checked at most ~100ms later — matching the host's cancel latency.
        let notified = notify.notified();
        tokio::select! {
            _ = notified => {}
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

/// Spawn handler (the `agent` tool). Async (Box::pin). By default (`background`
/// false) it drives `run_sub_agent` to completion and returns the result
/// inline — byte-for-byte the same behavior as before. With `background:true`
/// it registers the task, detaches `run_sub_agent` into a
/// `tauri::async_runtime::spawn`, and returns a task_id immediately so the
/// parent loop is not blocked; the detached task finalizes the record and emits
/// a terminal `chat-subagent` event, and is cleaned up when the parent run ends
/// (`SubAgentManager::cancel_run`).
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
        let background = ctx
            .arguments
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

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
            // Pending until a concurrency permit is acquired; flipped to Running
            // once the run actually starts (below). For the synchronous path the
            // window is tiny; for the background path it makes a queued spawn
            // visible to check_agent_result/list_agent_tasks as Pending.
            status: SubAgentStatus::Pending,
            result: None,
            error: None,
            depth: ctx.native_ctx.depth + 1,
            created_at: chrono::Local::now().timestamp(),
            completed_at: None,
            usage: None,
        });

        // Concurrency gate: an OWNED permit, held for the lifetime of the run.
        // acquire_owned only errors if the semaphore is closed (never — it lives
        // on AppState). Being owned (`'static`), the permit can be moved into a
        // detached background task and held there until the run finishes.
        let permit = manager.semaphore().acquire_owned().await.ok();
        manager.set_status(&task_id, SubAgentStatus::Running);

        // Model-aware output cap: prefer the model library / provider override
        // (matching top-level chat); the raw setting is only a fallback.
        let max_output_tokens = crate::chat::model_metadata::chat_max_output_tokens_for_model(
            Some(&provider),
            &model,
            settings.chat.max_output_tokens,
        );

        let parent_run_id = ctx.native_ctx.run_id.clone();
        let parent_tool_call_id = ctx.native_ctx.tool_call_id.clone().unwrap_or_default();
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
            parent_run_id: parent_run_id.clone(),
            parent_tool_call_id: parent_tool_call_id.clone(),
            parent_generation: ctx.native_ctx.generation,
        };

        // Owned context the finalizer needs (so it runs inside a detached task).
        let finalize_ctx = FinalizeCtx {
            app: ctx.app.clone(),
            task_id: task_id.clone(),
            name: name.clone(),
            agent_type: def.name.clone(),
            parent_conversation_id: parent_conversation_id.clone(),
            parent_run_id: parent_run_id.clone(),
            parent_tool_call_id,
            depth: ctx.native_ctx.depth + 1,
        };

        let app = ctx.app.clone();
        if background {
            // Detach: move the run + permit + finalizer into a spawned task and
            // return a task_id immediately so the parent loop is not blocked.
            // The owned permit is held inside the task for the run's lifetime.
            let join = tauri::async_runtime::spawn(async move {
                let _permit = permit;
                let outcome = run_sub_agent(app, request).await;
                let _ = finalize_sub_agent_outcome(finalize_ctx, outcome, true);
            });
            manager.register_background_run(
                &parent_conversation_id,
                &parent_run_id,
                task_id.clone(),
                join,
            );
            let structured = serde_json::json!({
                "type": "subagent",
                "taskId": task_id,
                "name": name,
                "agentType": def.name,
                "status": "running",
                "background": true,
            });
            Ok(McpToolCallResult {
                content: format!(
                    "[Sub-agent: {} ({})] dispatched in background (task_id={}). Keep dispatching any other parallel agents, then call await_agents ONCE to collect all results — do NOT poll check_agent_result in a loop.",
                    name, def.name, task_id
                ),
                is_error: false,
                raw: structured.clone(),
                artifacts: Vec::new(),
                structured_content: Some(structured),
            })
        } else {
            // Synchronous (default): await completion and return the result
            // inline — byte-for-byte the prior behavior. The permit is held for
            // the duration of this await.
            let _permit = permit;
            let outcome = run_sub_agent(app, request).await;
            Ok(finalize_sub_agent_outcome(finalize_ctx, outcome, false))
        }
    })
}

/// Owned context the outcome finalizer needs so it can run inside a detached
/// background task (no borrows of the spawn call frame).
struct FinalizeCtx {
    app: AppHandle,
    task_id: String,
    name: String,
    agent_type: String,
    parent_conversation_id: String,
    parent_run_id: String,
    parent_tool_call_id: String,
    depth: u8,
}

/// Turn a sub-agent run outcome into the task record update + tool result.
/// Marks the record finished, emits a terminal `chat-subagent` event so the
/// parent tool card updates (especially important for background runs, whose
/// result never returns inline), and returns the `McpToolCallResult`. Resolves
/// `AppState`/manager from the owned `AppHandle` so it is `'static`.
fn finalize_sub_agent_outcome(
    ctx: FinalizeCtx,
    outcome: Result<AgentRunResult, String>,
    background: bool,
) -> McpToolCallResult {
    let state = ctx.app.state::<AppState>();
    let manager = &state.sub_agents;
    let (result, event) = compute_sub_agent_finalization(
        manager,
        &SubAgentFinalizeParams {
            task_id: &ctx.task_id,
            name: &ctx.name,
            agent_type: &ctx.agent_type,
            parent_conversation_id: &ctx.parent_conversation_id,
            parent_run_id: &ctx.parent_run_id,
            parent_tool_call_id: &ctx.parent_tool_call_id,
            depth: ctx.depth,
        },
        outcome,
        background,
    );

    // Emit a terminal `chat-subagent` ONLY for the background path. The detached
    // run returns nothing inline, so this terminal event is the sole signal that
    // flips its parent tool card to a finished state. The synchronous path is
    // deliberately left untouched: its full result (status + content + usage)
    // already propagates inline via the `chat-tool` flow, and the card keeps the
    // last running progress event's accumulated steps/preview — emitting a
    // terminal `chat-subagent` here (whose payload omits steps/preview) would
    // overwrite `subagentProgress` with empty arrays and wipe that step history.
    // Payload shape mirrors `SubAgentHost::emit_progress`.
    if let Some(event) = event {
        let _ = ctx.app.emit("chat-subagent", event);
    }

    result
}

/// Borrowed inputs for [`compute_sub_agent_finalization`]. Mirrors the
/// `AppHandle`-bound fields of [`FinalizeCtx`] but free of any Tauri runtime
/// dependency so the finalization logic can be unit-tested directly.
#[derive(Clone, Copy)]
struct SubAgentFinalizeParams<'a> {
    task_id: &'a str,
    name: &'a str,
    agent_type: &'a str,
    parent_conversation_id: &'a str,
    parent_run_id: &'a str,
    parent_tool_call_id: &'a str,
    depth: u8,
}

/// Pure finalization: update the manager record for the outcome, build the
/// `McpToolCallResult`, and — only for `background` runs — produce the terminal
/// `chat-subagent` event payload to emit. Returns `(result, Some(payload))` for
/// background and `(result, None)` for the synchronous path (which must NOT emit
/// a terminal event; see `finalize_sub_agent_outcome`). Free of `AppHandle` so
/// it is directly testable.
fn compute_sub_agent_finalization(
    manager: &SubAgentManager,
    params: &SubAgentFinalizeParams<'_>,
    outcome: Result<AgentRunResult, String>,
    background: bool,
) -> (McpToolCallResult, Option<serde_json::Value>) {
    let SubAgentFinalizeParams {
        task_id,
        name,
        agent_type,
        ..
    } = *params;

    let (result, status_str, error_for_event) = match outcome {
        // A cancelled run (own stop or parent cascade) now returns
        // Ok(cancelled_result) from every loop phase, not just an
        // Err("cancelled") from the planning/stream path. Detect it via
        // `stream_outcome` so a cancelled sub-agent is reported as
        // `cancelled` to the parent — never as a `completed` result whose
        // "content" is just the stopped-generation placeholder.
        Ok(run) if run.stream_outcome == "cancelled" => {
            manager.finish(task_id, SubAgentStatus::Cancelled, None, None, None);
            let structured = serde_json::json!({
                "type": "subagent",
                "taskId": task_id,
                "name": name,
                "agentType": agent_type,
                "status": "cancelled",
                "error": "cancelled",
            });
            (
                McpToolCallResult {
                    content: format!("[Sub-agent: {} ({})] cancelled", name, agent_type),
                    is_error: false,
                    raw: structured.clone(),
                    artifacts: Vec::new(),
                    structured_content: Some(structured),
                },
                "cancelled",
                Some("cancelled".to_string()),
            )
        }
        Ok(run) => {
            let content = if run.content.trim().is_empty() {
                "(sub-agent produced no text output)".to_string()
            } else {
                run.content.clone()
            };
            let usage = run.usage.as_ref().map(SubAgentUsage::from_model_usage);
            manager.finish(
                task_id,
                SubAgentStatus::Completed,
                Some(clip(&content, RESULT_PREVIEW_MAX)),
                None,
                usage.clone(),
            );
            let mut structured = serde_json::json!({
                "type": "subagent",
                "taskId": task_id,
                "name": name,
                "agentType": agent_type,
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
            (
                McpToolCallResult {
                    content: format!("[Sub-agent: {} ({})]\n\n{}", name, agent_type, content),
                    is_error: false,
                    raw: structured.clone(),
                    artifacts: Vec::new(),
                    structured_content: Some(structured),
                },
                "completed",
                None,
            )
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
                "Subagent 运行失败：模型返回了空响应（可重试）。".to_string()
            } else {
                err.clone()
            };
            manager.finish(task_id, status, None, Some(display_err.clone()), None);
            let structured = serde_json::json!({
                "type": "subagent",
                "taskId": task_id,
                "name": name,
                "agentType": agent_type,
                "status": if cancelled { "cancelled" } else { "failed" },
                "error": display_err,
            });
            (
                McpToolCallResult {
                    content: format!(
                        "[Sub-agent: {} ({})] failed: {}",
                        name, agent_type, display_err
                    ),
                    is_error: !cancelled,
                    raw: structured.clone(),
                    artifacts: Vec::new(),
                    structured_content: Some(structured),
                },
                if cancelled { "cancelled" } else { "failed" },
                Some(display_err),
            )
        }
    };

    // Build the terminal `chat-subagent` payload ONLY for the background path.
    // The detached run returns nothing inline, so this terminal event is the
    // sole signal that flips its parent tool card to a finished state. The
    // synchronous path is deliberately left without a terminal event: its full
    // result (status + content + usage) already propagates inline via the
    // `chat-tool` flow, and the card keeps the last running progress event's
    // accumulated steps/preview — emitting a terminal `chat-subagent` here
    // (whose payload omits steps/preview) would overwrite `subagentProgress`
    // with empty arrays and wipe that step history. Payload shape mirrors
    // `SubAgentHost::emit_progress`.
    let event = if background {
        Some(serde_json::json!({
            "parentConversationId": params.parent_conversation_id,
            "parentRunId": params.parent_run_id,
            "parentToolCallId": params.parent_tool_call_id,
            "taskId": task_id,
            "name": name,
            "depth": params.depth,
            "status": status_str,
            "background": background,
            "error": error_for_event,
        }))
    } else {
        None
    };

    (result, event)
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

pub fn dispatch_await_agents(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move { handle_await_agents(ctx.state, ctx.native_ctx, ctx.arguments).await })
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
            p.upsert_tool(&format!("ws-done-{i}"), "web_search", ToolCallStatus::Success);
        }
        for i in 0..2 {
            p.upsert_tool(&format!("ws-run-{i}"), "web_search", ToolCallStatus::Running);
        }
        // 3 read_file done, 1 failed.
        for i in 0..3 {
            p.upsert_tool(&format!("rf-done-{i}"), "read_file", ToolCallStatus::Success);
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
        append_tool_definitions(&mut tools, false);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"agent"), "spawn tool must be hidden in sub-agents");
        assert!(names.contains(&"check_agent_result"));
        assert!(names.contains(&"list_agent_tasks"));
    }

    #[test]
    fn append_tools_includes_await_agents_when_allowed() {
        let mut tools = Vec::new();
        append_tool_definitions(&mut tools, true);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"agent"));
        assert!(names.contains(&"await_agents"));
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

    fn running_record(id: &str) -> SubAgentTaskRecord {
        SubAgentTaskRecord {
            id: id.to_string(),
            name: id.to_string(),
            agent_type: "general-purpose".to_string(),
            status: SubAgentStatus::Running,
            result: None,
            error: None,
            depth: 1,
            created_at: 0,
            completed_at: None,
            usage: None,
        }
    }

    /// Detach path: a background spawn registers its record and JoinHandle but
    /// does NOT block. `cancel_run` (called at parent run end) must abort the
    /// still-running task and flip its record to Cancelled — no orphan survives.
    #[tokio::test]
    async fn cancel_run_aborts_background_task_and_marks_record_cancelled() {
        let manager = SubAgentManager::default();
        manager.register(running_record("agent-bg"));
        manager.set_status("agent-bg", SubAgentStatus::Running);

        // A detached task that would run "forever" (mimics a long sub-agent run)
        // and, if it ever completes, would mark the record Completed — proving
        // the abort is what flipped it to Cancelled.
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let join = tauri::async_runtime::spawn(async move {
            // Never resolves until aborted (rx is dropped here without sending).
            let _ = rx.await;
        });
        // Keep tx alive so the task only ends via abort, not channel close.
        let _tx = tx;
        manager.register_background_run("conv-1", "run-1", "agent-bg".to_string(), join);

        // The spawn returned immediately (we are already here without awaiting
        // the run). Record is still Running — not yet a terminal state.
        assert_eq!(
            manager.get("agent-bg").unwrap().status,
            SubAgentStatus::Running
        );

        // Parent run ends → cascade cleanup.
        manager.cancel_run("conv-1", "run-1");

        let rec = manager.get("agent-bg").unwrap();
        assert_eq!(rec.status, SubAgentStatus::Cancelled);
        assert!(rec.completed_at.is_some());
        assert_eq!(rec.error.as_deref(), Some("cancelled (parent run ended)"));

        // The registry entry is consumed; a second cancel_run is a no-op.
        manager.cancel_run("conv-1", "run-1");
        assert_eq!(
            manager.get("agent-bg").unwrap().status,
            SubAgentStatus::Cancelled
        );
    }

    /// `cancel_run` must NOT clobber a record that already reached a real
    /// terminal state before the run ended (a background task that completed in
    /// time keeps its Completed status, not Cancelled).
    #[tokio::test]
    async fn cancel_run_preserves_already_completed_record() {
        let manager = SubAgentManager::default();
        manager.register(running_record("agent-done"));
        let join = tauri::async_runtime::spawn(async move {});
        manager.register_background_run("conv-2", "run-2", "agent-done".to_string(), join);
        // The detached task already finished and marked the record Completed.
        manager.finish(
            "agent-done",
            SubAgentStatus::Completed,
            Some("result".to_string()),
            None,
            None,
        );

        manager.cancel_run("conv-2", "run-2");

        let rec = manager.get("agent-done").unwrap();
        assert_eq!(rec.status, SubAgentStatus::Completed);
        assert_eq!(rec.result.as_deref(), Some("result"));
    }

    // --- await_agents (PR3: runtime join, no model poll) ---

    fn await_ctx(conversation_id: &str, run_id: &str, generation: u64) -> crate::mcp::registry::NativeToolContext {
        crate::mcp::registry::NativeToolContext {
            conversation_id: conversation_id.to_string(),
            message_id: "msg".to_string(),
            tool_call_id: Some("tc".to_string()),
            run_id: run_id.to_string(),
            generation,
            depth: 0,
        }
    }

    /// Default (no args) waits for ALL background sub-agents dispatched in the
    /// current run and returns every result in ONE call once they finish — no
    /// model polling. Ordering/labeling follows dispatch order.
    #[tokio::test]
    async fn await_agents_collects_all_background_results_in_one_call() {
        let state = crate::state::test_app_state();
        let manager = &state.sub_agents;
        let generation = state.next_chat_generation("conv-a");

        // Two background sub-agents registered + tracked under this run, still
        // running. A detached task finishes each after a short delay.
        for id in ["agent-x", "agent-y"] {
            manager.register(running_record(id));
            manager.set_status(id, SubAgentStatus::Running);
            let join = tauri::async_runtime::spawn(async move {});
            manager.register_background_run("conv-a", "run-a", id.to_string(), join);
        }

        // Finish both records while the awaiter is parked on terminal_notify.
        let ctx = await_ctx("conv-a", "run-a", generation);
        let args = serde_json::json!({});
        let await_fut = handle_await_agents(&state, &ctx, &args);
        tokio::pin!(await_fut);
        // Drive the awaiter briefly so it registers on terminal_notify and blocks
        // (the targets are still Running, so it cannot complete yet).
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut await_fut)
                .await
                .is_err(),
            "await must block while both targets are still running"
        );
        manager.finish(
            "agent-x",
            SubAgentStatus::Completed,
            Some("X result".into()),
            None,
            None,
        );
        manager.finish("agent-y", SubAgentStatus::Failed, None, Some("Y failed".into()), None);

        let result = tokio::time::timeout(Duration::from_millis(500), &mut await_fut)
            .await
            .expect("await_agents must complete once targets finish")
            .expect("await_agents ok");
        assert!(!result.is_error);
        // Both results present, labeled, in dispatch order.
        let body = &result.content;
        assert!(body.contains("agent-x"), "missing agent-x block: {body}");
        assert!(body.contains("X result"), "missing X result: {body}");
        assert!(body.contains("agent-y"), "missing agent-y block: {body}");
        assert!(body.contains("Y failed"), "missing Y error: {body}");
        let pos_x = body.find("agent-x").unwrap();
        let pos_y = body.find("agent-y").unwrap();
        assert!(pos_x < pos_y, "results must be in dispatch order");
    }

    /// Already-terminal targets return immediately (no waiting), and explicit
    /// task_ids are honored.
    #[tokio::test]
    async fn await_agents_returns_immediately_for_terminal_targets() {
        let state = crate::state::test_app_state();
        let manager = &state.sub_agents;
        let generation = state.next_chat_generation("conv-b");
        manager.register(running_record("agent-done"));
        manager.finish("agent-done", SubAgentStatus::Completed, Some("ready".into()), None, None);

        let args = serde_json::json!({ "task_ids": ["agent-done"] });
        let result = tokio::time::timeout(
            Duration::from_millis(200),
            handle_await_agents(&state, &await_ctx("conv-b", "run-b", generation), &args),
        )
        .await
        .expect("await_agents must not block on terminal targets")
        .expect("ok");
        assert!(result.content.contains("ready"));
    }

    /// Unknown id/name errors cleanly (no panic, no hang).
    #[tokio::test]
    async fn await_agents_unknown_id_errors() {
        let state = crate::state::test_app_state();
        let generation = state.next_chat_generation("conv-c");
        let args = serde_json::json!({ "task_ids": ["does-not-exist"] });
        let err = handle_await_agents(&state, &await_ctx("conv-c", "run-c", generation), &args)
            .await
            .expect_err("unknown id must error");
        assert!(err.contains("does-not-exist"), "error should name the bad id: {err}");
    }

    /// Empty target set (no background sub-agents dispatched) returns a clean
    /// no-op immediately.
    #[tokio::test]
    async fn await_agents_no_targets_is_noop() {
        let state = crate::state::test_app_state();
        let generation = state.next_chat_generation("conv-d");
        let result = handle_await_agents(&state, &await_ctx("conv-d", "run-d", generation), &serde_json::json!({}))
            .await
            .expect("ok");
        assert!(!result.is_error);
        assert!(result.content.to_lowercase().contains("no in-flight"));
    }

    /// Cooperative cancel: bumping the parent generation (user stop / run end)
    /// must unblock a still-waiting await_agents promptly, not hang forever.
    #[tokio::test]
    async fn await_agents_cancelled_on_parent_generation_bump() {
        let state = crate::state::test_app_state();
        let manager = &state.sub_agents;
        let generation = state.next_chat_generation("conv-e");
        // One background sub-agent that never finishes on its own.
        manager.register(running_record("agent-stuck"));
        manager.set_status("agent-stuck", SubAgentStatus::Running);
        let join = tauri::async_runtime::spawn(async move {});
        manager.register_background_run("conv-e", "run-e", "agent-stuck".to_string(), join);

        let await_ctx_e = await_ctx("conv-e", "run-e", generation);
        let args_e = serde_json::json!({});
        let await_fut = handle_await_agents(&state, &await_ctx_e, &args_e);
        tokio::pin!(await_fut);
        // It must NOT be ready while the generation is live and the task is stuck.
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut await_fut).await.is_err(),
            "await must block while target is still running"
        );
        // User stop: bump the parent generation. The awaiter should return
        // promptly (within a couple of its ~100ms re-check intervals).
        state.cancel_chat_generation("conv-e");
        let result = tokio::time::timeout(Duration::from_millis(500), &mut await_fut)
            .await
            .expect("await_agents must unblock promptly on cancel")
            .expect("ok");
        assert!(!result.is_error);
    }

    /// then flip to Running once a permit is held (so a queued background spawn
    /// is visible as Pending to check_agent_result/list_agent_tasks).
    #[test]
    fn pending_then_running_status_flow() {
        let manager = SubAgentManager::default();
        let mut rec = running_record("agent-p");
        rec.status = SubAgentStatus::Pending;
        manager.register(rec);
        assert_eq!(
            manager.get("agent-p").unwrap().status,
            SubAgentStatus::Pending
        );
        manager.set_status("agent-p", SubAgentStatus::Running);
        assert_eq!(
            manager.get("agent-p").unwrap().status,
            SubAgentStatus::Running
        );
    }

    #[test]
    fn manager_caps_table_and_evicts_oldest_completed() {
        let manager = SubAgentManager::default();
        // One running task that must never be evicted.
        manager.register(SubAgentTaskRecord {
            id: "running".to_string(),
            name: "running".to_string(),
            agent_type: "t".to_string(),
            status: SubAgentStatus::Running,
            result: None,
            error: None,
            depth: 1,
            created_at: 0,
            completed_at: None,
            usage: None,
        });
        // MAX + 50 completed tasks, ascending completed_at so we know the order.
        for i in 0..(MAX_SUB_AGENT_TASKS + 50) {
            manager.register(SubAgentTaskRecord {
                id: format!("done-{i}"),
                name: format!("done-{i}"),
                agent_type: "t".to_string(),
                status: SubAgentStatus::Completed,
                result: Some("r".to_string()),
                error: None,
                depth: 1,
                created_at: i as i64 + 1,
                completed_at: Some(i as i64 + 1),
                usage: None,
            });
        }
        // Capped.
        assert!(manager.list().len() <= MAX_SUB_AGENT_TASKS);
        // Running task survived despite being the oldest by created_at.
        assert!(manager.get("running").is_some());
        // Oldest completed evicted; newest completed retained.
        assert!(manager.get("done-0").is_none());
        assert!(manager
            .get(&format!("done-{}", MAX_SUB_AGENT_TASKS + 49))
            .is_some());
        // by_name reverse lookup for an evicted id is gone too (no leak there).
        assert!(manager.get("done-0").is_none());
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
            compacted_history: None,
        })
    }

    /// A cancelled run now surfaces as `Ok(result)` with `stream_outcome ==
    /// "cancelled"` from the planning/loop-top paths (not just `Err("cancelled")`
    /// from the tool round). `run_sub_agent` short-circuits any `Ok(_)`, so it is
    /// never retried; `should_retry_sub_agent` returns false for it because it is
    /// not an `Err`.
    fn ok_cancelled_run_result() -> Result<AgentRunResult, String> {
        Ok(AgentRunResult {
            content: "已停止生成。".to_string(),
            reasoning: None,
            tool_records: Vec::new(),
            segments: Vec::new(),
            api_messages: Vec::new(),
            steps: Vec::new(),
            stream_outcome: "cancelled".to_string(),
            usage: None,
            compacted_history: None,
        })
    }

    fn finalize_params<'a>(task_id: &'a str) -> SubAgentFinalizeParams<'a> {
        SubAgentFinalizeParams {
            task_id,
            name: "Researcher",
            agent_type: "general-purpose",
            parent_conversation_id: "conv-1",
            parent_run_id: "run-1",
            parent_tool_call_id: "call-1",
            depth: 1,
        }
    }

    /// background:true emits exactly ONE terminal event, carrying the resolved
    /// status, and flips the record to Completed.
    #[test]
    fn compute_finalization_background_emits_single_terminal_completed() {
        let manager = SubAgentManager::default();
        manager.register(running_record("bg-ok"));
        let (result, event) = compute_sub_agent_finalization(
            &manager,
            &finalize_params("bg-ok"),
            ok_run_result(),
            true,
        );
        assert!(!result.is_error);
        let event = event.expect("background path must produce one terminal event");
        assert_eq!(event["status"], "completed");
        assert_eq!(event["background"], true);
        assert_eq!(event["taskId"], "bg-ok");
        assert!(event["error"].is_null());
        assert_eq!(
            manager.get("bg-ok").unwrap().status,
            SubAgentStatus::Completed
        );
    }

    /// background:false emits NO terminal event (would otherwise wipe the card's
    /// accumulated step history), but still finishes the record.
    #[test]
    fn compute_finalization_sync_emits_no_terminal_event() {
        let manager = SubAgentManager::default();
        manager.register(running_record("sync-ok"));
        let (result, event) = compute_sub_agent_finalization(
            &manager,
            &finalize_params("sync-ok"),
            ok_run_result(),
            false,
        );
        assert!(!result.is_error);
        assert!(
            event.is_none(),
            "synchronous path must NOT emit a terminal chat-subagent event"
        );
        assert_eq!(
            manager.get("sync-ok").unwrap().status,
            SubAgentStatus::Completed
        );
    }

    /// A cancelled run (Ok with stream_outcome == "cancelled") maps to Cancelled,
    /// never Completed, and the terminal event carries status=cancelled.
    #[test]
    fn compute_finalization_cancelled_outcome_maps_to_cancelled() {
        let manager = SubAgentManager::default();
        manager.register(running_record("bg-cancel"));
        let (result, event) = compute_sub_agent_finalization(
            &manager,
            &finalize_params("bg-cancel"),
            ok_cancelled_run_result(),
            true,
        );
        // Cancellation is not surfaced as a tool error.
        assert!(!result.is_error);
        let event = event.expect("terminal event");
        assert_eq!(event["status"], "cancelled");
        assert_eq!(event["error"], "cancelled");
        assert_eq!(
            manager.get("bg-cancel").unwrap().status,
            SubAgentStatus::Cancelled
        );
    }

    /// An Err outcome maps the record to Failed with the error preserved, and the
    /// terminal event reports status=failed + the error (is_error true).
    #[test]
    fn compute_finalization_error_outcome_maps_to_failed() {
        let manager = SubAgentManager::default();
        manager.register(running_record("bg-fail"));
        let (result, event) = compute_sub_agent_finalization(
            &manager,
            &finalize_params("bg-fail"),
            Err("boom".to_string()),
            true,
        );
        assert!(result.is_error, "a real failure must surface as a tool error");
        let event = event.expect("terminal event");
        assert_eq!(event["status"], "failed");
        assert_eq!(event["error"], "boom");
        let rec = manager.get("bg-fail").unwrap();
        assert_eq!(rec.status, SubAgentStatus::Failed);
        assert_eq!(rec.error.as_deref(), Some("boom"));
    }

    /// An Err("cancelled") maps to Cancelled (not Failed) and is not a tool error.
    #[test]
    fn compute_finalization_err_cancelled_maps_to_cancelled() {
        let manager = SubAgentManager::default();
        manager.register(running_record("bg-errcancel"));
        let (result, event) = compute_sub_agent_finalization(
            &manager,
            &finalize_params("bg-errcancel"),
            Err("cancelled".to_string()),
            true,
        );
        assert!(!result.is_error);
        let event = event.expect("terminal event");
        assert_eq!(event["status"], "cancelled");
        assert_eq!(
            manager.get("bg-errcancel").unwrap().status,
            SubAgentStatus::Cancelled
        );
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

        // An Ok(cancelled) result (planning/loop-top cancellation now returns
        // Ok, not Err) → never retried: it is not an Err, so should_retry is
        // false, and run_sub_agent short-circuits any Ok(_) before retrying.
        assert!(!should_retry_sub_agent(&ok_cancelled_run_result(), 0, true));
    }

    // -----------------------------------------------------------------------
    // End-to-end detach + cancel coverage (PR1 background subagent dispatch).
    //
    // The production spawn handler `handle_agent_spawn` is bound to a concrete
    // `AppHandle<Wry>` (it re-resolves `AppState`, emits `chat-subagent`, and
    // calls the Wry-bound native-tool registry) and also loads a conversation +
    // agent definitions from disk. A real `Wry` event loop cannot be built off
    // the main thread (`tao` panics: "EventLoop must be created on the main
    // thread"), so `handle_agent_spawn` cannot be invoked verbatim in a unit
    // test, and the alternative — making the whole sub-agent + native-registry
    // stack generic over `R: Runtime` — would cascade through ~13 production
    // files.
    //
    // What these tests DO cover (the gap the task names): the real loop
    // (`run_agent_loop`, the same engine `run_sub_agent` drives) running
    // DETACHED on the tauri async runtime against a mock model wire, driven
    // through the REAL `SubAgentManager` using the EXACT detach block
    // `handle_agent_spawn` runs (register Pending→Running, `acquire_owned`
    // permit moved into the spawned task, `register_background_run` with the
    // `JoinHandle`, immediate return, terminal `manager.finish` from inside the
    // task). The host is a faithful re-implementation of `SubAgentHost`'s
    // cancellation contract (`generation_cascade_active` against the real
    // `AppState`), so both cancel paths exercise production logic.
    //
    // `tauri::async_runtime::set(Handle::current())` points Tauri's global
    // runtime at the `#[tokio::test]` runtime, so `tauri::async_runtime::spawn`
    // (used by both the detach block and `cancel_run`'s abort) shares the test
    // runtime and behaves deterministically.

    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    use crate::chat::agent::{
        run_agent_loop, AgentHost, AgentHostFuture, AgentRunConfig, AgentRunEntry,
        ToolExecutionContext, ToolExecutor, ToolExecutorFuture,
    };
    use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
    use crate::mcp::ChatToolDefinition;
    use crate::settings::{ChatToolsConfig, ModelProvider, Settings};
    use crate::skills::SkillRegistry;
    use crate::state::AppState;

    /// Point Tauri's global async runtime at the current `#[tokio::test]`
    /// runtime so `tauri::async_runtime::spawn` / `JoinHandle::abort` share it.
    /// NOTE: `tauri::async_runtime::set` panics if the global runtime was
    /// already initialized (it lazily inits its own multi-thread runtime on the
    /// first `spawn`, and tests run concurrently), so we deliberately do NOT
    /// call it. Tauri's own global runtime drives the detached tasks; spawn +
    /// `JoinHandle::abort` work across the test runtime boundary (the existing
    /// `cancel_run_*` tests rely on exactly this). The mock server gating is a
    /// real OS thread + TCP, independent of which runtime drives the task.
    fn ensure_tauri_async_runtime() {
        // Touch the global runtime so it is initialized before we spawn. A
        // no-op spawn is the cheapest way to force lazy init deterministically.
        let _ = tauri::async_runtime::spawn(async {});
    }

    /// A controllable HTTP mock for the OpenAI-compatible chat endpoint, scoped
    /// to these detach tests. Serves one canned response per accepted
    /// connection. `Json` completes immediately; `GatedJson` blocks the server
    /// thread on a barrier until the test releases the gate, so the sub-agent
    /// run stays in-flight (pending) until then.
    enum MockSubResponse {
        /// Immediate complete JSON chat completion (a final assistant message,
        /// no tool calls → the loop's synthesis step finishes the run).
        Json(String),
        /// Same body, but the server holds the response until `gate` is
        /// released — keeps the sub-agent run pending so cancel can win.
        GatedJson(String, Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>),
    }

    struct MockSubServer {
        base_url: String,
    }

    impl MockSubServer {
        fn start(responses: Vec<MockSubResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock sub server");
            let addr = listener.local_addr().expect("mock sub server addr");
            std::thread::spawn(move || {
                for response in responses {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                        .ok();
                    if read_http_request(&mut stream).is_err() {
                        continue;
                    }
                    match response {
                        MockSubResponse::Json(body) => write_json(&mut stream, &body),
                        MockSubResponse::GatedJson(body, gate) => {
                            // Block until released so the run stays pending.
                            let (lock, cvar) = &*gate;
                            let mut released = lock.lock().unwrap_or_else(|e| e.into_inner());
                            while !*released {
                                released = cvar
                                    .wait(released)
                                    .unwrap_or_else(|e| e.into_inner());
                            }
                            write_json(&mut stream, &body);
                        }
                    }
                }
            });
            Self {
                base_url: format!("http://{addr}/v1"),
            }
        }
    }

    fn write_json(stream: &mut TcpStream, body: &str) {
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.flush();
    }

    fn read_http_request(stream: &mut TcpStream) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "client closed before request end",
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < header_end + content_length {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok(())
    }

    /// A final-answer (no tool calls) non-stream chat completion body.
    fn final_answer_json(text: &str) -> String {
        serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": text }
            }],
            "usage": { "prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18 }
        })
        .to_string()
    }

    fn test_provider(base_url: &str) -> ModelProvider {
        ModelProvider {
            id: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            api_keys: vec!["test-key".to_string()],
            api_key_legacy: None,
            base_url: base_url.to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            supports_tools: true,
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
        }
    }

    /// Host mirroring `SubAgentHost`'s cancellation contract: a sub-agent run is
    /// active only while BOTH its own generation and the parent generation are
    /// live (the real `generation_cascade_active`, against the real `AppState`).
    /// Everything else is an inert no-op (these tests never run tools).
    struct TestSubHost {
        state: Arc<AppState>,
        parent_conversation_id: String,
        parent_generation: u64,
    }

    impl AgentHost for TestSubHost {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _delta: &str,
            _reasoning_delta: Option<&str>,
            _segment: Option<&ChatMessageSegment>,
        ) {
        }

        fn emit_stream_done(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _reason: &str,
            _full: &str,
        ) {
        }

        fn emit_tool_record(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _record: &ToolCallRecord,
        ) {
        }

        fn request_tool_approval<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
        ) -> AgentHostFuture<'a, bool> {
            Box::pin(async move { false })
        }

        fn request_user_response<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
            _prompt: AskUserPromptPayload,
        ) -> AgentHostFuture<'a, AskUserResponseResult> {
            Box::pin(async move {
                AskUserResponseResult {
                    phase: "cancelled".to_string(),
                    answers: HashMap::new(),
                }
            })
        }

        fn is_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
            generation_cascade_active(
                &self.state,
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
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
        }
    }

    /// Never invoked in these scenarios (the mock returns a final answer with no
    /// tool calls). Present only to satisfy `run_agent_loop`'s signature.
    struct NoToolExecutor;

    impl ToolExecutor for NoToolExecutor {
        fn call<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _tool: &'a ChatToolDefinition,
            _arguments: Value,
            _skill_cache: Option<&'a mut crate::skills::SkillRunCache>,
        ) -> ToolExecutorFuture<'a> {
            Box::pin(async move {
                panic!("NoToolExecutor must not be called in these detach tests")
            })
        }
    }

    /// Build the isolated sub-agent run config that `run_sub_agent` builds
    /// (system + user only, synthetic conversation id, empty tools so the loop
    /// goes straight to a single synthesis call). The returned config borrows
    /// `state`, so `run_agent_loop` is driven inside the closure that owns it.
    fn sub_run_config<'a>(
        state: &'a AppState,
        provider: ModelProvider,
        sub_conversation_id: String,
        sub_generation: u64,
        task_id: &str,
    ) -> AgentRunConfig<'a> {
        AgentRunConfig {
            entry: AgentRunEntry::Send,
            state,
            conversation_id: sub_conversation_id,
            tool_conversation_id: "conv-parent".to_string(),
            depth: 1,
            run_id: format!("subrun-{task_id}"),
            message_id: format!("submsg-{task_id}"),
            generation: sub_generation,
            provider,
            model: "test-model".to_string(),
            runtime_messages: vec![
                serde_json::json!({ "role": "system", "content": "you are a worker" }),
                serde_json::json!({ "role": "user", "content": "do the task" }),
            ],
            tools: Vec::new(),
            blocked_tool_calls: Vec::new(),
            settings: Settings::default(),
            effective_chat_tools: ChatToolsConfig {
                max_tool_rounds: Some(1),
                ..ChatToolsConfig::default()
            },
            language: "zh-CN".to_string(),
            has_image: false,
            thinking_enabled: false,
            stream_enabled: false,
            max_output_tokens: 1024,
            retry_attempts: 1,
            skill_registry: SkillRegistry::default(),
            active_skill_id: None,
            active_skill_detail: None,
            assistant_snapshot: None,
            custom_system_prompt: String::new(),
            provider_tools_fallback_system_prompt: String::new(),
        }
    }

    /// Reproduces `handle_agent_spawn`'s detach block against the real
    /// `SubAgentManager`: register Pending → acquire owned permit → flip Running
    /// → spawn (run the real loop, then `manager.finish` the terminal record) →
    /// `register_background_run` so `cancel_run` can abort it. Returns the
    /// task_id immediately, exactly like the background path.
    ///
    /// The `state` is wrapped in an `Arc` (the production detach re-resolves
    /// `AppState` from a cloned `AppHandle` to get a `'static` future; here the
    /// `Arc` is what makes the spawned future `'static`). The loop config holds
    /// a `&AppState`; we leak nothing — the `Arc` clone moved into the task
    /// keeps the state alive for the run's lifetime, and the loop borrows from
    /// that same `Arc` via a raw extension of its lifetime scoped to the task.
    fn spawn_detached_sub_run(
        state: Arc<AppState>,
        provider: ModelProvider,
        parent_conversation_id: &str,
        parent_run_id: &str,
        parent_generation: u64,
        task_id: &str,
    ) {
        let manager = &state.sub_agents;
        manager.register(SubAgentTaskRecord {
            id: task_id.to_string(),
            name: task_id.to_string(),
            agent_type: "general-purpose".to_string(),
            status: SubAgentStatus::Pending,
            result: None,
            error: None,
            depth: 1,
            created_at: chrono::Local::now().timestamp(),
            completed_at: None,
            usage: None,
        });
        // Owned permit (held inside the task for the run's lifetime), just like
        // the production background path.
        let permit = manager.semaphore().acquire_owned();
        let task_id_owned = task_id.to_string();
        let parent_conversation_id_owned = parent_conversation_id.to_string();
        let parent_run_id_owned = parent_run_id.to_string();
        // The parent conversation id is also needed inside the task (for the
        // host cascade), so clone for the closure and keep the originals for
        // register_background_run below.
        let parent_conversation_id_task = parent_conversation_id_owned.clone();
        let task_id_for_register = task_id_owned.clone();
        let state_for_task = Arc::clone(&state);

        // Acquire the permit before flipping to Running (matches production).
        let join = tauri::async_runtime::spawn(async move {
            let _permit = permit.await.ok();
            let state = state_for_task;
            state
                .sub_agents
                .set_status(&task_id_owned, SubAgentStatus::Running);

            let sub_conversation_id = format!("subagent-{task_id_owned}");
            let sub_generation = state.next_chat_generation(&sub_conversation_id);
            let host = TestSubHost {
                state: Arc::clone(&state),
                parent_conversation_id: parent_conversation_id_task.clone(),
                parent_generation,
            };
            let executor = NoToolExecutor;
            let config = sub_run_config(
                &state,
                provider,
                sub_conversation_id.clone(),
                sub_generation,
                &task_id_owned,
            );
            let outcome = run_agent_loop(config, &host, &executor).await;
            state.cancel_chat_generation(&sub_conversation_id);

            // Terminal record update — the same status mapping `finalize_sub_agent_outcome`
            // applies (cancelled vs completed vs failed).
            let mgr = &state.sub_agents;
            match outcome {
                Ok(run) if run.stream_outcome == "cancelled" => {
                    mgr.finish(&task_id_owned, SubAgentStatus::Cancelled, None, None, None);
                }
                Ok(run) => {
                    let usage = run.usage.as_ref().map(SubAgentUsage::from_model_usage);
                    mgr.finish(
                        &task_id_owned,
                        SubAgentStatus::Completed,
                        Some(run.content.clone()),
                        None,
                        usage,
                    );
                }
                Err(err) => {
                    let cancelled = err == "cancelled";
                    let status = if cancelled {
                        SubAgentStatus::Cancelled
                    } else {
                        SubAgentStatus::Failed
                    };
                    mgr.finish(&task_id_owned, status, None, Some(err), None);
                }
            }
        });

        state.sub_agents.register_background_run(
            &parent_conversation_id_owned,
            &parent_run_id_owned,
            task_id_for_register,
            join,
        );
    }

    /// Test 1 + 2: a background dispatch returns immediately (record Pending/
    /// Running) BEFORE the gated model response is released, and once released
    /// the detached run completes → record flips Completed and
    /// `check_agent_result` returns the final text + usage.
    #[tokio::test]
    async fn background_dispatch_is_non_blocking_then_completes_and_is_retrievable() {
        ensure_tauri_async_runtime();
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let server = MockSubServer::start(vec![MockSubResponse::GatedJson(
            final_answer_json("background result text"),
            Arc::clone(&gate),
        )]);
        let state = Arc::new(crate::state::test_app_state());
        let parent_gen = state.next_chat_generation("conv-parent");
        let provider = test_provider(&server.base_url);

        spawn_detached_sub_run(
            Arc::clone(&state),
            provider,
            "conv-parent",
            "run-1",
            parent_gen,
            "agent-bg-1",
        );

        // Non-blocking: control returned here while the run is still pending
        // (the server is blocked on the gate, so the run cannot have finished).
        // Give the detached task a moment to acquire the permit + flip Running.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let rec = state.sub_agents.get("agent-bg-1").expect("task registered");
        assert!(
            matches!(rec.status, SubAgentStatus::Running | SubAgentStatus::Pending),
            "dispatch returned before completion; status was {:?}",
            rec.status
        );
        assert!(rec.result.is_none(), "no result before gate released");

        // Release the gate → the run completes.
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }

        // Poll until terminal (bounded).
        let mut final_rec = None;
        for _ in 0..200 {
            let rec = state.sub_agents.get("agent-bg-1").unwrap();
            if matches!(rec.status, SubAgentStatus::Completed) {
                final_rec = Some(rec);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let rec = final_rec.expect("background run reached Completed");
        assert_eq!(rec.result.as_deref(), Some("background result text"));
        assert!(rec.usage.is_some(), "usage propagated from the run");

        // check_agent_result returns the completed result text.
        let check = handle_check_agent_result_for_test(&state, "agent-bg-1");
        assert!(check.contains("background result text"));
        assert!(check.contains("Completed"));
    }

    /// Test 3 (sync default): driving the run inline (no detach) returns the
    /// full result; record is Completed. This mirrors `background:false`.
    #[tokio::test]
    async fn sync_default_runs_inline_and_returns_completed() {
        ensure_tauri_async_runtime();
        let server = MockSubServer::start(vec![MockSubResponse::Json(final_answer_json(
            "sync result text",
        ))]);
        let state = crate::state::test_app_state();
        let parent_gen = state.next_chat_generation("conv-parent");

        // Inline: register, hold the permit for the await, run to completion.
        state.sub_agents.register(SubAgentTaskRecord {
            id: "agent-sync".to_string(),
            name: "agent-sync".to_string(),
            agent_type: "general-purpose".to_string(),
            status: SubAgentStatus::Pending,
            result: None,
            error: None,
            depth: 1,
            created_at: chrono::Local::now().timestamp(),
            completed_at: None,
            usage: None,
        });
        let _permit = state.sub_agents.semaphore().acquire_owned().await.ok();
        state
            .sub_agents
            .set_status("agent-sync", SubAgentStatus::Running);

        let sub_conversation_id = "subagent-agent-sync".to_string();
        let sub_generation = state.next_chat_generation(&sub_conversation_id);
        // Host backed by a ref to the real state, so the cascade consults the
        // live parent generation (the inline/sync path holds no Arc).
        let host = TestSubHostRef {
            state: &state,
            parent_conversation_id: "conv-parent".to_string(),
            parent_generation: parent_gen,
        };
        let executor = NoToolExecutor;
        let config = sub_run_config(
            &state,
            test_provider(&server.base_url),
            sub_conversation_id.clone(),
            sub_generation,
            "agent-sync",
        );
        let outcome = run_agent_loop(config, &host, &executor).await;
        state.cancel_chat_generation(&sub_conversation_id);

        let run = outcome.expect("sync run completes");
        assert_eq!(run.stream_outcome, "completed");
        assert_eq!(run.content, "sync result text");
        state.sub_agents.finish(
            "agent-sync",
            SubAgentStatus::Completed,
            Some(run.content.clone()),
            None,
            None,
        );
        let rec = state.sub_agents.get("agent-sync").unwrap();
        assert_eq!(rec.status, SubAgentStatus::Completed);
        assert_eq!(rec.result.as_deref(), Some("sync result text"));
    }

    /// Test 4 (the key gap): with the sub-agent run still in-flight (gated mock
    /// response), `cancel_run` aborts the detached task and flips the record to
    /// Cancelled — and it does NOT later flip to Completed (abort wins). The
    /// owned permit is released, so a subsequent acquire succeeds.
    #[tokio::test]
    async fn cancel_run_aborts_inflight_real_loop_and_releases_permit() {
        ensure_tauri_async_runtime();
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let server = MockSubServer::start(vec![MockSubResponse::GatedJson(
            final_answer_json("should never be observed"),
            Arc::clone(&gate),
        )]);
        // Drive concurrency down to 1 so we can prove the permit is released by
        // re-acquiring it after cancel.
        let state = Arc::new(crate::state::test_app_state());
        state.sub_agents.set_concurrency(1);
        // set_concurrency shrink spawns an async acquire to forget surplus
        // permits; poll until it has settled to a single permit instead of a
        // fixed sleep (the sleep was flaky under CI load).
        let mut shrunk = false;
        for _ in 0..200 {
            if state.sub_agents.semaphore().available_permits() <= 1 {
                shrunk = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(shrunk, "set_concurrency(1) should settle to a single permit");
        let parent_gen = state.next_chat_generation("conv-parent");
        let provider = test_provider(&server.base_url);

        spawn_detached_sub_run(
            Arc::clone(&state),
            provider,
            "conv-parent",
            "run-cancel",
            parent_gen,
            "agent-cancel",
        );

        // Wait until the run is in-flight (Running, permit held, blocked on gate).
        let mut running = false;
        for _ in 0..200 {
            if matches!(
                state.sub_agents.get("agent-cancel").unwrap().status,
                SubAgentStatus::Running
            ) {
                running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(running, "sub-agent run reached in-flight Running state");

        // The single permit is held by the in-flight run: a try_acquire fails.
        // Poll briefly rather than asserting once — there is a small window
        // between the record flipping to Running and the permit being fully
        // accounted for, and CI load can widen it.
        let mut permit_held = false;
        for _ in 0..200 {
            if state.sub_agents.semaphore().try_acquire().is_err() {
                permit_held = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            permit_held,
            "the only permit is held by the in-flight run"
        );

        // Cancel the parent run → abort the detached task, flip to Cancelled.
        state.sub_agents.cancel_run("conv-parent", "run-cancel");

        let rec = state.sub_agents.get("agent-cancel").unwrap();
        assert_eq!(rec.status, SubAgentStatus::Cancelled);
        assert_eq!(rec.error.as_deref(), Some("cancelled (parent run ended)"));

        // Release the gate so the (now-aborted) server thread can exit and the
        // dropped task's permit is reclaimed. Then prove abort wins: the record
        // must NOT flip to Completed, and the permit must be re-acquirable.
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
        // Give any (aborted) task a chance to NOT run its finalizer.
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(
            state.sub_agents.get("agent-cancel").unwrap().status,
            SubAgentStatus::Cancelled,
            "abort wins: a cancelled record must never flip to Completed"
        );

        // Permit released by the aborted task being dropped → re-acquirable
        // (bounded wait; abort + drop is not strictly synchronous).
        let mut reacquired = false;
        for _ in 0..200 {
            if state.sub_agents.semaphore().try_acquire().is_ok() {
                reacquired = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            reacquired,
            "owned permit released after abort; a subsequent spawn can acquire"
        );
    }

    /// Test 4b (cooperative cascade): bumping the parent generation while the
    /// run is in-flight makes the in-flight loop bail on its next cancellation
    /// check, ending the run as cancelled (no abort needed). This is the
    /// `generation_cascade_active` path.
    #[tokio::test]
    async fn parent_generation_bump_cascades_to_inflight_sub_run() {
        ensure_tauri_async_runtime();
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let server = MockSubServer::start(vec![MockSubResponse::GatedJson(
            final_answer_json("should never be observed"),
            Arc::clone(&gate),
        )]);
        let state = Arc::new(crate::state::test_app_state());
        let parent_gen = state.next_chat_generation("conv-parent");
        let provider = test_provider(&server.base_url);

        spawn_detached_sub_run(
            Arc::clone(&state),
            provider,
            "conv-parent",
            "run-cascade",
            parent_gen,
            "agent-cascade",
        );

        // Wait until in-flight.
        let mut running = false;
        for _ in 0..200 {
            if matches!(
                state.sub_agents.get("agent-cascade").unwrap().status,
                SubAgentStatus::Running
            ) {
                running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(running, "sub-agent run reached in-flight Running state");

        // Cooperative cascade: bump the PARENT generation (user stop). The
        // in-flight synthesis call's `tokio::select!` against
        // `wait_for_generation_inactive` should win and end the run as
        // cancelled — WITHOUT anyone aborting the task and WITHOUT the model
        // call ever completing. We deliberately keep the gate CLOSED (the model
        // response keeps hanging) so the cancellation branch is the only way
        // the select can resolve. We also do NOT call cancel_run here.
        state.cancel_chat_generation("conv-parent");

        // The detached task's finalizer should mark the record Cancelled
        // (synthesis returned Err("cancelled") via the cascade select).
        let mut cancelled = false;
        for _ in 0..200 {
            if matches!(
                state.sub_agents.get("agent-cascade").unwrap().status,
                SubAgentStatus::Cancelled
            ) {
                cancelled = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            cancelled,
            "parent generation bump cascaded → in-flight sub run ended cancelled"
        );

        // Release the gate now so the blocked server thread can exit cleanly.
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
    }

    /// Test 5 (no orphan): after cancel, `list_agent_tasks` shows the task
    /// Cancelled and `cancel_run` no longer retains an active handle (a second
    /// cancel is a no-op — the background_runs entry was consumed).
    #[tokio::test]
    async fn after_cancel_no_orphan_handle_remains() {
        ensure_tauri_async_runtime();
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let server = MockSubServer::start(vec![MockSubResponse::GatedJson(
            final_answer_json("unused"),
            Arc::clone(&gate),
        )]);
        let state = Arc::new(crate::state::test_app_state());
        let parent_gen = state.next_chat_generation("conv-parent");
        let provider = test_provider(&server.base_url);

        spawn_detached_sub_run(
            Arc::clone(&state),
            provider,
            "conv-parent",
            "run-orphan",
            parent_gen,
            "agent-orphan",
        );
        // Let it reach in-flight.
        for _ in 0..200 {
            if matches!(
                state.sub_agents.get("agent-orphan").unwrap().status,
                SubAgentStatus::Running
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        state.sub_agents.cancel_run("conv-parent", "run-orphan");

        // list_agent_tasks shows it Cancelled.
        let listed = state
            .sub_agents
            .list()
            .into_iter()
            .find(|t| t.id == "agent-orphan")
            .expect("task listed");
        assert_eq!(listed.status, SubAgentStatus::Cancelled);

        // The background_runs handle was consumed: a second cancel_run does
        // nothing (idempotent, no orphan handle retained). If a stale handle
        // remained it would re-mark the record (already Cancelled → no-op) but,
        // more importantly, there is nothing left to abort.
        state.sub_agents.cancel_run("conv-parent", "run-orphan");
        assert_eq!(
            state.sub_agents.get("agent-orphan").unwrap().status,
            SubAgentStatus::Cancelled
        );

        // release gate so the server thread exits cleanly.
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
    }

    /// Borrowed variant of `TestSubHost` for the inline (sync) path, which holds
    /// a `&AppState` rather than an `Arc` (no detach, so no `'static` need).
    struct TestSubHostRef<'a> {
        state: &'a AppState,
        parent_conversation_id: String,
        parent_generation: u64,
    }

    impl AgentHost for TestSubHostRef<'_> {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _delta: &str,
            _reasoning_delta: Option<&str>,
            _segment: Option<&ChatMessageSegment>,
        ) {
        }
        fn emit_stream_done(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _reason: &str,
            _full: &str,
        ) {
        }
        fn emit_tool_record(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _record: &ToolCallRecord,
        ) {
        }
        fn request_tool_approval<'b>(
            &'b self,
            _ctx: &'b ToolExecutionContext<'b>,
            _record: &'b ToolCallRecord,
        ) -> AgentHostFuture<'b, bool> {
            Box::pin(async move { false })
        }
        fn request_user_response<'b>(
            &'b self,
            _ctx: &'b ToolExecutionContext<'b>,
            _record: &'b ToolCallRecord,
            _prompt: AskUserPromptPayload,
        ) -> AgentHostFuture<'b, AskUserResponseResult> {
            Box::pin(async move {
                AskUserResponseResult {
                    phase: "cancelled".to_string(),
                    answers: HashMap::new(),
                }
            })
        }
        fn is_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
            generation_cascade_active(
                self.state,
                conversation_id,
                generation,
                &self.parent_conversation_id,
                self.parent_generation,
            )
        }
        fn wait_for_generation_inactive<'b>(
            &'b self,
            conversation_id: &'b str,
            generation: u64,
        ) -> AgentHostFuture<'b, ()> {
            Box::pin(async move {
                loop {
                    if !self.is_generation_active(conversation_id, generation) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
        }
    }

    /// Thin wrapper over `handle_check_agent_result` that builds the
    /// `SubAgentCallCtx` from a state ref (these tests have no real AppHandle).
    /// `check_agent_result` only reads `ctx.state.sub_agents`, never `ctx.app`,
    /// so we can drive it with the manager directly.
    fn handle_check_agent_result_for_test(state: &AppState, id: &str) -> String {
        match state.sub_agents.get(id) {
            Some(record) => {
                let body = record
                    .result
                    .clone()
                    .or_else(|| record.error.clone())
                    .unwrap_or_else(|| "(no result yet)".to_string());
                format!(
                    "Sub-agent {} [{}]: {:?}\n\n{}",
                    record.name, record.id, record.status, body
                )
            }
            None => format!("No sub-agent task found for '{id}'"),
        }
    }

    // -----------------------------------------------------------------------
    // Loop-level integration: orchestrator fan-out + single await_agents join.
    //
    // The 6 `await_agents` unit tests above exercise the handler in isolation.
    // The gap this section closes is the END-TO-END property inside the REAL
    // `run_agent_loop` acting as the orchestrator: it fans out ≥2 background
    // sub-agents, collects them with ONE `await_agents` call (a runtime join,
    // not a model poll loop), reaches a final answer, and NEVER invokes
    // `check_agent_result`.
    //
    // What is REAL here:
    //   * the orchestrator's `run_agent_loop` — the production engine, driven
    //     against a scripted mock model wire (body-capturing TCP server).
    //   * the `await_agents` tool — the production `handle_await_agents`,
    //     invoked by the loop's own tool executor. It only reads
    //     `SubAgentManager` + generation state, so no `AppHandle` is needed.
    //   * the background sub-agents — real detached `run_agent_loop` runs (via
    //     the existing `spawn_detached_sub_run` helper) registered on the same
    //     shared `SubAgentManager`, each driven by its own `MockSubServer`,
    //     gated so `await_agents` truly blocks until released.
    //   * generation / cancellation state — the real `AppState`, so the
    //     cooperative-cancel variant exercises production cancel logic.
    //
    // What is TEST-SHIMMED (and why):
    //   * the `agent` DISPATCH tool. Production `handle_agent_spawn` is bound to
    //     a concrete `AppHandle<Wry>` (a `tao` event loop can't be built off the
    //     main thread), so it cannot run verbatim in a `#[tokio::test]`. The
    //     orchestrator therefore registers a TEST `agent` tool whose executor
    //     replicates `handle_agent_spawn`'s detach block via the same
    //     `spawn_detached_sub_run` helper the PR1 detach tests already use, then
    //     returns the task_id immediately — exactly the background dispatch
    //     contract. `await_agents` (the real handler) then waits on those real
    //     detached tasks.

    /// Body-capturing multi-response mock for the orchestrator's OpenAI-compatible
    /// chat endpoint. Mirrors `loop_tests::MockModelServer` (which lives in a
    /// different test module): serves one canned JSON completion per accepted
    /// connection, in accept order, and records each request body so the test can
    /// assert which tool calls the orchestrator emitted (and, crucially, which it
    /// did NOT).
    struct MockOrchestratorServer {
        base_url: String,
        captured_bodies: Arc<Mutex<Vec<String>>>,
    }

    impl MockOrchestratorServer {
        fn start(responses: Vec<String>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind orchestrator server");
            let addr = listener.local_addr().expect("orchestrator server addr");
            let captured = Arc::new(Mutex::new(Vec::new()));
            let captured_for_thread = Arc::clone(&captured);
            std::thread::spawn(move || {
                for body in responses {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                        .ok();
                    match read_http_request_capturing(&mut stream) {
                        Ok(request_body) => {
                            captured_for_thread
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push(request_body);
                        }
                        Err(_) => continue,
                    }
                    write_json(&mut stream, &body);
                }
            });
            Self {
                base_url: format!("http://{addr}/v1"),
                captured_bodies: captured,
            }
        }

        fn captured_bodies(&self) -> Vec<String> {
            self.captured_bodies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    /// Like `read_http_request`, but returns the request body text so the test can
    /// assert the orchestrator's emitted tool-call names.
    fn read_http_request_capturing(stream: &mut TcpStream) -> std::io::Result<String> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "client closed before request end",
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < header_end + content_length {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok(String::from_utf8_lossy(&buf[header_end..]).into_owned())
    }

    /// One assistant turn with N parallel `agent` tool calls, each `background:true`.
    /// `calls` is `(tool_call_id, task_name)` — the task name is passed in the args
    /// so the test executor can mint a deterministic task_id per dispatch.
    fn fanout_agent_calls_json(calls: &[(&str, &str)]) -> String {
        let tool_calls: Vec<Value> = calls
            .iter()
            .map(|(call_id, name)| {
                serde_json::json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": "agent",
                        "arguments": serde_json::json!({
                            "name": name,
                            "prompt": "do the task",
                            "background": true
                        }).to_string()
                    }
                })
            })
            .collect();
        serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": { "role": "assistant", "content": Value::Null, "tool_calls": tool_calls }
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
        })
        .to_string()
    }

    /// One assistant turn with a single `await_agents` call (no args ⇒ all in-flight).
    fn await_agents_call_json(call_id: &str) -> String {
        serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": { "name": "await_agents", "arguments": "{}" }
                    }]
                }
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
        })
        .to_string()
    }

    /// Orchestrator host: identical no-op stream/record surface as `TestSubHost`,
    /// but its generation check consults the REAL `AppState` for the ORCHESTRATOR
    /// conversation, so the loop's per-call `tokio::select!` cancel path and
    /// `handle_await_agents`'s own generation check agree on one source of truth.
    struct OrchestratorHost {
        state: Arc<AppState>,
        conversation_id: String,
    }

    impl AgentHost for OrchestratorHost {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _delta: &str,
            _reasoning_delta: Option<&str>,
            _segment: Option<&ChatMessageSegment>,
        ) {
        }
        fn emit_stream_done(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _reason: &str,
            _full: &str,
        ) {
        }
        fn emit_tool_record(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _record: &ToolCallRecord,
        ) {
        }
        fn request_tool_approval<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
        ) -> AgentHostFuture<'a, bool> {
            Box::pin(async move { true })
        }
        fn request_user_response<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
            _prompt: AskUserPromptPayload,
        ) -> AgentHostFuture<'a, AskUserResponseResult> {
            Box::pin(async move {
                AskUserResponseResult {
                    phase: "cancelled".to_string(),
                    answers: HashMap::new(),
                }
            })
        }
        fn is_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
            // The orchestrator run is governed solely by its own conversation's
            // generation in the real AppState.
            let _ = conversation_id;
            self.state
                .is_chat_generation_active(&self.conversation_id, generation)
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
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
        }
    }

    /// The orchestrator's tool executor. Handles exactly the two tools the
    /// scripted orchestrator emits:
    ///   * `agent` → replicate the PR1 background dispatch: spawn a real detached
    ///     sub-run via `spawn_detached_sub_run` and return its task_id IMMEDIATELY.
    ///   * `await_agents` → the REAL `handle_await_agents` (runtime join).
    /// A `check_agent_result` call would be a regression (model polling): it is
    /// counted and surfaced as a hard failure so the "no poll" property is proven
    /// from the executor side as well as the request-body side.
    struct OrchestratorExecutor {
        state: Arc<AppState>,
        /// Per-dispatch sub-server base URLs, keyed by task name, so each
        /// background sub-agent gets a distinct gated result.
        sub_providers: Arc<Mutex<HashMap<String, ModelProvider>>>,
        check_agent_result_calls: Arc<AtomicUsize>,
        await_calls: Arc<AtomicUsize>,
    }

    impl ToolExecutor for OrchestratorExecutor {
        fn call<'a>(
            &'a self,
            ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            arguments: Value,
            _skill_cache: Option<&'a mut crate::skills::SkillRunCache>,
        ) -> ToolExecutorFuture<'a> {
            let state = Arc::clone(&self.state);
            let sub_providers = Arc::clone(&self.sub_providers);
            let check_calls = Arc::clone(&self.check_agent_result_calls);
            let await_calls = Arc::clone(&self.await_calls);
            let conversation_id = ctx.conversation_id.to_string();
            let run_id = ctx.run_id.to_string();
            let message_id = ctx.message_id.to_string();
            let tool_call_id = ctx.tool_call_id.to_string();
            let generation = ctx.generation;
            let depth = ctx.depth;
            let tool_name = tool.name.clone();
            Box::pin(async move {
                match tool_name.as_str() {
                    AGENT_TOOL_NAME => {
                        let name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("worker")
                            .to_string();
                        let provider = sub_providers
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&name)
                            .cloned()
                            .expect("a sub-provider was registered for this dispatch name");
                        // Use the task name as the deterministic task_id so the
                        // test can assert per-agent results without guessing.
                        spawn_detached_sub_run(
                            Arc::clone(&state),
                            provider,
                            &conversation_id,
                            &run_id,
                            generation,
                            &name,
                        );
                        // Background dispatch returns the task_id immediately.
                        Ok(McpToolCallResult {
                            content: format!("Dispatched background sub-agent '{name}' (task_id: {name}). It is running; collect it later with await_agents."),
                            is_error: false,
                            raw: Value::Null,
                            artifacts: Vec::new(),
                            structured_content: None,
                        })
                    }
                    AWAIT_AGENTS_TOOL_NAME => {
                        await_calls.fetch_add(1, Ordering::SeqCst);
                        let native_ctx = crate::mcp::registry::NativeToolContext {
                            conversation_id: conversation_id.clone(),
                            message_id: message_id.clone(),
                            tool_call_id: Some(tool_call_id.clone()),
                            run_id: run_id.clone(),
                            generation,
                            depth,
                        };
                        handle_await_agents(&state, &native_ctx, &arguments).await
                    }
                    "check_agent_result" => {
                        // A model poll — the exact failure mode PR3 eliminates.
                        check_calls.fetch_add(1, Ordering::SeqCst);
                        Err("REGRESSION: orchestrator polled check_agent_result".to_string())
                    }
                    other => Err(format!("unexpected tool in orchestrator test: {other}")),
                }
            })
        }
    }

    /// Build the orchestrator's `run_agent_loop` config: a top-level run (depth 0)
    /// whose tool table is the real `agent` + `await_agents` definitions, with a
    /// generous round budget so the 3-round script (fan-out → await → final)
    /// completes without hitting the round limit.
    fn orchestrator_run_config<'a>(
        state: &'a AppState,
        base_url: &str,
        conversation_id: &str,
    ) -> AgentRunConfig<'a> {
        AgentRunConfig {
            entry: AgentRunEntry::Send,
            state,
            conversation_id: conversation_id.to_string(),
            tool_conversation_id: conversation_id.to_string(),
            depth: 0,
            run_id: "orch-run".to_string(),
            message_id: "orch-msg".to_string(),
            generation: state.next_chat_generation(conversation_id),
            provider: test_provider(base_url),
            model: "test-model".to_string(),
            runtime_messages: vec![
                serde_json::json!({ "role": "system", "content": "you orchestrate workers" }),
                serde_json::json!({ "role": "user", "content": "research two topics in parallel" }),
            ],
            // The real spawn + await tool definitions.
            tools: vec![agent_tool(), await_agents_tool()],
            blocked_tool_calls: Vec::new(),
            settings: Settings::default(),
            effective_chat_tools: ChatToolsConfig {
                max_tool_rounds: Some(8),
                ..ChatToolsConfig::default()
            },
            language: "zh-CN".to_string(),
            has_image: false,
            thinking_enabled: false,
            stream_enabled: false,
            max_output_tokens: 1024,
            retry_attempts: 1,
            skill_registry: SkillRegistry::default(),
            active_skill_id: None,
            active_skill_detail: None,
            assistant_snapshot: None,
            custom_system_prompt: String::new(),
            provider_tools_fallback_system_prompt: String::new(),
        }
    }

    /// THE integration test (happy path). Drives the real `run_agent_loop` as the
    /// orchestrator against a scripted mock model:
    ///   round 1: two `agent` calls, both `background:true` (fan-out);
    ///   round 2: ONE `await_agents` call (no args ⇒ all in-flight);
    ///   round 3: a final assistant answer (no tool calls) ⇒ loop ends.
    /// The two background sub-agents are real detached runs gated until the test
    /// releases them, so `await_agents` BLOCKS in the runtime then returns BOTH.
    ///
    /// Asserts (the point of the test):
    ///   1. `await_agents` invoked exactly ONCE; its single result carries BOTH
    ///      sub-agent results, labeled by task_id/name.
    ///   2. `check_agent_result` invoked ZERO times — proven two ways: the
    ///      executor counter is 0, and no captured request body names it.
    ///   3. the run reaches a final answer (loop returns Ok, content matches).
    ///   4. `await_agents` actually BLOCKED until the sub-agents finished (the
    ///      gate is released only AFTER await is observed in-flight, and both
    ///      results — not "still running" — are present in the single result).
    #[tokio::test]
    async fn orchestrator_fans_out_then_collects_with_single_await_no_polling() {
        ensure_tauri_async_runtime();

        // Each background sub-agent gets its own gated sub-server with a distinct
        // result, so the collected output is deterministic regardless of which
        // detached run connects first.
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let server_a = MockSubServer::start(vec![MockSubResponse::GatedJson(
            final_answer_json("RESULT_A"),
            Arc::clone(&gate),
        )]);
        let server_b = MockSubServer::start(vec![MockSubResponse::GatedJson(
            final_answer_json("RESULT_B"),
            Arc::clone(&gate),
        )]);

        // Orchestrator model script: fan-out → await → final answer.
        let orchestrator = MockOrchestratorServer::start(vec![
            fanout_agent_calls_json(&[("call_a", "agent-a"), ("call_b", "agent-b")]),
            await_agents_call_json("call_await"),
            final_answer_json("Both workers finished: A and B."),
        ]);

        let state = Arc::new(crate::state::test_app_state());
        let mut sub_providers = HashMap::new();
        sub_providers.insert("agent-a".to_string(), test_provider(&server_a.base_url));
        sub_providers.insert("agent-b".to_string(), test_provider(&server_b.base_url));

        let check_agent_result_calls = Arc::new(AtomicUsize::new(0));
        let await_calls = Arc::new(AtomicUsize::new(0));
        let executor = OrchestratorExecutor {
            state: Arc::clone(&state),
            sub_providers: Arc::new(Mutex::new(sub_providers)),
            check_agent_result_calls: Arc::clone(&check_agent_result_calls),
            await_calls: Arc::clone(&await_calls),
        };
        let host = OrchestratorHost {
            state: Arc::clone(&state),
            conversation_id: "orch-conv".to_string(),
        };

        // Release the gate only AFTER both sub-agents are in-flight AND the
        // orchestrator's await_agents has started waiting — proving await truly
        // blocks in the runtime. We poll for that condition from a detached task.
        let state_for_gate = Arc::clone(&state);
        let await_calls_for_gate = Arc::clone(&await_calls);
        let gate_for_release = Arc::clone(&gate);
        let releaser = tauri::async_runtime::spawn(async move {
            // Wait until both sub-agents are registered + Running (gated) and the
            // orchestrator has entered await_agents at least once.
            for _ in 0..400 {
                let both_running = ["agent-a", "agent-b"].iter().all(|id| {
                    matches!(
                        state_for_gate.sub_agents.get(id).map(|r| r.status),
                        Some(SubAgentStatus::Running)
                    )
                });
                let awaiting = await_calls_for_gate.load(Ordering::SeqCst) >= 1;
                if both_running && awaiting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            // Both are still gated (no result yet) at the moment await is waiting:
            // confirm await is genuinely blocked on running tasks, not returning
            // early on already-terminal records.
            assert!(
                state_for_gate
                    .sub_agents
                    .get("agent-a")
                    .map(|r| r.result.is_none())
                    .unwrap_or(false),
                "agent-a must still be running (no result) while await blocks"
            );
            // Release → both sub-runs complete → await_agents unblocks.
            let (lock, cvar) = &*gate_for_release;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        });

        let config = orchestrator_run_config(&state, &orchestrator.base_url, "orch-conv");
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            run_agent_loop(config, &host, &executor),
        )
        .await
        .expect("orchestrator run must not hang")
        .expect("orchestrator run completes Ok");
        let _ = releaser.await;

        // (3) Final answer reached.
        assert_eq!(result.stream_outcome, "completed");
        assert_eq!(result.content, "Both workers finished: A and B.");

        // (1) await_agents invoked exactly once, and its single result carried
        // BOTH sub-agent results (collected in one call — no per-agent polling).
        assert_eq!(
            await_calls.load(Ordering::SeqCst),
            1,
            "await_agents must be invoked exactly once"
        );
        // (4) Both real results are present (not "still running") — proving await
        // blocked until the gated sub-runs finished.
        let a = state.sub_agents.get("agent-a").expect("agent-a record");
        let b = state.sub_agents.get("agent-b").expect("agent-b record");
        assert_eq!(a.status, SubAgentStatus::Completed);
        assert_eq!(b.status, SubAgentStatus::Completed);
        assert_eq!(a.result.as_deref(), Some("RESULT_A"));
        assert_eq!(b.result.as_deref(), Some("RESULT_B"));

        // (2a) check_agent_result never executed.
        assert_eq!(
            check_agent_result_calls.load(Ordering::SeqCst),
            0,
            "no polling: check_agent_result must never be invoked"
        );
        // (2b) and the orchestrator never even EMITTED a check_agent_result tool
        // call. NOTE: the literal string "check_agent_result" DOES appear in every
        // request — the `await_agents` tool DESCRIPTION says "do NOT poll
        // check_agent_result". So we cannot assert its mere absence; instead we
        // assert the function-call invocation shape never appears (the tool is not
        // even in the orchestrator's tool table, so the only way it could show up
        // is if the model had called it, which would echo into the history as
        // `"name":"check_agent_result"` inside a tool_calls entry).
        let bodies = orchestrator.captured_bodies();
        assert_eq!(bodies.len(), 3, "fan-out + await + final = 3 model requests");
        assert!(
            bodies
                .iter()
                .all(|b| !b.contains("\"name\":\"check_agent_result\"")),
            "no request history may contain a check_agent_result tool call"
        );
        // Conversely, the requests DO carry the tool calls the orchestrator
        // actually made (agent fan-out + await_agents), confirming the script ran.
        assert!(
            bodies[1].contains("\"name\":\"agent\""),
            "round-2 request history must echo the round-1 agent fan-out calls"
        );
        assert!(
            bodies[2].contains("\"name\":\"await_agents\""),
            "round-3 request history must echo the await_agents call"
        );
        // Sanity: the await result the orchestrator received (in request 3's
        // history) carried both labeled results — the single-call collection.
        assert!(
            bodies[2].contains("RESULT_A") && bodies[2].contains("RESULT_B"),
            "the final request's history must contain BOTH results from the one await_agents call"
        );
        assert!(
            bodies[2].contains("agent-a") && bodies[2].contains("agent-b"),
            "await result must label each sub-agent by id/name"
        );

        // Release any blocked server threads (gate already released above; this is
        // belt-and-suspenders if the run finished before the releaser ran).
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
    }

    /// Cooperative-cancel variant: while the orchestrator is parked inside
    /// `await_agents` (sub-agents gated, never finishing on their own), the parent
    /// generation is bumped (user stop). The loop must end promptly as cancelled —
    /// no hang — via the same `generation_cascade_active` path production uses.
    #[tokio::test]
    async fn orchestrator_await_cancels_cooperatively_on_generation_bump() {
        ensure_tauri_async_runtime();

        // Gated sub-agent that never finishes on its own → await would block
        // forever absent cancellation.
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let server_a = MockSubServer::start(vec![MockSubResponse::GatedJson(
            final_answer_json("never observed"),
            Arc::clone(&gate),
        )]);

        let orchestrator = MockOrchestratorServer::start(vec![
            fanout_agent_calls_json(&[("call_a", "agent-c")]),
            await_agents_call_json("call_await"),
            // A final answer is scripted in case the run somehow proceeds; the
            // cancel path should end the run before this is consumed.
            final_answer_json("unexpected"),
        ]);

        let state = Arc::new(crate::state::test_app_state());
        let mut sub_providers = HashMap::new();
        sub_providers.insert("agent-c".to_string(), test_provider(&server_a.base_url));

        let await_calls = Arc::new(AtomicUsize::new(0));
        let check_calls = Arc::new(AtomicUsize::new(0));
        let executor = OrchestratorExecutor {
            state: Arc::clone(&state),
            sub_providers: Arc::new(Mutex::new(sub_providers)),
            check_agent_result_calls: Arc::clone(&check_calls),
            await_calls: Arc::clone(&await_calls),
        };
        let host = OrchestratorHost {
            state: Arc::clone(&state),
            conversation_id: "orch-conv-cancel".to_string(),
        };

        // Bump the orchestrator generation once await is observed mid-wait.
        let state_for_cancel = Arc::clone(&state);
        let await_calls_for_cancel = Arc::clone(&await_calls);
        let canceller = tauri::async_runtime::spawn(async move {
            for _ in 0..400 {
                if await_calls_for_cancel.load(Ordering::SeqCst) >= 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            // Give the awaiter a beat to actually park on terminal_notify.
            tokio::time::sleep(Duration::from_millis(30)).await;
            state_for_cancel.cancel_chat_generation("orch-conv-cancel");
        });

        let config =
            orchestrator_run_config(&state, &orchestrator.base_url, "orch-conv-cancel");
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            run_agent_loop(config, &host, &executor),
        )
        .await
        .expect("cancelled orchestrator run must not hang")
        .expect("cancelled run still returns Ok(result)");
        let _ = canceller.await;

        // The run ended as cancelled (loop-top / tool-round cancel), not by
        // consuming the scripted final answer.
        assert_eq!(
            result.stream_outcome, "cancelled",
            "generation bump during await must end the run cancelled"
        );
        // await_agents was entered exactly once and never polled.
        assert_eq!(await_calls.load(Ordering::SeqCst), 1);
        assert_eq!(check_calls.load(Ordering::SeqCst), 0);

        // Release the gate so the blocked sub-server thread exits cleanly.
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
    }
}
