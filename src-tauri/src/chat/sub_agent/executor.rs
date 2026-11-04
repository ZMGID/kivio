//! Execution of one OMP task assignment, using the shared chat agent loop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::watch;

use crate::agents::AgentDefinition;
use crate::chat::agent::compaction::CompactOutcome;
use crate::chat::agent::prepare::{available_builtin_tool_names, build_chat_prompt};
use crate::chat::agent::provider_runtime::{MessageRequest, ProviderFuture, ProviderRuntime, StreamRequest, SummaryRequest};
use crate::chat::agent::stream::ChatStreamOutput;
use crate::chat::agent::{run_agent_loop, AgentHost, AgentHostFuture, AgentRunConfig, SteeringMessage, ToolExecutionContext, ToolExecutor, ToolExecutorFuture};
use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
use crate::chat::model::{ModelError, ModelErrorKind, ModelUsage};
use crate::chat::types::{ChatMessageSegment, ToolCallRecord, ToolCallStatus, WebSearchMode};
use crate::mcp::ChatToolDefinition;
use crate::settings::{ModelProvider, Settings};
use crate::state::AppState;

use super::{delivery, prompts, registry, task};

pub(crate) struct Request {
    pub record: registry::AgentRecord,
    pub job_id: String,
    pub index: usize,
    pub definition: AgentDefinition,
    pub task: String,
    pub context: String,
    pub parent_run_id: String,
    pub parent_tool_call_id: String,
    pub parent_provider_id: String,
    pub parent_model: String,
    pub cancel_rx: watch::Receiver<bool>,
    pub permit: task::Permit,
}

const EMPTY_FINAL: &str = "SYSTEM WARNING: Subagent ended without a final message.";
const PREVIEW_CHARS: usize = 5_000;
const INLINE_CHARS: usize = 11_500;

fn budget_for(agent: &str, configured: u32) -> u32 {
    if configured == 0 {
        0
    } else if agent.eq_ignore_ascii_case("scout") || agent.eq_ignore_ascii_case("sonic") {
        configured.min(100)
    } else {
        configured
    }
}

fn force_at(budget: u32) -> u32 {
    budget.saturating_add(budget.saturating_add(1) / 2)
}

fn progress_tokens(usage: &ModelUsage, api_format: &str) -> u64 {
    let input = usage.input_tokens.unwrap_or(0);
    let input = if api_format == "anthropic_messages" {
        input
    } else {
        input.saturating_sub(usage.cached_input_tokens.unwrap_or(0))
    };
    let computed = input.saturating_add(usage.output_tokens.unwrap_or(0))
        .saturating_add(usage.cache_creation_input_tokens.unwrap_or(0));
    if computed > 0 { computed } else { usage.total_tokens.unwrap_or(0) }
}

fn budget_notice(requests: u32, budget: u32) -> String {
    format!("[budget notice] You have used {requests} requests in this run (soft budget: {budget}). Wrap up now: finish the current step and deliver your final report. At {} requests the run is force-stopped and you will be asked to report whatever you have.", force_at(budget))
}

fn budget_reminder() -> String {
    prompts::render(prompts::SUBAGENT_BUDGET_REMINDER, &json!({"budgetStop": true, "structured": false}))
}

fn resolve_model(settings: &Settings, parent_provider: &str, parent_model: &str, selected: Option<&str>) -> Result<(ModelProvider, String, Option<String>), String> {
    let parent = || settings.get_provider(parent_provider).cloned().ok_or_else(|| "Parent chat provider not found".to_string());
    let selected = selected.map(str::trim).filter(|s| !s.is_empty());
    if let Some(model) = selected.filter(|model| !model.starts_with('@')) {
        return Ok((parent()?, model.to_string(), None));
    }
    let role = selected.and_then(|model| model.strip_prefix('@')).unwrap_or("task");
    if !matches!(role, "task" | "smol" | "slow") {
        return Err(format!("Unknown mixer role '@{role}'. Use @smol, @slow or @task."));
    }
    let configured = |role: &str| settings.chat_tools.sub_agent_models.get(role)
        .filter(|selection| !selection.provider_id.is_empty() && !selection.model.is_empty());
    if let Some(selection) = configured(role).or_else(|| configured("task")) {
        if let Some(provider) = settings.get_provider(&selection.provider_id).filter(|p| p.enabled && p.authentication_ready()) {
            return Ok((provider.clone(), selection.model.clone(), selection.thinking_level.clone()));
        }
    }
    Ok((parent()?, parent_model.to_string(), None))
}

#[derive(Default)]
struct Progress {
    text: String,
    last_assistant: String,
    history: Vec<Value>,
    records: Vec<ToolCallRecord>,
    recent_tools: Vec<Value>,
    tool_count: usize,
    requests: u32,
    tokens: u64,
    context_tokens: u64,
    context_window: usize,
    cost: f64,
    completion_percent: Option<u32>,
    stage: Option<String>,
    watch: Option<String>,
    notice_sent: bool,
    budget_stop: bool,
    budget_wrapping: bool,
    hard_budget: bool,
    status: &'static str,
    dirty: bool,
}

fn count_assistant_request(progress: &mut Progress, budget: u32) {
    progress.requests = progress.requests.saturating_add(1);
    if budget > 0 && progress.requests >= force_at(budget).saturating_add(5) {
        progress.hard_budget = true;
    } else if budget > 0 && progress.requests >= force_at(budget) && !progress.budget_wrapping {
        progress.budget_stop = true;
    }
    progress.dirty = true;
}

struct Host {
    app: AppHandle,
    record: registry::AgentRecord,
    job_id: String,
    index: usize,
    agent: String,
    task: String,
    parent_run_id: String,
    parent_tool_call_id: String,
    model: Mutex<String>,
    started: Instant,
    cancel: watch::Receiver<bool>,
    budget: u32,
    permit: tokio::sync::Mutex<Option<task::Permit>>,
    progress: Mutex<Progress>,
}

impl Host {
    fn lock(&self) -> std::sync::MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cancelled(&self) -> bool {
        *self.cancel.borrow()
    }

    fn begin_request(&self) -> bool {
        let mut p = self.lock();
        if self.budget > 0 && p.requests >= force_at(self.budget).saturating_add(5) {
            p.hard_budget = true;
            p.dirty = true;
            return false;
        }
        if p.budget_stop { return false; }
        true
    }

    fn finish_request(&self) {
        count_assistant_request(&mut self.lock(), self.budget);
    }

    fn forced(&self) -> bool {
        self.budget > 0 && self.lock().requests >= force_at(self.budget)
    }

