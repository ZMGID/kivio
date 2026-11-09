use crate::chat::agent::host::{AgentHost, AgentHostFuture, SpeculativeTaskScope};
use crate::chat::agent::{
    content::{AgentToolCall, AgentToolStatus},
    execute::{
        limit_tool_text_for_model, PreparedToolCall, ToolExecutorFuture, ToolFuture,
        ToolPreparation, ToolProgress, ToolResultUpdate,
    },
};
use crate::chat::agent_preparation::{
    builtin_tool_bypasses_approval, disabled_builtin_tool_feedback,
};
pub(crate) use crate::chat::tools::schema::coerce_arguments as coerce_tool_arguments;
use crate::chat::{ask_user, model::PendingToolCall, tools::ArgumentPolicy};
use crate::mcp::ChatToolDefinition;
use crate::settings::Settings;
use serde_json::Value;
use std::{sync::Arc, time::Instant};

pub trait ToolDispatcher: Send + Sync {
    fn call<'a>(
        &'a self,
        progress: &'a ToolProgress,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a>;
}

pub struct ToolExecutionContext<'a> {
    pub run_tools: &'a [ChatToolDefinition],
    pub conversation_id: &'a str,
    pub run_id: &'a str,
    pub message_id: &'a str,
    /// Provider and model of the run executing this tool, including answer arms.
    pub provider: &'a crate::settings::ModelProvider,
    pub model: &'a str,
    pub generation: u64,
    pub round: u32,
    /// Sub-agent nesting depth: 0 for a top-level run; used to limit task recursion.
    pub depth: u8,
    /// Conversation used by conversation-scoped tools; it can differ from
    /// `conversation_id` in a sub-agent run.
    pub tool_conversation_id: &'a str,
    pub plan_state: &'a crate::chat::types::AgentPlanState,
    pub mcp_tool_ids: &'a [(String, String)],
    pub mcp_server_ids: &'a [String],
    pub approval_policy: &'a str,
    pub interactive_host: bool,
    /// The model-issued tool_call id of the call currently executing. Sub-agent
    /// progress is reported back onto the parent tool card addressed by this id.
    pub tool_call_id: &'a str,
}

pub trait ApplicationHost: AgentHost {
    fn collect_runtime_inputs<'a>(
        &'a self,
        _context: kivio_agent_sdk::agent::InjectionContext<'a>,
        _admission: &'a mut kivio_agent_sdk::agent::InjectionAdmission,
    ) -> kivio_agent_sdk::agent::InjectionFuture<'a, Result<(), kivio_agent_sdk::agent::InjectionError>> {
        Box::pin(async { Ok(()) })
    }

    fn compaction_threshold(&self, _context_window: usize) -> Option<usize> {
        None
    }

    /// Consecutive automatic compaction failures remembered for the conversation, so the
    /// ZCode circuit breaker outlives one run. Default: nothing is remembered between runs.
    fn auto_compact_failures(&self, _conversation_id: &str) -> u32 {
        0
    }

    fn set_auto_compact_failures(&self, _conversation_id: &str, _failures: u32) {}

    fn tool_execution_hooks_active(&self) -> bool {
        false
    }
    fn supports_interactive_approval(&self) -> bool {
        true
    }
    fn workflow_hooks(&self) -> Option<&crate::chat::workflow_hooks::Runtime> {
        None
    }
    fn start_speculation(
        &self,
        _settings: &Settings,
        _tools: &[ChatToolDefinition],
        _ctx: &ToolExecutionContext<'_>,
    ) -> Option<Box<dyn SpeculativeTaskScope>> {
        None
    }
    fn request_tool_approval<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        record: &'a AgentToolCall,
    ) -> AgentHostFuture<'a, bool>;
    fn request_session_consent<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
    ) -> AgentHostFuture<'a, bool> {
        Box::pin(async { false })
    }
    fn request_user_response<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        record: &'a AgentToolCall,
        prompt: ask_user::AskUserPromptPayload,
    ) -> AgentHostFuture<'a, ask_user::AskUserResponseResult>;
}

