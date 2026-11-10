use crate::chat::agent::content::{AgentToolCall, ContentBlock};
use crate::chat::agent::provider_runtime::ModelRuntime;
use crate::mcp::ChatToolDefinition;
use crate::settings::{ChatToolsConfig, ModelProvider, Settings};
use serde_json::Value;

#[derive(Clone)]
pub struct ApplicationRunConfig<'a> {
    pub(crate) state: &'a crate::state::AppState,
    pub(crate) provider_runtime: Option<&'a dyn ModelRuntime>,
    pub todo_state: crate::chat::types::AgentTodoState,
    pub conversation_id: String,
    /// Conversation that conversation-scoped tools (todo / native workspace)
    /// target. Equals `conversation_id` for a normal chat run; for a sub-agent
    /// run it is the parent conversation (see `ToolExecutionContext`).
    pub tool_conversation_id: String,
    /// Sub-agent nesting depth (0 = top-level chat run).
    pub depth: u8,
    /// True only when this run is hosted by the interactive chat UI.
    pub interactive_host: bool,
    pub run_id: String,
    pub message_id: String,
    pub generation: u64,
    pub provider: ModelProvider,
    pub model: String,
    pub history: kivio_agent_sdk::agent::SessionHistory,
    pub runtime_context: Option<String>,
    pub(crate) request_projection: Option<crate::chat::context_policy::MainRequestProjection>,
    pub tools: Vec<ChatToolDefinition>,
    pub blocked_tool_calls: Vec<ChatToolDefinition>,
    /// Delegation inherits the mode and the exact MCP authority exposed by this run.
    pub plan_state: crate::chat::types::AgentPlanState,
    /// Exact (server ID, tool ID) pairs; empty means no MCP tools.
    pub mcp_tool_ids: Vec<(String, String)>,
    pub mcp_server_ids: Vec<String>,
    pub settings: Settings,
    pub effective_chat_tools: ChatToolsConfig,
    pub language: String,
    pub thinking_enabled: bool,
    /// 每对话「思考等级」(`Some("low"|"medium"|"high")`)。`None` = 未设置，维持现状。
    /// 仅作用于答案生成（planning/synthesis），不作用于压缩摘要。
    pub thinking_level: Option<String>,
    /// 会话级联网搜索有效模式（任务 07-23）。由 `effective_web_search_mode` 解析后传入。
    /// 决定本 run 是否暴露 `search_web`（ThirdParty）或请求内置搜索（Builtin，且模型支持）。
    pub web_search_mode: crate::chat::types::WebSearchMode,
    pub max_output_tokens: u32,
    pub retry_attempts: usize,
    pub provider_tools_fallback_prompt: crate::chat::agent_preparation::PreparedChatPrompt,
    /// Completed selected-branch main requests: (normalized input, cache-read tokens).
    pub initial_cache_usage: Option<(u64, u64)>,
    /// 真实用量锚点（来自会话最后一条带 `anchor_usage` 的 assistant 消息，由 commands.rs 解析）：
    /// run 首次压缩检查前用它把上下文占用锚定到 provider 实报值。值为「上次 prompt + 该次响应」的
    /// token **总数**（含 output，见 `crate::chat::model::context_usage::anchor_total_tokens`）。`None` = 无可用锚点
    /// （新对话 / 旧数据无 usage / 切换过 provider / 压缩边界之后），回落纯字符估算。
    pub initial_anchor_total_tokens: Option<u64>,
    /// 锚点响应**之后**（不含响应本身，响应用 output 计入锚点）新增消息的字符估算，由 commands.rs
    /// 组装 runtime_messages 时算好。与 `initial_anchor_total_tokens` 配对：`effective = 锚点 + 该 trailing`。
    pub initial_anchor_trailing_estimate: usize,
}

impl ApplicationRunConfig<'_> {
    /// 本 run 是否应请求模型**原生内置联网搜索**（任务 07-23）：
    /// 会话为 `Builtin` 模式且当前 provider 支持（按 `api_format`）。
    /// 用于答案生成（planning/synthesis）设置 `GenerateOptions.builtin_web_search`。
    pub(crate) fn builtin_web_search_active(&self) -> bool {
        self.web_search_mode == crate::chat::types::WebSearchMode::Builtin
            && crate::chat::model_metadata::builtin_web_search_supported(&self.provider)
    }
}