    fn update_context_usage(&self, provider: &ModelProvider, usage: Option<&ModelUsage>) {
        if let Some(total) = usage.and_then(|usage| crate::chat::agent::context_estimate::anchor_total_tokens(usage, &provider.api_format)) {
            let mut p = self.lock();
            p.context_tokens = total;
            p.dirty = true;
        }
    }

    fn note_usage(&self, provider: &ModelProvider, model: &str, usage: Option<&ModelUsage>, main_request: bool) {
        let Some(usage) = usage else { return };
        let mut p = self.lock();
        if main_request {
            p.tokens = p.tokens.saturating_add(progress_tokens(usage, &provider.api_format));
            if let Some(total) = crate::chat::agent::context_estimate::anchor_total_tokens(usage, &provider.api_format) {
                p.context_tokens = total;
            }
        }
        if let Some((pricing, _)) = crate::chat::model_metadata::pricing_for_model(Some(provider), model) {
            let input = usage.input_tokens.unwrap_or(0);
            let output = usage.output_tokens.unwrap_or(0);
            let cached = usage.cached_input_tokens.unwrap_or(0);
            let input_price = pricing.input.unwrap_or(0.0);
            let cached_price = pricing.cached_input.unwrap_or(input_price);
            let output_price = pricing.output.unwrap_or(0.0);
            let cost = if provider.api_format == "anthropic_messages" {
                let multiplier = if model.to_ascii_lowercase().contains("claude") {
                    if provider.cache_retention() == crate::settings::CacheRetention::Long { 2.0 } else { 1.25 }
                } else { 1.0 };
                input as f64 * input_price + cached as f64 * cached_price
                    + usage.cache_creation_input_tokens.unwrap_or(0) as f64 * input_price * multiplier
                    + output as f64 * output_price
            } else {
                let cached = cached.min(input);
                (input - cached) as f64 * input_price + cached as f64 * cached_price + output as f64 * output_price
            };
            p.cost += cost / 1_000_000.0;
        }
        p.dirty = true;
    }

    fn snapshot(&self, force: bool) -> Option<Value> {
        let mut p = self.lock();
        if !force && !p.dirty { return None; }
        p.dirty = false;
        let current = p.records.iter().rev().find(|record| record.status == ToolCallStatus::Running);
        let args = current.map(tool_args_preview);
        let intent = current.and_then(tool_intent);
        let tail_start = p.text.char_indices().rev().nth(8_192).map(|(index, _)| index).unwrap_or(0);
        let recent_output: Vec<&str> = p.text[tail_start..].lines().filter(|line| !line.trim().is_empty()).rev().take(8).collect();
        let description: String = self.task.lines().find(|line| !line.trim().is_empty()).unwrap_or("").chars().take(160).collect();
        Some(json!({
            "index": self.index, "id": self.record.id, "agent": self.agent,
            "status": p.status, "task": self.task, "description": description,
            "currentTool": current.map(|record| &record.name),
            "currentToolArgs": args, "currentToolIntent": intent,
            "currentToolStartMs": current.and_then(|record| record.started_at).map(|time| time * 1000),
            "recentTools": p.recent_tools, "recentOutput": recent_output,
            "toolCount": p.tool_count, "requests": p.requests, "tokens": p.tokens,
            "contextTokens": p.context_tokens, "contextWindow": p.context_window,
            "cost": p.cost, "durationMs": self.started.elapsed().as_millis() as u64,
            "completionPercent": p.completion_percent, "stage": p.stage, "watch": p.watch,
        }))
    }

    fn emit_progress(&self, force: bool) {
        let Some(progress) = self.snapshot(force) else { return };
        let state = self.app.state::<AppState>();
        state.sub_agents.jobs.report_progress(&self.job_id, progress.clone());
        let output = progress["recentOutput"].as_array().map(|lines| {
            lines.iter().rev().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")
        }).unwrap_or_default();
        let steps = progress["recentTools"].as_array().map(|tools| tools.iter().filter_map(|tool| {
            tool["tool"].as_str().map(str::to_string)
        }).collect()).unwrap_or_default();
        crate::chat::protocol::emit_run_event_or_live(&self.app, &self.record.root, &self.parent_run_id,
            crate::chat::protocol::ChatRunEvent::SubagentUpdated {
                parent_tool_call_id: self.parent_tool_call_id.clone(), task_id: self.record.id.clone(),
                name: self.record.display_name.clone(), model: Some(self.model.lock().unwrap_or_else(|e| e.into_inner()).clone()),
                depth: self.record.depth, status: progress["status"].as_str().unwrap_or("running").to_string(),
                preview: Some(output), steps,
            });
    }

    async fn save_history(&self, history: &[Value]) -> Result<(), String> {
        let records = self.lock().records.clone();
        let mut history = history.to_vec();
        annotate_tool_results(&mut history, &records);
        let mut data = String::new();
        for message in history {
            data.push_str(&serde_json::to_string(&message).map_err(|error| error.to_string())?);
            data.push('\n');
        }
        write_artifact(&self.record.session_file, data.as_bytes()).await.map_err(|error| format!("Cannot write subagent transcript: {error}"))
    }

    async fn wait_children(&self, conversation: &str, generation: u64) -> Result<bool, String> {
        let state = self.app.state::<AppState>();
        let jobs = &state.sub_agents.jobs;
        let mut changed = jobs.subscribe();
        let running = !jobs.running(&self.record.root, &self.record.id).is_empty();
        if !running {
            return Ok(!jobs.undelivered(&self.record.root, &self.record.id).is_empty());
        }
        // shortcut: parked final turns hold no wake monitor; add IRC wake turns in step 2.
        let mut slot = self.permit.lock().await;
        let suspended = slot.take().map(task::Permit::release);
        let mut cancel = self.cancel.clone();
        loop {
            if !self.is_generation_active(conversation, generation) { return Err("cancelled".into()); }
            if jobs.running(&self.record.root, &self.record.id).is_empty() { break; }
            tokio::select! {
                result = changed.changed() => if result.is_err() { break; },
                result = cancel.changed() => if result.is_err() || *cancel.borrow() { return Err("cancelled".into()); },
                _ = state.chat_runtime().wait_for_generation_inactive(conversation, generation) => return Err("cancelled".into()),
            }
        }
        if let Some(suspended) = suspended {
            let max = state.settings_read().chat_tools.task_max_concurrency;
            *slot = tokio::select! {
                permit = suspended.reacquire(max, &mut cancel) => permit,
                _ = state.chat_runtime().wait_for_generation_inactive(conversation, generation) => return Err("cancelled".into()),
            };
            if slot.is_none() { return Err("cancelled".into()); }
        }
        Ok(true)
    }
}

