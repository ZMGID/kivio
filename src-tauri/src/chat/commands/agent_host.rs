use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::chat::agent::history::RuntimeMessage;
#[cfg(test)]
use crate::chat::agent::host::InputPersistence;
use crate::chat::CompactionBoundaryRecord;
use crate::mcp::{self, ChatToolDefinition};
use crate::skills;
use crate::state::AppState;

use super::context::{emit_chat_compaction_state, emit_chat_context_usage_live};
use super::interaction::{request_session_consent, request_tool_approval, request_user_response};
use super::messages::persist_partial_assistant_snapshot;

#[cfg(test)]
use crate::chat::sub_agent::session_input;

fn checkpoint_facts(history: &kivio_agent_sdk::agent::SessionHistory) -> Vec<RuntimeMessage> {
    history
        .active_path()
        .into_iter()
        .filter_map(|entry| match &entry.kind {
            kivio_agent_sdk::agent::SessionEntryKind::Message(message) => Some(message.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;

    #[test]
    fn compacted_report_keeps_its_dedup_identity_and_ui_owner() {
        use kivio_agent_sdk::agent::{session::CompactionPlan, SessionHistory};
        let report = RuntimeMessage::from_legacy(serde_json::json!({
            "role":"developer", "content":"child result", "_ui_message_id":"subagent-result-job",
            "_subagent_result_id":"subagent-result-job", "subagent_parent_persisted":true
        }))
        .unwrap();
        let mut history =
            SessionHistory::from(vec![RuntimeMessage::system("system"), report.clone()]);
        history
            .compact(CompactionPlan {
                covered: 1..2,
                replacement: vec![RuntimeMessage::system("summary")],
            })
            .unwrap();
        assert!(!history.context().contains(&report));
        assert!(checkpoint_facts(&history).contains(&report));
        let input = session_input(report);
        assert_eq!(input.persistence, InputPersistence::AlreadyPersisted);
        assert_eq!(
            input.message.source["_ui_message_id"],
            "subagent-result-job"
        );
    }
}

pub(super) struct ChatAgentHost {
    pub(super) app: AppHandle,
    pub(super) run_id: String,
    pub(super) source_anchor: Option<kivio_agent_sdk::agent::EntryId>,
    pub(super) source_branch_id: String,
    /// 多模型臂置 true：抑制 mid-run 部分快照落盘（协调者统一落盘）。默认 false（现状）。
    pub(super) suppress_partial_persist: bool,
    pub(super) context_owner: bool,
    pub(super) interactive_approval: bool,
    /// 用户配置的生命周期 Hooks。无启用 Hook 时为 `None`，loop 零开销。
    pub(super) hooks: Option<crate::chat::hooks::HookDispatcher>,
    pub(super) workflow_hooks: crate::chat::workflow_hooks::Runtime,
}

impl ChatAgentHost {
    fn emit_compaction_status(
        &self,
        conversation_id: &str,
        phase: &str,
        trigger: Option<&str>,
        boundary: Option<&CompactionBoundaryRecord>,
    ) {
        emit_chat_compaction_state(
            &self.app,
            conversation_id,
            &self.run_id,
            phase,
            trigger,
            boundary,
        );
    }
    fn begin_context_request(
        &self,
        conversation_id: &str,
        message_id: &str,
        provider_id: &str,
        model: &str,
        messages: &[RuntimeMessage],
        tools: &[crate::chat::model::ModelTool],
    ) {
        if !self.context_owner {
            return;
        }
        let state = self.app.state::<AppState>();
        let runtime = state.chat_runtime();
        let cache = runtime
            .context_measurement(conversation_id)
            .filter(|live| live.message_id == message_id)
            .and_then(|live| live.run_cache);
        // One walk of the request that is about to be sent. No event and no
        // disk write: categories ride the first provider report. Until then,
        // refreshes retain the last reported request's total and categories.
        let settings = state.settings_read();
        let segments = crate::chat::context_measure::measure_prepared_request(
            messages,
            tools,
            settings.get_provider(provider_id),
            model,
        );
        drop(settings);
        runtime.bind_prepared_context(
            conversation_id,
            &self.run_id,
            message_id,
            provider_id,
            model,
            &segments,
            cache,
        );
    }
    fn note_context_cache(&self, conversation_id: &str, run_cache: Option<(u64, u64)>) {
        if !self.context_owner {
            return;
        }
        self.app
            .state::<AppState>()
            .chat_runtime()
            .note_context_run_cache(conversation_id, &self.run_id, run_cache);
    }
    fn finish_unreported_context(&self, conversation_id: &str) {
        if !self.context_owner {
            return;
        }
        let Some(live) = self
            .app
            .state::<AppState>()
            .chat_runtime()
            .finish_unreported_context(conversation_id, &self.run_id)
        else {
            return;
        };
        // No valid report exists yet: publish this request's categories with an
        // unknown meter. Otherwise the runtime retains the last valid snapshot.
        emit_chat_context_usage_live(
            &self.app,
            conversation_id,
            &self.run_id,
            0,
            None,
            None,
            None,
            live.seq,
            live.lifecycle_id,
            Some(&live.segments),
        );
    }
    fn emit_context_usage_live(
        &self,
        conversation_id: &str,
        used_tokens: u64,
        token_count_source: Option<&str>,
        context_window_tokens: Option<u64>,
        cache_usage: Option<(u64, u64)>,
    ) {
        if !self.context_owner {
            return;
        }
        let state = self.app.state::<AppState>();
        let runtime = state.chat_runtime();
        let (live, segments_once) = if token_count_source.is_some() {
            let Some((live, include_segments)) =
                runtime.report_context_tokens(conversation_id, &self.run_id, used_tokens)
            else {
                return;
            };
            (live, include_segments)
        } else {
            (runtime.invalidate_context_display(conversation_id), true)
        };
        let segments = segments_once.then_some(live.segments.as_slice());
        emit_chat_context_usage_live(
            &self.app,
            conversation_id,
            &self.run_id,
            used_tokens,
            token_count_source,
            context_window_tokens,
            cache_usage,
            live.seq,
            live.lifecycle_id,
            segments,
        );
    }
}
impl crate::chat::agent::events::AgentEventSink for ChatAgentHost {
    fn on_event(
        &self,
        event: &crate::chat::agent::events::AgentEvent<'_>,
    ) -> crate::chat::agent::events::EventDelivery {
        use crate::chat::agent::events::AgentEventKind;
        let id = event.identity;
        for projected in crate::chat::agent_presentation::project_event(event) {
            crate::chat::protocol::emit_run_event(&self.app, event.identity.run_id, projected);
        }
        match &event.kind {
            AgentEventKind::Compaction {
                phase,
                trigger,
                boundary,
            } => self.emit_compaction_status(
                id.conversation_id,
                phase,
                *trigger,
                boundary
                    .as_ref()
                    .map(crate::chat::agent_presentation::project_compaction)
                    .as_ref(),
            ),
            AgentEventKind::ContextPrepared {
                provider_id,
                model,
                messages,
                tools,
            } => self.begin_context_request(
                id.conversation_id,
                id.message_id,
                provider_id,
                model,
                messages,
                tools,
            ),
            AgentEventKind::ContextCache { cache } => {
                self.note_context_cache(id.conversation_id, *cache)
            }
            AgentEventKind::ContextUnreported => self.finish_unreported_context(id.conversation_id),
            AgentEventKind::ContextUsage {
                used_tokens,
                source,
                window,
                cache,
            } => self.emit_context_usage_live(
                id.conversation_id,
                *used_tokens,
                *source,
                *window,
                *cache,
            ),
            _ => {}
        }
        crate::chat::agent::events::EventDelivery::Accepted
    }
}
impl crate::chat::agent::AgentHost for ChatAgentHost {
    fn run_ended(&self, conversation_id: &str) {
        crate::chat::sub_agent::delivery::on_main_run_ended(&self.app, conversation_id);
    }

    fn persist_partial_assistant<'a>(
        &'a self,
        conversation_id: &'a str,
        message_id: &'a str,
        tool_records: &'a [crate::chat::agent::content::AgentToolCall],
        segments: &'a [crate::chat::agent::content::ContentBlock],
        api_messages: &'a [RuntimeMessage],
        history: &'a kivio_agent_sdk::agent::SessionHistory,
    ) -> crate::chat::agent::AgentHostFuture<'a, ()> {
        Box::pin(async move {
            if self.suppress_partial_persist {
                return;
            }
            let (tool_records, segments) = crate::chat::agent_presentation::project_snapshot(
                tool_records,
                segments,
                Some(&self.run_id),
            );
            if let Err(err) = persist_partial_assistant_snapshot(
                &self.app,
                conversation_id,
                message_id,
                &tool_records,
                &segments,
                api_messages,
                history,
                self.source_anchor,
                &self.source_branch_id,
            )
            .await
            {
                eprintln!("persist partial assistant snapshot failed: {err}");
            }
        })
    }

    fn hooks(&self) -> Option<&dyn crate::chat::agent::events::AgentEventSink> {
        self.hooks
            .as_ref()
            .map(|hooks| hooks as &dyn crate::chat::agent::events::AgentEventSink)
    }
}

impl crate::chat::tool_runtime::ApplicationHost for ChatAgentHost {
    fn collect_runtime_inputs<'a>(
        &'a self,
        context: kivio_agent_sdk::agent::InjectionContext<'a>,
        admission: &'a mut kivio_agent_sdk::agent::InjectionAdmission,
    ) -> kivio_agent_sdk::agent::InjectionFuture<'a, Result<(), kivio_agent_sdk::agent::InjectionError>> {
        Box::pin(async move {
            crate::chat::sub_agent::collect_for_main(
                &self.app, context.identity.conversation_id, context.identity.run_id,
                &checkpoint_facts(context.history), admission,
            ).await.map_err(kivio_agent_sdk::agent::InjectionError::Persistence)
        })
    }

    fn auto_compact_failures(&self, conversation_id: &str) -> u32 {
        self.app
            .state::<AppState>()
            .chat_runtime()
            .auto_compact_failures(conversation_id)
    }

    fn set_auto_compact_failures(&self, conversation_id: &str, failures: u32) {
        self.app
            .state::<AppState>()
            .chat_runtime()
            .set_auto_compact_failures(conversation_id, failures);
    }

    fn tool_execution_hooks_active(&self) -> bool {
        self.hooks.as_ref().is_some_and(|hooks| {
            hooks.handles(crate::chat::hooks::HookEvent::ToolExecutionStart)
                || hooks.handles(crate::chat::hooks::HookEvent::ToolExecutionEnd)
        })
    }

    fn supports_interactive_approval(&self) -> bool {
        self.interactive_approval
    }

    fn start_speculation(
        &self,
        settings: &crate::settings::Settings,
        tools: &[ChatToolDefinition],
        context: &crate::chat::tool_runtime::ToolExecutionContext<'_>,
    ) -> Option<Box<dyn crate::chat::agent::host::SpeculativeTaskScope>> {
        crate::chat::sub_agent::speculative::start(&self.app, self, settings, tools, context)
    }

    fn workflow_hooks(&self) -> Option<&crate::chat::workflow_hooks::Runtime> {
        Some(&self.workflow_hooks)
    }

    fn request_tool_approval<'a>(
        &'a self,
        ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
        record: &'a crate::chat::agent::content::AgentToolCall,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async move {
            let record = crate::chat::agent_presentation::project_tool(record, Some(ctx.run_id));
            request_tool_approval(
                &self.app,
                &self.app.state::<AppState>(),
                ctx.conversation_id,
                ctx.run_id,
                ctx.generation,
                &record,
                &uuid::Uuid::new_v4().to_string(),
            )
            .await
        })
    }

    fn request_session_consent<'a>(
        &'a self,
        ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async move {
            request_session_consent(
                &self.app,
                &self.app.state::<AppState>(),
                ctx.tool_conversation_id,
                ctx.run_id,
                ctx.generation,
            )
            .await
        })
    }

    fn request_user_response<'a>(
        &'a self,
        ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
        record: &'a crate::chat::agent::content::AgentToolCall,
        prompt: crate::chat::ask_user::AskUserPromptPayload,
    ) -> crate::chat::agent::AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult> {
        Box::pin(async move {
            let state = self.app.state::<AppState>();
            let _waiting = state.sub_agents.awaits.hold(
                ctx.tool_conversation_id,
                crate::chat::sub_agent::MAIN_AGENT_ID,
                vec!["@user".into()],
                self.app.state::<AppState>().sub_agents.irc.mark(),
                false,
            );
            let record = crate::chat::agent_presentation::project_tool(record, Some(ctx.run_id));
            request_user_response(
                &self.app,
                &self.app.state::<AppState>(),
                ctx.conversation_id,
                ctx.run_id,
                ctx.generation,
                &record,
                &uuid::Uuid::new_v4().to_string(),
                prompt,
            )
            .await
        })
    }
}