struct ApplicationToolContext<'a> {
    host: &'a dyn ApplicationHost,
    control: crate::chat::agent::control::AgentRunHandle,
    progress: &'a ToolProgress,
}
impl<'a> std::ops::Deref for ApplicationToolContext<'a> {
    type Target = dyn ApplicationHost + 'a;
    fn deref(&self) -> &Self::Target {
        self.host
    }
}
impl ApplicationToolContext<'_> {
    async fn tool_updated(&self, record: &AgentToolCall) {
        self.progress.updated(record).await;
    }
}

async fn before_tool_call(
    host: &ApplicationToolContext<'_>,
    settings: &Settings,
    ctx: &ToolExecutionContext<'_>,
    tool: &ChatToolDefinition,
    call: PendingToolCall,
    mut record: AgentToolCall,
) -> Result<ToolPreparation, String> {
    if let Some(error) = call.arguments_parse_error.clone() {
        return Ok(schema_rejection(record, error, tool));
    }
    let task_preflight = tool.argument_policy() == ArgumentPolicy::ToolAfterApproval;
    let mut call = call;
    if tool.source == "native" {
        crate::mcp::types::inject_legacy_tool_action(&call.function_name, &mut call.arguments);
    }
    if !task_preflight {
        call.arguments =
            coerce_tool_arguments(&tool.input_schema, std::mem::take(&mut call.arguments));
    }
    record.arguments = call.arguments.to_string();

    if let Err(error) = validate_read_only_action(tool, &call.arguments, ctx.plan_state) {
        return Ok(action_rejection(record, error));
    }

    let mut hook_context = Vec::new();
    let mut hook_approval_reason = None;
    if let Some(package_id) = tool
        .server_id
        .as_deref()
        .and_then(|id| id.strip_prefix("plugin-package-"))
        .and_then(|rest| rest.get(..36))
    {
        if !crate::plugins::packages::owner_enabled(package_id) {
            record.status = AgentToolStatus::Skipped;
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some("Plugin has been disabled".into());
            return Ok(ToolPreparation::Complete(
                record,
                "Plugin has been disabled".into(),
                Vec::new(),
            ));
        }
    }
    // Task batches own validation after approval; all other tools validate before policy hooks.
    if !task_preflight {
        if let Err(error) = validate_tool_arguments(tool, &call.arguments) {
            return Ok(schema_rejection(record, error, tool));
        }
    }
    if let Some(runtime) = host.workflow_hooks().filter(|r| !r.hooks.is_empty()) {
        let mut input = runtime.input("PreToolUse", ctx.conversation_id);
        input["tool_name"] = serde_json::json!(tool.name);
        input["tool_input"] = call.arguments.clone();
        input["tool_use_id"] = serde_json::json!(call.id);
        let outcome = crate::chat::workflow_hooks::run_for_host(
            host.host,
            host.control.clone(),
            "PreToolUse",
            input,
        )
        .await;
        let failure = match outcome {
            Ok(out) => {
                if let Some(updated) = out.updated_input {
                    call.arguments = updated;
                    call.arguments_raw = call.arguments.to_string();
                    record.arguments = call.arguments_raw.clone();
                    if !task_preflight {
                        if let Err(error) = validate_tool_arguments(tool, &call.arguments) {
                            return Ok(schema_rejection(record, error, tool));
                        }
                    }
                }
                hook_context = out.context;
                hook_approval_reason = out.ask;
                out.denied
            }
            Err(err) => return Err(err),
        };
        if let Some(reason) = failure {
            record.status = if host.control.active() {
                AgentToolStatus::Skipped
            } else {
                AgentToolStatus::Cancelled
            };
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some(reason.clone());
            return Ok(ToolPreparation::Complete(
                record,
                format!("Workflow hook: {reason}"),
                Vec::new(),
            ));
        }
    }

    if let Err(error) = validate_read_only_action(tool, &call.arguments, ctx.plan_state) {
        return Ok(action_rejection(record, error));
    }
    let requires_approval = tool_call_requires_approval(settings, tool, &call.arguments);
    let interactive_approval = host.supports_interactive_approval();
    let asks_user = tool.source == "native" && ask_user::is_ask_user_tool_name(&tool.name);
    if !interactive_approval && (hook_approval_reason.is_some() || requires_approval || asks_user) {
        let reason = "多答模式无法安全审批；请改用单模型执行，或在设置中显式选择自动授权。Interactive approval is unavailable in parallel answers; use single-model mode or explicitly select automatic authorization.";
        record.status = AgentToolStatus::Skipped;
        record.duration_ms = Some(0);
        record.completed_at = Some(chrono::Local::now().timestamp());
        record.error = Some(reason.into());
        return Ok(ToolPreparation::Complete(record, reason.into(), Vec::new()));
    }

    let hook_approved = if let Some(reason) = hook_approval_reason {
        record.output = Some(reason);
        if !host.request_tool_approval(ctx, &record).await {
            record.status = AgentToolStatus::Skipped;
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some("Workflow hook approval was declined".into());
            return Ok(ToolPreparation::Complete(
                record,
                "Workflow hook approval was declined".into(),
                Vec::new(),
            ));
        }
        true
    } else {
        false
    };
    if tool.source == "native" && ask_user::is_ask_user_tool_name(&tool.name) {
        let (record, content) =
            execute_ask_user_call(host, settings, ctx, record, call.arguments.clone()).await;
        return Ok(ToolPreparation::Complete(record, content, Vec::new()));
    }

    // Gate. The file/shell tools (read/write/edit/bash/grep/find/ls) are
    // governed by a single per-conversation **session consent**: prompt once,
    // then run freely with full-disk access for the rest of the conversation.
    // `approval_policy`: "auto" implicitly consents (no prompt); "always_confirm"
    // confirms each call (the per-call prompt IS the gate — no separate session
    // consent); the legacy read-only policy prompts once per conversation. Everything else (MCP
    // tools, etc.) keeps the existing per-call approval.
    let skip = |record: &mut AgentToolCall, reason: &str| {
        record.status = AgentToolStatus::Skipped;
        record.completed_at = Some(chrono::Local::now().timestamp());
        record.error = Some(reason.to_string());
    };

    if !hook_approved
        && interactive_approval
        && crate::chat::agent_preparation::tool_requires_session_consent(tool)
    {
        let policy = settings.chat_tools.approval_policy.as_str();
        if policy == "always_confirm" {
            if !host.request_tool_approval(ctx, &record).await {
                skip(&mut record, "Tool call was not approved");
                let content = record.error.clone().unwrap_or_default();
                return Ok(ToolPreparation::Complete(record, content, Vec::new()));
            }
        } else if policy != "auto" && !host.request_session_consent(ctx).await {
            skip(&mut record, "用户未授权本会话使用文件 / 命令工具");
            let content = record.error.clone().unwrap_or_default();
            return Ok(ToolPreparation::Complete(record, content, Vec::new()));
        }
    } else if !hook_approved && requires_approval {
        let approved = host.request_tool_approval(ctx, &record).await;
        if !approved {
            skip(&mut record, "Tool call was not approved");
            let content = record.error.clone().unwrap_or_default();
            return Ok(ToolPreparation::Complete(record, content, Vec::new()));
        }
    }

    let timeout_ms = effective_tool_timeout_ms(settings, tool, &call.arguments);
    Ok(ToolPreparation::Ready(PreparedToolCall {
        record,
        timeout_ms: (timeout_ms != NO_OUTER_TOOL_TIMEOUT).then_some(timeout_ms),
        timeout_message: format_tool_timeout_error(tool, timeout_ms, &call.arguments),
        arguments: call.arguments,
        output_limit: settings.chat_tools.max_tool_output_chars,
        drain_on_cancel: tool.source == "mcp" || host.requires_tool_completion(),
        feedback: hook_context,
    }))
}