fn annotate_tool_results(history: &mut [Value], records: &[ToolCallRecord]) {
    let mut statuses: HashMap<&str, Vec<bool>> = HashMap::new();
    for record in records {
        let failed = match record.status {
            ToolCallStatus::Success => false,
            ToolCallStatus::Error | ToolCallStatus::Cancelled | ToolCallStatus::Skipped => true,
            ToolCallStatus::Pending | ToolCallStatus::Running => continue,
        };
        statuses.entry(&record.id).or_default().push(failed);
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for message in history.iter().filter(|message| message["role"] == "tool") {
        if let Some(id) = message["tool_call_id"].as_str() {
            *counts.entry(id.to_string()).or_default() += 1;
        }
    }
    let mut offsets: HashMap<String, usize> = counts.into_iter().map(|(id, count)| {
        let offset = statuses.get(id.as_str()).map_or(0, |items| items.len().saturating_sub(count));
        (id, offset)
    }).collect();
    for message in history.iter_mut().filter(|message| message["role"] == "tool") {
        let failure = message["tool_call_id"].as_str().and_then(|id| {
            let offset = offsets.get_mut(id)?;
            let status = statuses.get(id)?.get(*offset).copied();
            *offset += 1;
            status
        });
        if let Some(failure) = failure { message["is_error"] = json!(failure); }
    }
}

fn tool_args_preview(record: &ToolCallRecord) -> String {
    let parsed: Value = serde_json::from_str(&record.arguments).unwrap_or(Value::Null);
    let text = ["path", "command", "pattern", "query", "url", "task"]
        .iter().find_map(|key| parsed[*key].as_str()).unwrap_or(&record.arguments);
    text.chars().take(200).collect()
}

fn tool_intent(record: &ToolCallRecord) -> Option<String> {
    let parsed: Value = serde_json::from_str(&record.arguments).ok()?;
    parsed["i"].as_str().or_else(|| parsed["intent"].as_str()).map(str::to_string)
}

impl AgentHost for Host {
    fn requires_tool_completion(&self) -> bool { true }

    fn checkpoint_runtime<'a>(&'a self, _conversation: &'a str, _run: &'a str, history: &'a [Value], _finishing: bool) -> AgentHostFuture<'a, Result<Vec<Value>, String>> {
        Box::pin(async move {
            self.lock().history = history.to_vec();
            self.save_history(history).await?;
            let mut incoming = delivery::take_inputs(&self.app, &self.record.root, &self.record.id);
            for message in &mut incoming {
                if let Some(object) = message.as_object_mut() { object.remove("subagent_parent_persisted"); }
            }
            if !incoming.is_empty() {
                let mut saved = history.to_vec();
                saved.extend(incoming.iter().cloned());
                self.lock().history = saved;
            }
            Ok(incoming)
        })
    }

    fn wait_for_child_results<'a>(&'a self, conversation: &'a str, _run: &'a str, generation: u64) -> AgentHostFuture<'a, Result<bool, String>> {
        Box::pin(self.wait_children(conversation, generation))
    }

    fn emit_stream_delta(&self, _conversation: &str, _run: &str, _message: &str, delta: &str, _reasoning: Option<&str>, _segment: Option<&ChatMessageSegment>) {
        if delta.is_empty() { return; }
        let mut p = self.lock();
        p.text.push_str(delta);
        if p.text.len() > 16_384 {
            let mut cut = p.text.len() - 8_192;
            while !p.text.is_char_boundary(cut) { cut += 1; }
            p.text.drain(..cut);
        }
        p.dirty = true;
    }

    fn discard_stream_attempt(&self, _run: &str, text_chars: u32, _reasoning_chars: u32, _segments: Vec<String>, tool_ids: Vec<String>) {
        let mut p = self.lock();
        crate::chat::protocol::truncate_stream_tail(&mut p.text, text_chars);
        p.records.retain(|record| !tool_ids.contains(&record.id)
            || matches!(record.status, ToolCallStatus::Success | ToolCallStatus::Error | ToolCallStatus::Cancelled | ToolCallStatus::Skipped));
        p.dirty = true;
    }

    fn emit_tool_record(&self, _conversation: &str, _run: &str, _message: &str, record: &ToolCallRecord) {
        if matches!(record.name.as_str(), "user_steer" | "user_follow_up") { return; }
        let mut p = self.lock();
        let old = p.records.iter().position(|item| item.id == record.id && item.round == record.round);
        let was_running = old.is_some_and(|i| p.records[i].status == ToolCallStatus::Running);
        let was_finished = old.is_some_and(|i| matches!(p.records[i].status, ToolCallStatus::Success | ToolCallStatus::Error | ToolCallStatus::Cancelled | ToolCallStatus::Skipped));
        if record.status == ToolCallStatus::Running && !was_running && !was_finished { p.tool_count += 1; }
        if matches!(record.status, ToolCallStatus::Success | ToolCallStatus::Error | ToolCallStatus::Cancelled | ToolCallStatus::Skipped) && !was_finished {
            p.recent_tools.insert(0, json!({"tool": record.name, "args": tool_args_preview(record), "intent": tool_intent(record), "isError": matches!(record.status, ToolCallStatus::Error | ToolCallStatus::Cancelled | ToolCallStatus::Skipped), "endMs": registry::now_ms()}));
            p.recent_tools.truncate(5);
        }
        if let Some(i) = old { p.records[i] = record.clone(); } else { p.records.push(record.clone()); }
        p.dirty = true;
    }

    fn request_tool_approval<'a>(&'a self, _ctx: &'a ToolExecutionContext<'a>, _record: &'a ToolCallRecord) -> AgentHostFuture<'a, bool> { Box::pin(async { true }) }
    fn request_session_consent<'a>(&'a self, _ctx: &'a ToolExecutionContext<'a>) -> AgentHostFuture<'a, bool> { Box::pin(async { true }) }
    fn request_user_response<'a>(&'a self, _ctx: &'a ToolExecutionContext<'a>, _record: &'a ToolCallRecord, _prompt: AskUserPromptPayload) -> AgentHostFuture<'a, AskUserResponseResult> {
        Box::pin(async { AskUserResponseResult { phase: "cancelled".into(), answers: HashMap::new() } })
    }
    fn is_generation_active(&self, conversation: &str, generation: u64) -> bool {
        let p = self.lock();
        !self.cancelled() && !p.hard_budget && !p.budget_stop
            && self.app.state::<AppState>().chat_runtime().is_generation_active(conversation, generation)
    }
    fn wait_for_generation_inactive<'a>(&'a self, conversation: &'a str, generation: u64) -> AgentHostFuture<'a, ()> {
        Box::pin(async move {
            let mut cancel = self.cancel.clone();
            if *cancel.borrow() { return; }
            let state = self.app.state::<AppState>();
            tokio::select! {
                _ = state.chat_runtime().wait_for_generation_inactive(conversation, generation) => {},
                _ = cancel.changed() => {},
            }
        })
    }
    fn take_steering_messages(&self, _conversation: &str) -> Vec<SteeringMessage> {
        let mut p = self.lock();
        let mut messages = Vec::new();
        if self.budget > 0 && p.requests >= self.budget && !p.notice_sent {
            p.notice_sent = true;
            messages.push(SteeringMessage { id: format!("budget-{}", self.job_id), text: budget_notice(p.requests, self.budget) });
        }
        messages
    }
}