#[derive(Debug, Clone)]
pub struct ApplicationRunResult {
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_records: Vec<AgentToolCall>,
    pub segments: Vec<ContentBlock>,
    pub api_messages: Vec<Value>,
    pub status: crate::chat::agent::types::AgentRunStatus,
    /// 本轮全部模型调用（规划/合成/压缩摘要）累计的 provider 真实 usage；
    /// provider 不报告时为 None（前端回落到 chars 估算）。
    pub usage: Option<crate::chat::model::ModelUsage>,
    /// Sum of complete per-request cache telemetry pairs; independent of billing usage.
    pub cache_pairs: Option<(u64, u64)>,
    /// 本轮**最后一次**模型调用的 usage（真实用量锚点，落盘到 `ChatMessage.anchor_usage`）。
    /// 与累计 `usage` 区分——累计是多步之和会虚高，锚点须是单次调用值。None = provider 未报。
    pub last_step_usage: Option<crate::chat::model::ModelUsage>,
    pub history: kivio_agent_sdk::agent::SessionHistory,
    pub compacted: bool,
    /// Agent-loop L2 compaction boundary for timeline UI persistence.
    pub compaction_boundary: Option<crate::chat::types::CompactionBoundaryRecord>,
    /// Agent-loop L2 compaction summary for `context_state.summary` persistence
    /// (L2 不再只 push boundary，run 结束时由 commands.rs 写回 summary + compression_count）。
    pub compaction_summary: Option<crate::chat::types::ConversationContextSummary>,
    /// 本轮降级兜底的结构化描述（模型失败但已产出工具结果）。None = 正常回答。
    /// 落到 `ChatMessage.degraded`，前端据此渲染错误卡片。
    pub degraded: Option<crate::chat::recovery_policy::DegradedAnswer>,
    /// 模型原生生成的图片（Gemini native image gen，任务 07-24）：跨轮累积的
    /// `GenerateOutput.images`，reply 侧落成 assistant 消息级 artifacts。空 = 未出图。
    pub images: Vec<crate::chat::model::GeneratedImageData>,
}

#[derive(Debug)]
pub enum ApplicationRunError {
    Admission(kivio_agent_sdk::agent::AgentAdmissionError),
    Failed {
        cause: kivio_agent_sdk::agent::AgentFailure,
        partial: Box<ApplicationRunResult>,
    },
}

impl std::fmt::Display for ApplicationRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Admission(error) => {
                kivio_agent_sdk::agent::AgentRunError::Admission(error.clone()).fmt(f)
            }
            Self::Failed { cause, .. } => cause.fmt(f),
        }
    }
}

impl std::error::Error for ApplicationRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Failed { cause, .. } => Some(cause),
            Self::Admission(_) => None,
        }
    }
}

use crate::chat::agent::{
    control::AgentRunHandle,
    history::RuntimeMessage,
    recovery::{FallbackReason, RecoveryDecision, RecoveryFailure, RecoverySnapshot},
    session::{
        ContextPreparation, ContextReason, ContextSnapshot, PolicyFuture, PolicyProgress,
        PromptUpdate, RunSnapshot, SessionPolicy, ToolsRejected,
    },
};

pub(crate) struct AppSessionPolicy<'a> {
    host: &'a dyn crate::chat::tool_runtime::ApplicationHost,
    conversation_id: String,
    subagent: bool,
    runtime_context: Option<String>,
    fallback_prompt: crate::chat::agent_preparation::PreparedChatPrompt,
    tried_skill_only_tools: bool,
    pub(crate) context: crate::chat::context_policy::ContextPolicy<'a>,
    pub(crate) recovery: crate::chat::recovery_policy::RecoveryPolicy,
}