/// 无头测试通道（probe）的 AgentHost，仅 debug 构建。跑的是与 GUI 完全相同的生成核心
/// （`complete_assistant_reply_inner`），但所有需要 GUI 应答的交互门一律自动放行：审批 /
/// 会话 consent → 允许，`ask_user` → 取消态（不阻塞）。事件发射 no-op（结果从落盘的 assistant
/// 消息内联读取，不靠事件）。generation 相关沿用标准机制，保证超时/取消能生效。
#[cfg(debug_assertions)]
pub(super) struct ProbeAgentHost {
    pub(super) run_id: String,
    pub(super) app: AppHandle,
}

#[cfg(debug_assertions)]
impl crate::chat::agent::events::AgentEventSink for ProbeAgentHost {
    fn on_event(
        &self,
        event: &crate::chat::agent::events::AgentEvent<'_>,
    ) -> crate::chat::agent::events::EventDelivery {
        use crate::chat::agent::events::AgentEventKind;
        if matches!(
            event.kind,
            AgentEventKind::AttemptDiscarded { .. }
                | AgentEventKind::Status { .. }
                | AgentEventKind::Retry(_)
        ) {
            for projected in crate::chat::agent_presentation::project_event(event) {
                crate::chat::protocol::emit_run_event(&self.app, event.identity.run_id, projected);
            }
        }
        crate::chat::agent::events::EventDelivery::Accepted
    }
}
#[cfg(debug_assertions)]
impl crate::chat::agent::AgentHost for ProbeAgentHost {
    fn run_ended(&self, conversation_id: &str) {
        crate::chat::sub_agent::delivery::on_main_run_ended(&self.app, conversation_id);
    }