struct Executor { app: AppHandle, host: Arc<Host> }

impl ToolExecutor for Executor {
    fn prepare_result<'a>(&'a self, ctx: &ToolExecutionContext<'_>, tool: &ChatToolDefinition, args: &Value, output: crate::mcp::types::McpToolCallResult) -> ToolExecutorFuture<'a> {
        crate::chat::artifacts::prepare_output(&self.app, ctx.tool_conversation_id, ctx.message_id, tool, args, output)
    }
    fn call<'a>(&'a self, ctx: &'a ToolExecutionContext<'a>, tool: &'a ChatToolDefinition, args: Value, skill_context: Option<&'a mut crate::skills::SkillRunContext>) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            if self.host.forced() { return Err("Request budget crossed; tools are disabled for the final report".into()); }
            let native_ctx = crate::mcp::registry::NativeToolContext::from_execution(ctx);
            if tool.name == super::WAIT_TOOL_NAME {
                let mut slot = self.host.permit.lock().await;
                let suspended = slot.take().map(task::Permit::release);
                let state = self.app.state::<AppState>();
                let mut cancel = self.host.cancel.clone();
                if *cancel.borrow() { return Err("cancelled".into()); }
                let result = tokio::select! {
                    result = crate::mcp::registry::call_tool(&self.app, &state, tool, args, skill_context, Some(native_ctx)) => result,
                    _ = cancel.changed() => return Err("cancelled".into()),
                    _ = state.chat_runtime().wait_for_generation_inactive(ctx.conversation_id, ctx.generation) => return Err("cancelled".into()),
                };
                if let Some(suspended) = suspended {
                    let max = state.settings_read().chat_tools.task_max_concurrency;
                    *slot = tokio::select! {
                        permit = suspended.reacquire(max, &mut cancel) => permit,
                        _ = state.chat_runtime().wait_for_generation_inactive(ctx.conversation_id, ctx.generation) => return Err("cancelled".into()),
                    };
                    if slot.is_none() { return Err("cancelled".into()); }
                }
                result
            } else {
                crate::mcp::registry::call_tool(&self.app, &self.app.state::<AppState>(), tool, args, skill_context, Some(native_ctx)).await
            }
        })
    }
}

struct Metered<'a> { state: &'a AppState, host: &'a Host }

impl ProviderRuntime for Metered<'_> {
    fn stream<'a>(&'a self, mut request: StreamRequest<'a>) -> ProviderFuture<'a, Result<ChatStreamOutput, ModelError>> {
        Box::pin(async move {
            let forced = self.host.forced();
            if !self.host.begin_request() { return Err(ModelError::with_kind("Subagent request budget exhausted", ModelErrorKind::Cancelled)); }
            if forced {
                request.tools = None;
                let reminder = budget_reminder();
                if !request.messages.iter().any(|message| message["content"].as_str().is_some_and(|text| text.contains(&reminder))) {
                    request.messages.push(json!({"role": "user", "content": reminder}));
                }
            }
            {
                let mut p = self.host.lock();
                p.history = request.messages.clone();
                p.text.clear();
                p.dirty = true;
            }
            let provider = request.provider;
            let model = request.model;
            let result = ProviderRuntime::stream(self.state, request).await;
            if let Ok(output) = &result {
                if !output.cancelled { self.host.finish_request(); }
                self.host.note_usage(provider, model, output.billing_usage.as_ref().or(output.usage.as_ref()), true);
                self.host.update_context_usage(provider, output.usage.as_ref());
                if !output.content.trim().is_empty() { self.host.lock().last_assistant = output.content.clone(); }
            }
            result
        })
    }
    fn message<'a>(&'a self, mut request: MessageRequest<'a>) -> ProviderFuture<'a, Result<(Value, Option<ModelUsage>), String>> {
        Box::pin(async move {
            let forced = self.host.forced();
            if forced {
                let reminder = budget_reminder();
                if !request.messages.iter().any(|message| message["content"].as_str().is_some_and(|text| text.contains(&reminder))) {
                    request.messages.push(json!({"role": "user", "content": reminder}));
                }
            }
            self.host.lock().history = request.messages.clone();
            let provider = request.provider;
            let model = request.model;
            let result = ProviderRuntime::message(self.state, request).await;
            if let Ok((_, usage)) = &result { self.host.note_usage(provider, model, usage.as_ref(), true); }
            result
        })
    }
    fn summarize<'a>(&'a self, request: SummaryRequest<'a>) -> ProviderFuture<'a, CompactOutcome> {
        ProviderRuntime::summarize(self.state, request)
    }
}

fn line(value: &Value, max: usize) -> Option<String> {
    let safe: String = value.as_str()?.chars().filter(|ch| !ch.is_control() || ch.is_whitespace()).collect();
    let flattened = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.is_empty() { return None; }
    if flattened.chars().count() <= max { Some(flattened) } else { Some(format!("{}…", flattened.chars().take(max - 1).collect::<String>().trim_end())) }
}

fn parse_probe(text: &str) -> Option<(u32, Option<String>, Option<String>)> {
    let mut stage = None;
    let mut watch = None;
    for (start, _) in text.match_indices('{') {
        let Some(Ok(value)) = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>().next() else { continue };
        stage = stage.or_else(|| line(&value["stage"], 80));
        watch = watch.or_else(|| line(&value["watch"], 200));
        let pct = value.get("pct").or_else(|| value.get("percent")).and_then(|pct| {
            pct.as_f64().or_else(|| pct.as_str().and_then(|s| s.trim().trim_end_matches('%').trim().parse().ok()))
        });
        if let Some(pct) = pct.filter(|pct| pct.is_finite()) {
            return Some((pct.round().clamp(0.0, 100.0) as u32, stage, watch));
        }
    }
    for (end, _) in text.match_indices('%') {
        let prefix = text[..end].trim_end();
        let start = prefix.char_indices().rev().find(|(_, ch)| !ch.is_ascii_digit() && *ch != '.')
            .map_or(0, |(index, ch)| index + ch.len_utf8());
        if let Ok(pct) = prefix[start..].parse::<f64>() {
            if pct.is_finite() { return Some((pct.round().clamp(0.0, 100.0) as u32, stage, watch)); }
        }
    }
    None
}