impl<'a> AppSessionPolicy<'a> {
    pub(crate) fn new(
        config: &ApplicationRunConfig<'a>,
        host: &'a dyn crate::chat::tool_runtime::ApplicationHost,
    ) -> Self {
        Self {
            host,
            conversation_id: config.conversation_id.clone(),
            subagent: config.depth > 0,
            runtime_context: config.runtime_context.clone(),
            fallback_prompt: config.provider_tools_fallback_prompt.clone(),
            tried_skill_only_tools: false,
            context: crate::chat::context_policy::ContextPolicy::new(config.state, config, host),
            recovery: crate::chat::recovery_policy::RecoveryPolicy::new(
                &config.provider,
                &config.model,
                &config.language,
            ),
        }
    }

    async fn prompt_context(
        &self,
        prompt: &str,
        control: AgentRunHandle,
    ) -> Result<Vec<String>, String> {
        let Some(runtime) = self
            .host
            .workflow_hooks()
            .filter(|runtime| !runtime.hooks.is_empty())
        else {
            return Ok(Vec::new());
        };
        let mut input = runtime.input("UserPromptSubmit", &self.conversation_id);
        input["prompt"] = serde_json::json!(prompt);
        let outcome = crate::chat::workflow_hooks::run_for_host(
            self.host,
            control,
            "UserPromptSubmit",
            input,
        )
        .await?;
        if let Some(reason) = outcome.denied {
            return Err(reason);
        }
        crate::chat::workflow_hooks::remember_context(
            runtime,
            &self.conversation_id,
            &outcome.context,
        )
        .await;
        Ok(outcome.context)
    }
}