fn validate_read_only_action(
    tool: &ChatToolDefinition,
    arguments: &Value,
    plan_state: &crate::chat::types::AgentPlanState,
) -> Result<(), String> {
    if tool.read_only_actions().is_empty() || tool.is_read_only_call(arguments) {
        return Ok(());
    }
    let mode = if plan_state.mode == crate::chat::types::AgentPlanMode::Plan {
        Some("plan")
    } else {
        tool.annotations
            .as_ref()
            .and_then(|annotations| annotations.get("kivioReadOnlyMode"))
            .and_then(Value::as_str)
    };
    if let Some(mode) = mode {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("(missing action)");
        return Err(format!(
            "{action} is not allowed in {mode} mode; only read-only actions {} are allowed",
            tool.read_only_actions().join(", ")
        ));
    }
    Ok(())
}

fn action_rejection(mut record: AgentToolCall, error: String) -> ToolPreparation {
    record.status = AgentToolStatus::Skipped;
    record.completed_at = Some(chrono::Local::now().timestamp());
    record.error = Some(error.clone());
    ToolPreparation::Complete(record, error, Vec::new())
}

fn tool_call_requires_approval(
    settings: &Settings,
    tool: &ChatToolDefinition,
    arguments: &Value,
) -> bool {
    if !tool.read_only_actions().is_empty() && tool.is_read_only_call(arguments) {
        // schedule list historically bypassed approval; automation reads still honor always_confirm.
        return tool.name != "schedule" && settings.chat_tools.approval_policy == "always_confirm";
    }
    tool_requires_approval(settings, tool)
}