async fn completion_probes(host: Arc<Host>, provider: ModelProvider, model: String, message_id: String) {
    let delays = [2, 5, 10, 30, 60];
    let mut previous: Option<(u32, Option<String>, Instant)> = None;
    let mut step = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(60 * delays[step.min(delays.len() - 1)])).await;
        step += 1;
        if host.cancelled() { return; }
        let (mut history, inflight, partial) = {
            let p = host.lock();
            let inflight: Vec<Value> = p.records.iter().filter(|record| record.status == ToolCallStatus::Running).map(|record| json!({"name": record.name, "path": tool_args_preview(record), "chars": record.arguments.chars().count()})).collect();
            (p.history.clone(), inflight, p.text.clone())
        };
        if history.is_empty() { continue; }
        if !partial.trim().is_empty() {
            history.push(json!({"role": "assistant", "content": partial}));
        }
        let prior = previous.as_ref().map(|(percent, stage, time)| json!({"percent": percent, "stage": stage, "ago": duration_text(time.elapsed().as_millis() as u64)}));
        let prompt = prompts::render(prompts::COMPLETION_PROBE, &json!({"inflight": inflight, "previous": prior}));
        history.push(json!({"role": "user", "content": prompt}));
        let state = host.app.state::<AppState>();
        let result = ProviderRuntime::message(&*state, MessageRequest {
            provider: &provider, model: &model, messages: history, retry_attempts: 1,
            thinking_enabled: false, thinking_level: None, builtin_web_search: false, max_output_tokens: 512,
            conversation_id: &host.record.child_conversation_id, message_id: &message_id, label: "subagent completion probe",
        }).await;
        if let Ok((message, usage)) = result {
            host.note_usage(&provider, &model, usage.as_ref(), false);
            if let Some((percent, stage, watch)) = parse_probe(&crate::chat::agent::stop::assistant_content_from_api_message(&message)) {
                previous = Some((percent, stage.clone(), Instant::now()));
                let mut p = host.lock();
                p.completion_percent = Some(percent); p.stage = stage; p.watch = watch; p.dirty = true;
            }
        }
    }
}

fn duration_text(ms: u64) -> String {
    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if ms < 1_000 { return format!("{ms}ms"); }
    if ms < MINUTE {
        let tenths = (ms + 50) / 100;
        return if tenths < 600 { format!("{}.{:01}s", tenths / 10, tenths % 10) } else { "1m".into() };
    }
    if ms < HOUR {
        let minutes = ms / MINUTE;
        let seconds = (ms % MINUTE) / 1_000;
        return if seconds == 0 { format!("{minutes}m") } else { format!("{minutes}m{seconds}s") };
    }
    if ms < DAY {
        let hours = ms / HOUR;
        let minutes = (ms % HOUR) / MINUTE;
        return if minutes == 0 { format!("{hours}h") } else { format!("{hours}h{minutes}m") };
    }
    let days = ms / DAY;
    let hours = (ms % DAY) / HOUR;
    if hours == 0 { format!("{days}d") } else { format!("{days}d{hours}h") }
}

fn size_text(bytes: usize) -> String {
    if bytes < 1_024 { format!("{bytes}B") }
    else if bytes < 1_048_576 { format!("{:.1}KB", bytes as f64 / 1_024.0) }
    else if bytes < 1_073_741_824 { format!("{:.1}MB", bytes as f64 / 1_048_576.0) }
    else { format!("{:.1}GB", bytes as f64 / 1_073_741_824.0) }
}

async fn write_artifact(path: &std::path::Path, data: &[u8]) -> Result<(), std::io::Error> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    if let Err(error) = tokio::fs::write(&temporary, data).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error);
    }
    Ok(())
}


fn escape_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn result_summary(id: &str, agent: &str, status: &str, output: &str, error: Option<&str>, duration_ms: u64, transcript_available: bool) -> String {
    let raw_output = output;
    let has_output = !raw_output.trim().is_empty();
    let output = raw_output.trim();
    let output = if output.is_empty() { error.filter(|s| !s.trim().is_empty()).unwrap_or("(no output)") } else { output };
    let count = if has_output { raw_output.chars().count() } else { output.chars().count() };
    let lines = if has_output { raw_output.split('\n').count() } else { output.lines().count().max(1) };
    let slice: String = output.chars().take(PREVIEW_CHARS).collect();
    let preview = if let Some(at) = slice.rfind('\n').filter(|at| slice[..*at].chars().count() >= PREVIEW_CHARS / 2) { &slice[..at] } else { &slice };
    let aborted = status == "cancelled";
    let shown_error = if !aborted && has_output { error } else { None };
    let render_summary = |truncated| prompts::render(prompts::TASK_SUMMARY, &json!({
        "id": escape_attribute(id), "agentName": escape_attribute(agent), "status": status, "duration": duration_text(duration_ms),
        "truncated": truncated, "meta": {"lineCount": lines, "charSize": size_text(count)},
        "preview": delivery::escape_harness_tags(if truncated { preview } else { output }),
        "abortReason": if aborted { error.map(delivery::escape_harness_tags) } else { None },
        "resumable": false, "error": shown_error.map(delivery::escape_harness_tags), "mergeSummary": "",
    }));
    let mut summary = render_summary(count > INLINE_CHARS);
    if count <= INLINE_CHARS && count > PREVIEW_CHARS && summary.chars().count() > 12_000 {
        summary = render_summary(true);
    }
    let follow_up = prompts::render(prompts::TASK_FOLLOW_UP, &json!({
        "agentId": id, "aborted": aborted, "resumable": false, "ircEnabled": false, "transcriptAvailable": transcript_available,
    }));
    format!("{}\n\n{}", summary.trim(), follow_up.trim())
}

fn salvage(text: &str, requests: u32, tokens: u64) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() { return String::new(); }
    let clipped = if flat.chars().count() > 500 { format!("{}…", flat.chars().take(499).collect::<String>()) } else { flat };
    format!("[cancelled after {requests} req, {tokens} tok — last activity: \"{clipped}\"]")
}