    /// 与 GUI host 同一份草稿落盘(append-only 草稿日志)。probe 的定位是「和 chat 窗口
    /// 完全相同的生成路径」,持久化侧路径也该一致,probe:e2e 才能覆盖草稿日志的生命周期。
    fn persist_partial_assistant<'a>(
        &'a self,
        conversation_id: &'a str,
        message_id: &'a str,
        tool_records: &'a [crate::chat::agent::content::AgentToolCall],
        segments: &'a [crate::chat::agent::content::ContentBlock],
        api_messages: &'a [RuntimeMessage],
        history: &'a kivio_agent_sdk::agent::SessionHistory,
    ) -> crate::chat::agent::AgentHostFuture<'a, ()> {
        Box::pin(async move {
            let source = match crate::chat::repository::repository(&self.app)
                .get(&self.app, conversation_id)
                .await
            {
                Ok(conversation) => conversation.builtin_history,
                Err(error) => {
                    eprintln!("probe: cannot load draft source: {error}");
                    return;
                }
            };
            let Some(source) = source else {
                return;
            };
            let (tool_records, segments) = crate::chat::agent_presentation::project_snapshot(
                tool_records,
                segments,
                Some(&self.run_id),
            );
            if let Err(err) = persist_partial_assistant_snapshot(
                &self.app,
                conversation_id,
                message_id,
                &tool_records,
                &segments,
                api_messages,
                history,
                source.snapshot.active_leaf(),
                &source.active_branch_id,
            )
            .await
            {
                eprintln!("probe: persist partial assistant snapshot failed: {err}");
            }
        })
    }
}