fn schema_rejection(
    mut record: AgentToolCall,
    mut error: String,
    tool: &ChatToolDefinition,
) -> ToolPreparation {
    if super::tool_on_demand::is_on_demand(tool) {
        error.push_str(&format!(". Input JSON schema: {}", tool.input_schema));
    }
    record.status = AgentToolStatus::Error;
    record.duration_ms = Some(0);
    record.completed_at = Some(chrono::Local::now().timestamp());
    record.error = Some(error.clone());
    ToolPreparation::Complete(record, format!("Tool arguments failed schema validation: {error}. Retry this tool call with arguments that match the declared JSON schema."), Vec::new())
}

async fn after_tool_call(
    host: &ApplicationToolContext<'_>,
    ctx: &ToolExecutionContext<'_>,
    tool: &ChatToolDefinition,
    arguments: Value,
    content: &str,
    mut hook_context: Vec<String>,
) -> Result<ToolResultUpdate, String> {
    if let Some(runtime) = host.workflow_hooks().filter(|r| !r.hooks.is_empty()) {
        let mut input = runtime.input("PostToolUse", ctx.conversation_id);
        input["tool_name"] = serde_json::json!(tool.name);
        input["tool_input"] = arguments;
        input["tool_response"] = serde_json::json!(content);
        input["tool_use_id"] = serde_json::json!(ctx.tool_call_id);
        match crate::chat::workflow_hooks::run_for_host(
            host.host,
            host.control.clone(),
            "PostToolUse",
            input,
        )
        .await
        {
            Ok(out) => {
                hook_context.extend(out.context);
                if let Some(reason) = out.denied {
                    hook_context.push(reason);
                }
            }
            Err(err) => return Err(err),
        }
    }
    if !hook_context.is_empty() {
        return Ok(ToolResultUpdate {
            content: Some(format!(
                "{content}\n\n[Workflow hook feedback]\n{}",
                hook_context.join("\n\n")
            )),
            follow_up_messages: None,
        });
    }
    Ok(ToolResultUpdate::default())
}