fn prepare_worker_prompt(
    language: &str,
    thinking_enabled: bool,
    registry: &crate::skills::SkillRegistry,
    settings: &crate::settings::ChatToolsConfig,
    tools: &[ChatToolDefinition],
    persona: &str,
    workdir: Option<&str>,
) -> crate::chat::agent::prepare::PreparedChatPrompt {
    let names = available_builtin_tool_names(tools);
    let mut prompt = build_chat_prompt(language, false, thinking_enabled, registry, settings, !tools.is_empty(), &names,
        None, None, None, None, "", false, None, None, None, None, workdir, None, &[]);
    prompt.system.push_str("\n\n");
    prompt.system.push_str(persona);
    prompt
}

async fn execute(app: &AppHandle, host: &Arc<Host>, definition: &AgentDefinition, context: &str, parent_provider: &str, parent_model: &str, generation: u64) -> Result<String, String> {
    let state = app.state::<AppState>();
    let state: &AppState = &state;
    let mut settings = state.settings_read().clone();
    settings.chat_tools.approval_policy = "auto".into();
    let (provider, model, role_thinking) = resolve_model(&settings, parent_provider, parent_model, definition.model.as_deref())?;
    let requested_thinking = definition.thinking_level.as_deref().or(role_thinking.as_deref());
    let (thinking_enabled, thinking_level) = crate::chat::commands::reasoning::resolve_thinking(
        requested_thinking, settings.chat.thinking_enabled, Some(&provider), &model,
    )?;
    *host.model.lock().unwrap_or_else(|e| e.into_inner()) = model.clone();
    host.lock().context_window = crate::chat::model_metadata::context_window_for_model(Some(&provider), &model).0;
    let cwd = crate::chat::storage::load_conversation(app, &host.record.root).ok().and_then(|conversation| {
        crate::chat::storage::resolve_conversation_working_directory(app, &conversation, &settings.chat_tools.native_tools.working_directory).ok()
    });
    let resources = state.chat_runtime().resource_catalog(app, &host.record.root, &settings.chat_tools.skill_scan_paths, cwd.as_deref(), false).await?;
    let catalog = resources.skills.clone();
    let selected = definition.skills.clone();
    let loaded = tauri::async_runtime::spawn_blocking(move || catalog.read_selected(&selected)).await.map_err(|e| e.to_string())??;
    let mut persona = definition.system_prompt.clone();
    for id in &definition.skills {
        if let Some(skill) = loaded.find(id).filter(|skill| !skill.body.trim().is_empty()) {
            persona.push_str(&format!("\n\nPreloaded Skill ({}):\n{}", skill.meta.name, skill.body));
        }
    }
    let persona = prompts::render(prompts::SUBAGENT_SYSTEM, &json!({"agent": persona, "context": context, "ircSelfId": "", "structured": false}));
    let mut tools = crate::mcp::registry::list_enabled_tool_catalog(app, state).await.tools;
    super::append_tool_definitions(&mut tools, &settings, &resources.agents);
    if let Some(tool) = tools.iter_mut().find(|tool| tool.name == super::TASK_TOOL_NAME) {
        *tool = task::tool_definition_with_policy(&resources.agents, &settings, definition.spawns.as_deref());
    }
    crate::chat::agent::filter::filter_tools_for_agent(&mut tools, definition, host.record.depth, settings.chat_tools.task_max_recursion_depth);
    let effective = settings.chat_tools.clone();
    let language = settings.settings_language.clone().unwrap_or_else(|| "zh".into());
    let workdir = cwd.as_ref().map(|path| path.to_string_lossy().into_owned());
    let prompt = prepare_worker_prompt(&language, thinking_enabled, &resources.skills, &effective, &tools, &persona, workdir.as_deref());
    let assignment = prompts::render(prompts::SUBAGENT_USER, &json!({"assignment": host.task.trim()}));
    let initial = vec![json!({"role": "system", "content": prompt.system}), json!({"role": "user", "content": assignment})];
    host.save_history(&initial).await?;
    host.lock().history = initial.clone();
    let probe = if host.record.depth == 1 {
        Some(tokio::spawn(completion_probes(host.clone(), provider.clone(), model.clone(), format!("submsg-{}", host.job_id))))
    } else { None };
    let metered = Metered { state, host };
    let executor = Executor { app: app.clone(), host: host.clone() };
    let max_output_tokens = settings.chat.max_output_tokens;
    let retry_attempts = if settings.retry_enabled { settings.retry_attempts as usize } else { 1 };
    let mut config = AgentRunConfig {
        provider_runtime: &metered, conversation_id: host.record.child_conversation_id.clone(),
        tool_conversation_id: host.record.root.clone(), depth: host.record.depth,
        run_id: host.record.run_id.clone().unwrap_or_else(|| format!("subrun-{}", host.job_id)),
        message_id: format!("submsg-{}", host.job_id), generation, provider, model, runtime_messages: initial,
        runtime_context: Some(prompt.context.clone()), tools, blocked_tool_calls: Vec::new(), settings,
        effective_chat_tools: effective, language, thinking_enabled, thinking_level,
        web_search_mode: WebSearchMode::Off, max_output_tokens, retry_attempts, assistant_snapshot: None,
        provider_tools_fallback_prompt: prompt, initial_cache_usage: None, initial_anchor_total_tokens: None,
        initial_anchor_trailing_estimate: 0, skill_registry: resources.skills.clone(), todo_state: Default::default(),
    };
    let outcome = async {
        loop {
            let attempt = run_agent_loop(config.clone(), host.as_ref(), &executor).await;
            let budget_stop = {
                let mut progress = host.lock();
                if progress.budget_stop && !progress.hard_budget && !host.cancelled() {
                    progress.budget_stop = false;
                    progress.budget_wrapping = true;
                    progress.records.retain(|record| !matches!(record.status, ToolCallStatus::Pending | ToolCallStatus::Running));
                    true
                } else {
                    false
                }
            };
            if budget_stop {
                let mut history = match &attempt {
                    Ok(result) => result.compacted_history.clone().unwrap_or_else(|| {
                        let mut history = config.runtime_messages.clone();
                        history.extend(result.api_messages.iter().cloned());
                        history
                    }),
                    Err(_) => host.lock().history.clone(),
                };
                if !host.is_generation_active(&config.conversation_id, generation) {
                    return Err("cancelled".into());
                }
                host.wait_children(&config.conversation_id, generation).await?;
                let incoming = host.checkpoint_runtime(&config.conversation_id, &config.run_id, &history, false).await?;
                history.extend(incoming);
                history.push(json!({"role": "user", "content": budget_reminder()}));
                host.lock().history = history.clone();
                host.save_history(&history).await?;
                config.runtime_messages = history;
                config.runtime_context = None;
                config.tools.clear();
                continue;
            }
            let result = attempt.map_err(|error| {
                if error.contains("empty assistant response") { EMPTY_FINAL.to_string() } else { error }
            })?;
            let mut history = result.compacted_history.clone().unwrap_or_else(|| {
                let mut history = config.runtime_messages.clone(); history.extend(result.api_messages.iter().cloned()); history
            });
            if result.stream_outcome != "completed" { host.lock().history = history; return Err("cancelled".into()); }
            if let Some(degraded) = &result.degraded {
                host.lock().history = history;
                if degraded.kind == "empty_response" { return Err(EMPTY_FINAL.into()); }
                let detail = degraded.detail.as_deref().filter(|detail| !detail.trim().is_empty());
                return Err(detail.map_or_else(|| degraded.reason.clone(), |detail| format!("{}: {detail}", degraded.reason)));
            }
            if !result.content.trim().is_empty() {
                let already = history.last().is_some_and(|message| message["role"] == "assistant"
                    && message["tool_calls"].as_array().is_none_or(Vec::is_empty)
                    && crate::chat::model::openai_message_text_content(message).is_some_and(|text| text.trim() == result.content.trim()));
                if !already { history.push(json!({"role": "assistant", "content": result.content})); }
            }
            host.lock().history = history.clone();
            host.save_history(&history).await?;
            host.wait_children(&config.conversation_id, generation).await?;
            let incoming = host.checkpoint_runtime(&config.conversation_id, &config.run_id, &history, true).await?;
            if !incoming.is_empty() {
                history.extend(incoming); config.runtime_messages = history; config.runtime_context = None; continue;
            }
            let last = history.iter().rev().find(|message| message["role"] == "assistant");
            let output = last.filter(|message| message["tool_calls"].as_array().is_none_or(Vec::is_empty))
                .and_then(crate::chat::model::openai_message_text_content).unwrap_or_default();
            return if output.trim().is_empty() { Err(EMPTY_FINAL.into()) } else { Ok(output) };
        }
    }.await;
    if let Some(probe) = probe { probe.abort(); let _ = probe.await; }
    outcome
}