impl crate::chat::tool_runtime::ApplicationHost for ProbeAgentHost {
    fn collect_runtime_inputs<'a>(
        &'a self,
        context: kivio_agent_sdk::agent::InjectionContext<'a>,
        admission: &'a mut kivio_agent_sdk::agent::InjectionAdmission,
    ) -> kivio_agent_sdk::agent::InjectionFuture<'a, Result<(), kivio_agent_sdk::agent::InjectionError>> {
        Box::pin(async move {
            crate::chat::sub_agent::collect_for_main(
                &self.app, context.identity.conversation_id, context.identity.run_id,
                &checkpoint_facts(context.history), admission,
            ).await.map_err(kivio_agent_sdk::agent::InjectionError::Persistence)
        })
    }

    fn start_speculation(
        &self,
        settings: &crate::settings::Settings,
        tools: &[ChatToolDefinition],
        context: &crate::chat::tool_runtime::ToolExecutionContext<'_>,
    ) -> Option<Box<dyn crate::chat::agent::host::SpeculativeTaskScope>> {
        crate::chat::sub_agent::speculative::start(&self.app, self, settings, tools, context)
    }

    fn request_tool_approval<'a>(
        &'a self,
        _ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
        _record: &'a crate::chat::agent::content::AgentToolCall,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async { true })
    }

    fn request_session_consent<'a>(
        &'a self,
        _ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
    ) -> crate::chat::agent::AgentHostFuture<'a, bool> {
        Box::pin(async { true })
    }

    fn request_user_response<'a>(
        &'a self,
        _ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
        _record: &'a crate::chat::agent::content::AgentToolCall,
        _prompt: crate::chat::ask_user::AskUserPromptPayload,
    ) -> crate::chat::agent::AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult> {
        // 无头：不能向用户提问，直接返回取消态让 loop 继续（不阻塞）。
        Box::pin(async { crate::chat::ask_user::cancelled_response() })
    }
}