async fn execute_ask_user_call(
    host: &ApplicationToolContext<'_>,
    settings: &Settings,
    ctx: &ToolExecutionContext<'_>,
    mut record: AgentToolCall,
    arguments: Value,
) -> (AgentToolCall, String) {
    let prompt = match ask_user::normalize_prompt(arguments) {
        Ok(prompt) => prompt,
        Err(err) => {
            record.status = AgentToolStatus::Error;
            record.duration_ms = Some(0);
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some(err.clone());
            return (
                record,
                format!("Invalid ask_user prompt: {err}. Retry with a valid questions payload."),
            );
        }
    };

    record.status = AgentToolStatus::Running;
    record.structured_content = Some(ask_user::structured_content(
        &prompt,
        ask_user::ASK_USER_PHASE_AWAITING,
        &Default::default(),
    ));
    host.tool_updated(&record).await;

    let started = Instant::now();
    let response = host
        .request_user_response(ctx, &record, prompt.clone())
        .await;
    record.duration_ms = Some(started.elapsed().as_millis() as u64);
    record.completed_at = Some(chrono::Local::now().timestamp());
    record.structured_content = Some(ask_user::structured_content(
        &prompt,
        &response.phase,
        &response.answers,
    ));
    let content = ask_user::tool_result_content(&response);
    match response.phase.as_str() {
        ask_user::ASK_USER_PHASE_ANSWERED => {
            record.status = AgentToolStatus::Success;
            record.output = Some("User answered clarification questions".to_string());
        }
        ask_user::ASK_USER_PHASE_SKIPPED => {
            record.status = AgentToolStatus::Skipped;
            record.output = Some("User skipped clarification".to_string());
        }
        ask_user::ASK_USER_PHASE_TIMEOUT => {
            record.status = AgentToolStatus::Skipped;
            record.output = Some("Clarification timed out".to_string());
        }
        ask_user::ASK_USER_PHASE_CANCELLED => {
            record.status = AgentToolStatus::Cancelled;
            record.error = Some("Clarification cancelled".to_string());
        }
        _ => {
            record.status = AgentToolStatus::Error;
            record.error = Some(format!(
                "Invalid ask_user response phase: {}",
                response.phase
            ));
        }
    }
    (
        record,
        limit_tool_text_for_model(&content, settings.chat_tools.max_tool_output_chars),
    )
}

pub fn tool_requires_approval(settings: &Settings, tool: &ChatToolDefinition) -> bool {
    tool_requires_approval_for_policy(&settings.chat_tools.approval_policy, tool)
}

pub(crate) fn tool_requires_approval_for_policy(policy: &str, tool: &ChatToolDefinition) -> bool {
    if builtin_tool_bypasses_approval(tool) {
        return false;
    }
    match policy {
        "auto" => false,
        "always_confirm" => true,
        // MCP annotations are self-reported by the server and cannot weaken approval.
        // The explicit user-side `auto` policy above remains the trusted opt-out.
        _ if tool.source == "mcp" => true,
        _ => tool.sensitive,
    }
}

pub fn validate_tool_arguments(tool: &ChatToolDefinition, arguments: &Value) -> Result<(), String> {
    crate::chat::tools::schema::validate_arguments(&tool.input_schema, arguments)
}

/// Sentinel for tools that enforce their own wait or timeout.
const NO_OUTER_TOOL_TIMEOUT: u64 = u64::MAX;

fn effective_tool_timeout_ms(
    settings: &Settings,
    tool: &ChatToolDefinition,
    arguments: &Value,
) -> u64 {
    tool.timeout_policy()
        .milliseconds(settings.chat_tools.tool_timeout_ms, arguments)
        .unwrap_or(NO_OUTER_TOOL_TIMEOUT)
}