impl SessionPolicy for AppSessionPolicy<'_> {
    fn start<'a>(
        &'a mut self,
        run: RunSnapshot<'a>,
        control: AgentRunHandle,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move {
            let mut messages = Vec::new();
            if let Some(context) = self.runtime_context.take() {
                let mut history = run.history.to_vec();
                messages.extend(crate::chat::agent_preparation::append_runtime_context(
                    &mut history,
                    &context,
                ));
            }
            let resumed = run
                .history
                .iter()
                .any(|message| message.role() == Some(crate::chat::model::ModelRole::Assistant));
            let mut context = crate::chat::workflow_hooks::start_context(
                self.host,
                control.clone(),
                &self.conversation_id,
                resumed,
                self.subagent,
            )
            .await?;
            if !self.subagent {
                if let Some(runtime) = self.host.workflow_hooks() {
                    context.extend(
                        self.prompt_context(runtime.prompt.as_deref().unwrap_or(""), control)
                            .await?,
                    );
                }
            }
            crate::chat::workflow_hooks::inject_context(&mut messages, &context);
            Ok(messages)
        })
    }

    fn input<'a>(
        &'a mut self,
        prompt: &'a str,
        control: AgentRunHandle,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move {
            let context = self.prompt_context(prompt, control).await?;
            let mut messages = Vec::new();
            crate::chat::workflow_hooks::inject_context(&mut messages, &context);
            Ok(messages)
        })
    }

    fn tools_rejected(&mut self, run: RunSnapshot<'_>) -> ToolsRejected {
        let skill_only: Vec<_> = run
            .tools
            .iter()
            .filter(|tool| tool.label.source == "skill")
            .cloned()
            .collect();
        if !self.tried_skill_only_tools
            && !skill_only.is_empty()
            && skill_only.len() < run.tools.len()
        {
            self.tried_skill_only_tools = true;
            return ToolsRejected::Retry(skill_only);
        }
        let context = if self.fallback_prompt.context.is_empty() {
            None
        } else {
            crate::chat::agent_preparation::append_runtime_context(
                &mut run.history.to_vec(),
                &self.fallback_prompt.context,
            )
        };
        ToolsRejected::Plain(Some(PromptUpdate {
            system: self.fallback_prompt.system.clone(),
            context,
        }))
    }

    fn recover(
        &mut self,
        run: RecoverySnapshot<'_>,
        error: &crate::chat::model::ModelError,
    ) -> RecoveryDecision {
        self.recovery.recover(run, error)
    }

    fn recovery_failed(
        &mut self,
        run: RecoverySnapshot<'_>,
        failure: RecoveryFailure<'_>,
    ) -> String {
        self.recovery.recovery_failed(run, failure)
    }

    fn context_blocked(&mut self, run: RecoverySnapshot<'_>) -> String {
        self.recovery.context_blocked(run)
    }

    fn fallback(&self, reason: FallbackReason) -> String {
        self.recovery.fallback(reason)
    }

    fn prepare_context<'a>(
        &'a mut self,
        reason: ContextReason,
        run: ContextSnapshot<'a>,
        control: AgentRunHandle,
        progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, Result<ContextPreparation, String>> {
        Box::pin(self.context.prepare_context(reason, run, control, progress))
    }

    fn context_applied<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        control: AgentRunHandle,
        progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, (Vec<RuntimeMessage>, bool)> {
        Box::pin(async move {
            self.context.context_applied(history, progress);
            let context = crate::chat::workflow_hooks::compact_context(
                self.host,
                control.clone(),
                &self.conversation_id,
            )
            .await
            .unwrap_or_else(|error| vec![format!("SessionStart compact hook failed: {error}")]);
            let mut messages = Vec::new();
            crate::chat::workflow_hooks::inject_context(&mut messages, &context);
            let blocked = control.active() && self.context.request_blocked(history, &messages);
            (messages, blocked)
        })
    }

    fn context_rejected(&mut self, phase: &str, progress: &dyn PolicyProgress) {
        self.context.context_rejected(phase, progress);
    }

    fn transform_context<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        _control: AgentRunHandle,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move { self.context.transform_context(history) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::agent::loop_::tests::{test_app_state, test_run_config, TestHost};
    use crate::chat::tools::ToolDefinition;

    #[test]
    fn tool_rejection_retries_skills_once_then_prepares_deduplicated_plain_context() {
        let state = test_app_state();
        let host = TestHost::default();
        let mut config = test_run_config(&state, "http://unused");
        config.provider_tools_fallback_prompt =
            crate::chat::agent_preparation::PreparedChatPrompt {
                system: "plain system".into(),
                context: "fallback skill instructions".into(),
            };
        let tools = ["native", "skill"].map(|source| {
            let mut tool = ToolDefinition::new(crate::chat::model::ModelTool {
                name: source.into(),
                description: String::new(),
                input_schema: serde_json::json!({"type": "object"}),
            });
            tool.label.source = source.into();
            tool
        });
        let mut policy = AppSessionPolicy::new(&config, &host);
        let mut history = Vec::new();
        let run = RunSnapshot {
            history: &history,
            tools: &tools,
            tool_records: &[],
        };
        let ToolsRejected::Retry(retry) = policy.tools_rejected(run) else {
            panic!("mixed tools should retry the skill subset");
        };
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].label.source, "skill");
        let ToolsRejected::Plain(Some(prompt)) = policy.tools_rejected(run) else {
            panic!("a second rejection should use the plain prompt");
        };
        assert_eq!(prompt.system, "plain system");
        let context = prompt.context.expect("prepared runtime context");
        assert!(context
            .text_content()
            .starts_with("<kivio_runtime_context>\n"));
        assert!(context
            .text_content()
            .contains("fallback skill instructions"));
        history.push(context);
        let ToolsRejected::Plain(Some(prompt)) = policy.tools_rejected(RunSnapshot {
            history: &history,
            tools: &retry,
            tool_records: &[],
        }) else {
            panic!("the skill-only retry must also fall back after rejection");
        };
        assert!(prompt.context.is_none());
    }
}