pub(super) struct RegistryToolExecutor {
    pub(super) app: AppHandle,
    pub(super) skills: skills::SkillRunContext,
    pub(super) video_analysis: tokio::sync::Mutex<crate::chat::video_analysis::VideoTool>,
}
impl crate::chat::tool_runtime::ToolDispatcher for RegistryToolExecutor {
    fn call<'a>(
        &'a self,
        progress: &'a crate::chat::agent::execute::ToolProgress,
        ctx: &'a crate::chat::tool_runtime::ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
    ) -> crate::chat::agent::ToolExecutorFuture<'a> {
        Box::pin(async move {
            let result_arguments = arguments.clone();
            let output: crate::mcp::types::McpToolCallResult = async {
                if ctx.depth == 0 {
                    let (app, id) = (self.app.clone(), ctx.conversation_id.to_string());
                    // Full-file read + deserialize: keep it off the async worker's stack and thread.
                    if let Ok(Ok(conversation)) = tokio::task::spawn_blocking(move || {
                        crate::chat::storage::load_conversation(&app, &id)
                    })
                    .await
                    {
                        if conversation.goal_state.as_ref().is_some_and(|goal| {
                            goal.status == crate::chat::types::GoalStatus::Completed
                                && goal.active_run_id.as_deref() == Some(ctx.run_id)
                        }) {
                            return Err(
                                "Goal completion was accepted; no further tools may run in this turn"
                                    .into(),
                            );
                        }
                    }
                }
                if tool.source == "mixer" && tool.name == "mixer_video_analysis" {
                    return self
                        .video_analysis
                        .lock()
                        .await
                        .call(&self.app, &self.app.state::<AppState>(), ctx, &arguments)
                        .await;
                }
                let native_ctx = mcp::registry::NativeToolContext::from_execution(ctx, progress.control.clone());
                let result = mcp::registry::call_tool(
                    &self.app,
                    &self.app.state::<AppState>(),
                    tool,
                    arguments,
                    Some(&self.skills),
                    Some(native_ctx),
                )
                .await;
                // run 域这条有 replay 价值，保留；但别为它再整读一遍会话 JSON——
                // todo_write 的结构化返回里本来就带着刚落盘的权威 todoState。
                if crate::mcp::types::canonical_tool_name(&tool.name) == "todo_write" {
                    if let Some(todo_state) = result.as_ref().ok().and_then(|res| {
                        serde_json::from_value::<crate::chat::types::AgentTodoState>(
                            res.structured_content.as_ref()?.get("todoState")?.clone(),
                        )
                        .ok()
                    }) {
                        crate::chat::protocol::emit_run_event(
                            &self.app,
                            ctx.run_id,
                            crate::chat::protocol::ChatRunEvent::TodoUpdated {
                                todo_state: (&todo_state).into(),
                            },
                        );
                    }
                }
                result
            }
            .await?;
            crate::chat::tool_output::prepare(
                &self.app,
                ctx.tool_conversation_id,
                ctx.message_id,
                tool,
                &result_arguments,
                output,
            )
            .await
        })
    }
}