pub(crate) async fn run(app: AppHandle, req: Request) {
    let state = app.state::<AppState>();
    if *req.cancel_rx.borrow() {
        state.sub_agents.registry.update(&req.record.root, &req.record.id, |record| {
            if record.generation == req.record.generation {
                record.status = registry::AgentStatus::Idle;
                record.run_id = None;
                record.activity = None;
            }
        });
        crate::chat::protocol::emit_run_event_or_live(&app, &req.record.root, &req.parent_run_id,
            crate::chat::protocol::ChatRunEvent::SubagentUpdated {
                parent_tool_call_id: req.parent_tool_call_id.clone(), task_id: req.record.id.clone(),
                name: req.record.display_name.clone(), model: None, depth: req.record.depth,
                status: "aborted".into(), preview: None, steps: Vec::new(),
            });
        return;
    }
    let settings = state.settings_read().clone();
    let mut record = req.record.clone();
    let run_id = format!("subrun-{}-{}", record.generation, req.job_id);
    let registered = state.sub_agents.registry.update(&record.root, &record.id, |current| {
        if current.generation == record.generation { current.run_id = Some(run_id.clone()); }
    }).is_some_and(|current| current.generation == record.generation);
    if !registered { return; }
    record.run_id = Some(run_id);
    let generation = state.chat_runtime().begin_generation(&record.child_conversation_id);
    let host = Arc::new(Host {
        app: app.clone(), record, job_id: req.job_id.clone(), index: req.index,
        agent: req.definition.name.clone(), task: req.task.clone(), parent_run_id: req.parent_run_id,
        parent_tool_call_id: req.parent_tool_call_id, model: Mutex::new(req.parent_model.clone()), started: Instant::now(),
        cancel: req.cancel_rx, budget: budget_for(&req.definition.name, settings.chat_tools.task_soft_request_budget),
        permit: tokio::sync::Mutex::new(Some(req.permit)),
        progress: Mutex::new(Progress { status: "running", dirty: true, ..Default::default() }),
    });
    host.emit_progress(true);
    let progress_host = host.clone();
    let updates = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(150));
        loop { interval.tick().await; progress_host.emit_progress(false); }
    });
    let initial_file = tokio::fs::write(&host.record.session_file, "").await.map_err(|e| format!("Cannot create subagent transcript: {e}"));
    let outcome = match initial_file {
        Ok(()) => execute(&app, &host, &req.definition, &req.context, &req.parent_provider_id, &req.parent_model, generation).await,
        Err(error) => Err(error),
    };
    updates.abort();
    let _ = updates.await;
    let (requests, tokens, hard, tail, history) = {
        let p = host.lock();
        let tail = if p.text.trim().is_empty() { p.last_assistant.clone() } else { p.text.clone() };
        (p.requests, p.tokens, p.hard_budget, tail, p.history.clone())
    };
    let cancelled = host.cancelled() || outcome.as_ref().err().is_some_and(|error| error == "cancelled");
    let mut aborted = cancelled || hard;
    let mut error = outcome.as_ref().err().cloned();
    if hard { error = Some(format!("Soft request budget exceeded ({requests} requests; budget {}) — agent did not report when force-stopped", host.budget)); }
    else if cancelled { error = Some("Cancelled by caller".into()); }
    let mut output = outcome.unwrap_or_default();
    if error.as_deref() == Some(EMPTY_FINAL) { output = EMPTY_FINAL.into(); }
    if aborted && output.trim().is_empty() { output = salvage(&tail, requests, tokens); }
    let transcript_available = match host.save_history(&history).await {
        Ok(()) => true,
        Err(err) => { if !aborted { error = Some(err); } false }
    };
    if host.cancelled() && !aborted {
        aborted = true;
        error = Some("Cancelled by caller".into());
        if output.trim().is_empty() { output = salvage(&tail, requests, tokens); }
    }
    if let Err(err) = write_artifact(&host.record.output_file, output.as_bytes()).await {
        if !aborted { error = Some(format!("Cannot write subagent output: {err}")); }
    }
    let status = if aborted { "cancelled" } else if error.is_some() { "failed (exit 1)" } else { "completed" };
    let summary = result_summary(&host.record.id, &host.agent, status, &output, error.as_deref(), host.started.elapsed().as_millis() as u64, transcript_available);
    {
        let mut p = host.lock();
        p.status = if aborted { "aborted" } else if error.is_some() { "failed" } else { "completed" };
        p.dirty = true;
    }
    host.emit_progress(true);
    drop(host.permit.lock().await.take());
    let state = app.state::<AppState>();
    state.chat_runtime().cancel_conversation(&host.record.child_conversation_id);
    state.sub_agents.registry.update(&host.record.root, &host.record.id, |record| {
        if record.generation == host.record.generation {
            record.status = registry::AgentStatus::Idle; record.run_id = None; record.activity = None;
        }
    });
    if error.is_some() || aborted { state.sub_agents.jobs.settle(&host.job_id, Err(summary)); }
    else { state.sub_agents.jobs.settle(&host.job_id, Ok(summary)); }
    if !aborted && state.sub_agents.jobs.get(&host.job_id)
        .is_some_and(|job| job.status == super::jobs::JobStatus::Cancelled)
    {
        if output.trim().is_empty() {
            let recovered = salvage(&tail, requests, tokens);
            if !recovered.is_empty() {
                if let Err(err) = write_artifact(&host.record.output_file, recovered.as_bytes()).await {
                    eprintln!("Cannot save cancelled subagent output: {err}");
                }
            }
        }
        host.lock().status = "aborted";
        host.emit_progress(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_tokens_exclude_repeated_cache_reads() {
        let usage = ModelUsage {
            input_tokens: Some(1_000), output_tokens: Some(100), cached_input_tokens: Some(900),
            ..Default::default()
        };
        assert_eq!(progress_tokens(&usage, "openai_chat"), 200);
        assert_eq!(progress_tokens(&usage, "anthropic_messages"), 1_100);
    }

    #[test]
    fn budgets_use_ceiling_and_agent_caps() {
        assert_eq!(budget_for("scout", 200), 100);
        assert_eq!(budget_for("sonic", 40), 40);
        assert_eq!(budget_for("reviewer", 300), 300);
        assert_eq!(budget_for("scout", 0), 0);
        assert_eq!(force_at(101), 152);
        assert_eq!(force_at(200).saturating_add(5), 305);
        assert!(budget_notice(100, 100).contains("At 150 requests"));
        let mut progress = Progress::default();
        for _ in 0..149 { count_assistant_request(&mut progress, 100); }
        assert!(!progress.budget_stop);
        count_assistant_request(&mut progress, 100);
        assert!(progress.budget_stop);
        progress.budget_stop = false;
        progress.budget_wrapping = true;
        for _ in 0..4 { count_assistant_request(&mut progress, 100); }
        assert!(!progress.hard_budget);
        count_assistant_request(&mut progress, 100);
        assert!(progress.hard_budget);
        let mut unlimited = Progress::default();
        for _ in 0..400 { count_assistant_request(&mut unlimited, 0); }
        assert!(!unlimited.budget_stop && !unlimited.hard_budget);
    }

    #[test]
    fn worker_prompt_keeps_default_and_rendered_persona() {
        let settings = Settings::default();
        let persona = prompts::render(prompts::SUBAGENT_SYSTEM, &json!({
            "agent": "A focused persona", "context": "Shared project context", "structured": false, "ircSelfId": ""
        }));
        let prompt = prepare_worker_prompt("en", true, &crate::skills::SkillRegistry::default(), &settings.chat_tools, &[], &persona, None);
        let default_prompt = crate::settings::default_chat_system_prompt(false);
        assert!(prompt.system.contains(default_prompt.as_str()));
        assert!(prompt.system.contains("A focused persona"));
        assert!(prompt.system.contains("Shared project context"));
    }

    #[test]
    fn transcript_pairs_reused_tool_ids_in_order_and_after_compaction() {
        let record = |round, status| serde_json::from_value::<ToolCallRecord>(
            json!({"id": "reused", "name": "read", "round": round, "status": status})
        ).unwrap();
        let records = vec![record(1, "success"), record(2, "error"), record(3, "running")];
        let mut history = vec![
            json!({"role": "tool", "tool_call_id": "reused", "content": "first"}),
            json!({"role": "tool", "tool_call_id": "reused", "content": "second"}),
        ];
        annotate_tool_results(&mut history, &records);
        assert_eq!(history[0]["is_error"], false);
        assert_eq!(history[1]["is_error"], true);
        let mut compacted = vec![json!({"role": "tool", "tool_call_id": "reused"})];
        annotate_tool_results(&mut compacted, &records);
        assert_eq!(compacted[0]["is_error"], true);
    }

    #[test]
    fn summaries_show_status_and_bounded_preview() {
        let short = result_summary("Alpha", "scout", "completed", "found it", None, 61_000, true);
        assert!(short.contains("status=\"completed\""));
        assert!(short.contains("<output>\nfound it\n</output>"));
        assert!(short.contains("duration=\"1m1s\""));
        assert_eq!(duration_text(123), "123ms");
        assert_eq!(duration_text(1_500), "1.5s");
        assert_eq!(duration_text(59_950), "1m");
        assert_eq!(size_text(1_536), "1.5KB");
        let long = format!("{}\n{}", "a".repeat(3_000), "b".repeat(9_000));
        let summary = result_summary("Alpha", "scout", "failed (exit 1)", &long, Some("provider failed"), 500, true);
        assert!(summary.contains("status=\"failed (exit 1)\""));
        assert!(summary.contains("full-output=\"agent://Alpha\""));
        assert!(summary.contains("<error>provider failed</error>"));
        assert!(!summary.contains(&"b".repeat(1_000)));
        let cancelled = result_summary("Alpha", "task", "cancelled", "partial", Some("Cancelled by caller"), 0, true);
        assert!(cancelled.contains("<abort-reason>Cancelled by caller"));
        assert!(cancelled.contains("history://Alpha"));
        let escaped = result_summary("Alpha", "scout\"<irc>", "completed", "ok", None, 0, true);
        assert!(escaped.contains("agent=\"scout&quot;&lt;irc&gt;\""));
    }

    #[test]
    fn probe_bounds_and_salvage_are_safe() {
        let estimate = parse_probe("```json\n{\"pct\": 140, \"stage\": \"working\\n now\", \"watch\": \"x\"}\n```").unwrap();
        assert_eq!(estimate, (100, Some("working now".into()), Some("x".into())));
        assert_eq!(parse_probe("header {bad} then {\"pct\":\"-4%\"}").unwrap().0, 0);
        assert_eq!(parse_probe("roughly 45%").unwrap().0, 45);
        assert_eq!(parse_probe("进度45%").unwrap().0, 45);
        assert!(salvage("first\nsecond", 2, 10).contains("first second"));
        assert!(delivery::escape_harness_tags("<task-result> x <system-reminder>").contains("&lt;task-result>"));
    }
}