/// 超时错误给模型用：含工具名、时限、简短参数摘要与可执行建议（避免只回一句「超时」）。
fn format_tool_timeout_error(
    tool: &ChatToolDefinition,
    timeout_ms: u64,
    arguments: &Value,
) -> String {
    let secs = timeout_ms as f64 / 1000.0;
    let arg_summary = timeout_argument_summary(arguments);
    let mut msg = format!(
        "工具调用超时（已中止，限时 {secs:.0}s）。工具：`{}`（source={}）。",
        tool.name, tool.source
    );
    if !arg_summary.is_empty() {
        msg.push_str(&format!(" 参数摘要：{arg_summary}"));
    }
    if tool.source == "native" && tool.name == "bash" {
        msg.push_str(
            " 请勿原样重试同一条调用。先缩小命令范围；确需更长时间时设置更大的 timeout_ms，\
明确不设期限时使用 timeout_ms:0。不要把有限命令改成 background:true；只有永不退出的 server 才进后台。",
        );
    } else {
        msg.push_str(
            " 请勿原样重试同一条调用。建议：缩小单次任务（拆成更小的步骤）；\
核对路径与语法后换一版参数再试。需要更长等待可在设置中提高「工具超时」。",
        );
    }
    msg
}

fn timeout_argument_summary(arguments: &Value) -> String {
    // 优先 command 字段（officecli MCP / bash）
    let raw = arguments
        .get("command")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .or_else(|| {
            arguments
                .get("cmd")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            let s = arguments.to_string();
            if s.len() > 240 {
                format!("{}…", s.chars().take(240).collect::<String>())
            } else {
                s
            }
        });
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 220 {
        format!("{}…", compact.chars().take(220).collect::<String>())
    } else {
        compact
    }
}

pub struct RunToolSnapshot {
    pub settings: Settings,
    pub provider: crate::settings::ModelProvider,
    pub model: String,
    pub depth: u8,
    pub tool_conversation_id: String,
    pub plan_state: crate::chat::types::AgentPlanState,
    pub mcp_tool_ids: Vec<(String, String)>,
    pub mcp_server_ids: Vec<String>,
    pub approval_policy: String,
    pub interactive_host: bool,
    pub tools: Vec<ChatToolDefinition>,
}
impl RunToolSnapshot {
    #[cfg(test)]
    pub fn from_context(
        settings: &Settings,
        ctx: &ToolExecutionContext<'_>,
        tools: &[ChatToolDefinition],
    ) -> Self {
        Self {
            settings: settings.clone(),
            provider: ctx.provider.clone(),
            model: ctx.model.into(),
            depth: ctx.depth,
            tool_conversation_id: ctx.tool_conversation_id.into(),
            plan_state: ctx.plan_state.clone(),
            mcp_tool_ids: ctx.mcp_tool_ids.to_vec(),
            mcp_server_ids: ctx.mcp_server_ids.to_vec(),
            approval_policy: ctx.approval_policy.into(),
            interactive_host: ctx.interactive_host,
            tools: tools.to_vec(),
        }
    }
    fn context<'a>(
        &'a self,
        ctx: &'a crate::chat::agent::execute::ToolExecutionContext,
    ) -> ToolExecutionContext<'a> {
        ToolExecutionContext {
            run_tools: &self.tools,
            conversation_id: &ctx.conversation_id,
            run_id: &ctx.run_id,
            message_id: &ctx.message_id,
            generation: ctx.generation,
            round: ctx.round,
            tool_call_id: &ctx.tool_call_id,
            provider: &self.provider,
            model: &self.model,
            depth: self.depth,
            tool_conversation_id: &self.tool_conversation_id,
            plan_state: &self.plan_state,
            mcp_tool_ids: &self.mcp_tool_ids,
            mcp_server_ids: &self.mcp_server_ids,
            approval_policy: &self.approval_policy,
            interactive_host: self.interactive_host,
        }
    }
}
pub struct ApplicationToolExecutor {
    pub snapshot: Arc<RunToolSnapshot>,
    pub host: Arc<dyn ApplicationHost>,
    pub dispatcher: Arc<dyn ToolDispatcher>,
}

impl ApplicationToolExecutor {
    fn definition(&self, tool: &crate::chat::tools::ToolDefinition) -> &ChatToolDefinition {
        self.snapshot
            .tools
            .iter()
            .find(|candidate| candidate.id == tool.key)
            .expect("registered tool has application definition")
    }
}