pub(crate) async fn run_session(
    mut config: Box<ApplicationRunConfig<'_>>,
    host: std::sync::Arc<dyn crate::chat::tool_runtime::ApplicationHost>,
    dispatcher: std::sync::Arc<dyn crate::chat::tool_runtime::ToolDispatcher>,
    sink: &dyn crate::chat::agent::events::AgentEventSink,
    control: crate::chat::agent::control::AgentRunHandle,
) -> Result<ApplicationRunResult, ApplicationRunError> {
    use crate::chat::agent::types::AgentRunConfig;
    use crate::chat::tool_runtime::{ApplicationToolExecutor, RunToolSnapshot};
    config
        .settings
        .chat_tools
        .approval_policy
        .clone_from(&config.effective_chat_tools.approval_policy);
    let model = crate::chat::model::agent_runtime::AppAgentModel::new(
        config.state,
        &config.provider,
        config.retry_attempts,
    );
    let mut injections = crate::chat::runtime_injections::sources(&config, host.clone());
    let core = AgentRunConfig {
        provider_runtime: config.provider_runtime.unwrap_or(&model),
        conversation_id: config.conversation_id.clone(),
        run_id: config.run_id.clone(),
        message_id: config.message_id.clone(),
        generation: config.generation,
        provider_id: config.provider.id.clone(),
        provider_label: config.provider.name.clone(),
        api_format: config.provider.api_format.clone(),
        model: config.model.clone(),
        history: std::mem::take(&mut config.history),
        tools: config
            .tools
            .iter()
            .map(|tool| tool.for_run(&config.settings))
            .collect(),
        blocked_tool_calls: config
            .blocked_tool_calls
            .iter()
            .map(|tool| tool.for_run(&config.settings))
            .collect(),
        options: crate::chat::model::GenerateOptions {
            thinking_enabled: config.thinking_enabled,
            thinking_level: config.thinking_level.clone(),
            builtin_web_search: config.builtin_web_search_active(),
            max_tokens: config.max_output_tokens,
            ..Default::default()
        },
        capabilities: crate::chat::model_metadata::agent_model_capabilities(
            &config.provider,
            &config.model,
        ),
        initial_cache_usage: config.initial_cache_usage,
        report_context: config.depth == 0,
        limits: Default::default(),
    };
    let snapshot = std::sync::Arc::new(RunToolSnapshot {
        settings: config.settings.clone(),
        provider: config.provider.clone(),
        model: config.model.clone(),
        depth: config.depth,
        tool_conversation_id: config.tool_conversation_id.clone(),
        plan_state: config.plan_state.clone(),
        mcp_tool_ids: config.mcp_tool_ids.clone(),
        mcp_server_ids: config.mcp_server_ids.clone(),
        approval_policy: config.effective_chat_tools.approval_policy.clone(),
        interactive_host: config.interactive_host,
        tools: config.tools.clone(),
    });
    let executor = std::sync::Arc::new(ApplicationToolExecutor {
        snapshot,
        host: host.clone(),
        dispatcher,
    });
    let mut policy = AppSessionPolicy::new(&config, host.as_ref());
    let result = match crate::chat::agent::run_agent_loop(
        core,
        host.as_ref(),
        executor,
        &mut policy,
        &mut injections,
        sink,
        control,
    )
    .await
    {
        Ok(result) => result,
        Err(kivio_agent_sdk::agent::AgentRunError::Failed { cause, partial }) => {
            if !partial.generated_history.is_empty()
                || !partial.tool_records.is_empty()
                || !partial.segments.is_empty()
            {
                host.persist_partial_assistant(
                    &config.conversation_id,
                    &config.message_id,
                    &partial.tool_records,
                    &partial.segments,
                    &partial.generated_history,
                    &partial.history,
                )
                .await;
            }
            return Err(ApplicationRunError::Failed {
                cause,
                partial: Box::new(application_result(*partial, &mut policy)),
            });
        }
        Err(kivio_agent_sdk::agent::AgentRunError::Admission(error)) => {
            return Err(ApplicationRunError::Admission(error));
        }
    };
    Ok(application_result(result, &mut policy))
}

fn application_result(
    result: crate::chat::agent::types::AgentRunResult,
    policy: &mut AppSessionPolicy<'_>,
) -> ApplicationRunResult {
    use crate::chat::agent::history::export_history;
    let (compaction_boundary, compaction_summary) =
        policy.context.finish_run(result.history.context());
    let degraded = policy
        .recovery
        .take_degraded()
        .filter(|_| result.status == crate::chat::agent::types::AgentRunStatus::Recovered);
    ApplicationRunResult {
        content: result.content,
        reasoning: result.reasoning,
        tool_records: result.tool_records,
        segments: result.segments,
        api_messages: export_history(&result.generated_history),
        status: result.status,
        usage: result.usage,
        cache_pairs: result.cache_pairs,
        last_step_usage: result.last_step_usage,
        history: result.history,
        compacted: result.compacted,
        compaction_boundary,
        compaction_summary,
        degraded,
        images: result.images,
    }
}