impl crate::chat::agent::execute::ToolExecutor for ApplicationToolExecutor {
    fn resolve_tool_call(
        &self,
        tool: &crate::chat::tools::ToolDefinition,
        mut call: PendingToolCall,
    ) -> Result<(crate::chat::tools::ToolDefinition, PendingToolCall), String> {
        if tool.label.source == "native" && tool.label.name == "write" {
            if let Some(name) = call
                .arguments
                .get("path")
                .and_then(Value::as_str)
                .and_then(|path| path.strip_prefix("tool://"))
            {
                let target = super::tool_on_demand::find(&self.snapshot.tools, name)
                    .ok_or_else(|| super::tool_on_demand::unavailable(name))?;
                let arguments = call
                    .arguments
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "tool:// content must be JSON text".to_string())
                    .and_then(|content| {
                        serde_json::from_str(content)
                            .map_err(|error| format!("Invalid JSON arguments: {error}"))
                    });
                call.function_name = name.to_string();
                call.arguments_parse_error = arguments.as_ref().err().cloned();
                call.arguments = arguments.unwrap_or(Value::Null);
                call.arguments_raw = call.arguments.to_string();
                return Ok((target.for_run(&self.snapshot.settings), call));
            }
        }
        Ok((tool.clone(), call))
    }
    fn before_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress,
        ctx: &'a crate::chat::agent::execute::ToolExecutionContext,
        tool: &'a crate::chat::tools::ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
        Box::pin(async move {
            let host = ApplicationToolContext {
                host: self.host.as_ref(),
                control: progress.control.clone(),
                progress,
            };
            before_tool_call(
                &host,
                &self.snapshot.settings,
                &self.snapshot.context(ctx),
                self.definition(tool),
                call,
                record,
            )
            .await
        })
    }

    fn call(
        self: Arc<Self>,
        invocation: crate::chat::agent::execute::ToolInvocation,
    ) -> ToolExecutorFuture<'static> {
        Box::pin(async move {
            self.dispatcher
                .call(
                    &invocation.progress,
                    &self.snapshot.context(&invocation.context),
                    self.definition(&invocation.tool),
                    invocation.arguments,
                )
                .await
        })
    }

    fn after_tool_call<'a>(
        &'a self,
        progress: &'a ToolProgress,
        ctx: &'a crate::chat::agent::execute::ToolExecutionContext,
        tool: &'a crate::chat::tools::ToolDefinition,
        arguments: Value,
        content: &'a str,
        feedback: Vec<String>,
    ) -> ToolFuture<'a, Result<ToolResultUpdate, String>> {
        Box::pin(async move {
            let host = ApplicationToolContext {
                host: self.host.as_ref(),
                control: progress.control.clone(),
                progress,
            };
            after_tool_call(
                &host,
                &self.snapshot.context(ctx),
                self.definition(tool),
                arguments,
                content,
                feedback,
            )
            .await
        })
    }

    fn start_speculation(
        &self,
        ctx: &crate::chat::agent::execute::ToolExecutionContext,
        tools: &[crate::chat::tools::ToolDefinition],
    ) -> Option<Box<dyn SpeculativeTaskScope>> {
        let definitions: Vec<_> = tools
            .iter()
            .map(|tool| self.definition(tool).clone())
            .collect();
        self.host.start_speculation(
            &self.snapshot.settings,
            &definitions,
            &self.snapshot.context(ctx),
        )
    }

    fn blocked_feedback(&self, tool: &crate::chat::tools::ToolDefinition) -> String {
        format!("Tool `{}` is blocked in Plan mode. Switch to Act / execute the plan before using side-effecting tools.", tool.model.name)
    }
    fn unknown_feedback(&self, name: &str) -> Option<String> {
        disabled_builtin_tool_feedback(name)
    }
}

#[cfg(test)]
include!("tool_runtime_tests.rs");
